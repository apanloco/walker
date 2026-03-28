# Walker

## Development Philosophy

This project is **spec-driven**. This file (CLAUDE.md) is the absolute source of truth for how the program works. All requirements, commands, and behavior must be documented here before implementation. Implementation details that are too granular for this file may live as comments in code.

## License

MIT. Use super permissive licenses for all code and dependencies where possible.

## Overview

Walker is a cross-platform Rust command-line tool that detects Bluetooth-connected walking machines (treadmills), monitors start/stop events, and reports session data to a server.

## Architecture

### Device Profile System

Walker uses a **profile-based** architecture to support multiple treadmill brands. Each brand/protocol is implemented as a `TreadmillProfile` trait implementation.

```
src/
  main.rs          — CLI (clap) + command orchestration
  activity.rs      — ActivityTracker: infers walking/idle from step deltas
  auth.rs          — client-side auth: login flow, token storage (~/.config/walker/auth.json)
  ble.rs           — BLE adapter, scanning, profile-aware device discovery
  reporter.rs      — sends updates to server (HTTP POST, state changes + heartbeats)
  device/
    mod.rs         — TreadmillProfile trait, common types, ProfileRegistry
    urevo.rs       — UREVO profile implementation
  display.rs       — terminal output formatting
  server/
    mod.rs         — server startup, wiring
    auth.rs        — OAuth device code flow, GitHub/Google providers
    db.rs          — PostgreSQL: migrations, token/user/daily_stats persistence
    live.rs        — POST /api/update (event-driven), /ws/live WebSocket broadcast
    state.rs       — in-memory user state, MET-based calorie computation (micocalories)
    dashboard.rs   — dashboard HTML page served at /
    leaderboard.rs — GET /api/leaderboard (today/weekly/all-time)
```

**Key types:**
- `TreadmillProfile` trait — detection (`matches`), activation (`activate`), parsing (`parse_notification`)
- `TreadmillStatus` — Off, Standby, Starting, Running, Pausing, Paused
- `TreadmillData` — normalized data: speed, duration, distance, calories, steps
- `TreadmillEvent` — StatusOnly, Data, or Unknown
- `ProfileRegistry` — holds all profiles, matches discovered devices to the first matching profile
- `StepTracker` — per-session step wrap detection (profile-independent)

**Adding a new treadmill brand:** implement `TreadmillProfile` in a new file under `src/device/`, register it in `default_registry()`.

### Data Layers

Walker distinguishes between two data layers:

1. **Raw device data** (`TreadmillData`) — what the treadmill reports: speed, distance, calories, steps. The treadmill can lie (e.g., distance/calories keep ticking when you step off the belt, but steps stop).

2. **Activity state** (`ActivityState`) — what we *infer* from the raw data. The source of truth for whether the user is actually walking. Derived primarily from **step deltas**: if steps aren't increasing, the user is not walking, regardless of what the treadmill's speed/distance says.

`ActivityState` detection rules:
- **Any step increase** → immediately **WALKING**
- **No step increase for 3 seconds** → **IDLE**

`ActivityState` fields:
- `moving: bool` — is the user currently walking?
- `active_duration_secs: u64` — cumulative time spent actually walking
- `idle_duration_secs: u64` — cumulative time the treadmill was running but user was idle

The activity layer is what gets reported to the server and what the game uses to determine player movement. Raw device data is logged/stored for debugging and analytics.

```
[Treadmill] --BLE--> [TreadmillData (raw)] ---> [ActivityState (inferred)] ---> [Server/Game]
```

### System Architecture

```
Walker CLI  ──→  POST /api/update    (HTTP, stateless, authenticated)
                      ↓
                 Server (1s tick)
                   ├─ computes calories/distance
                   ├─ updates in-memory state
                   ├─ writes DB (periodic)
                   ↓
Game        ←──  /ws/live            (WebSocket, server pushes every 1s)
Dashboard   ←──  /ws/live            (WebSocket, same live stream)
Dashboard   ←──  GET /api/leaderboard (REST, polled every 10s)
```

### Client → Server Protocol

The walker CLI sends updates via **HTTP POST** (not WebSocket — too infrequent for a persistent connection).

```json
POST /api/update
Authorization: Bearer <token>

{"moving": true, "speed_mph": 2.0}
```

**Two fields only:**
- `moving` — is the user actually walking? (derived from step deltas, client-side)
- `speed_mph` — belt speed (only meaningful when `moving=true`)

**When updates are sent:**
- **Immediately** on state change (walking→idle, idle→walking, speed change)
- **Every ~1 second** as a heartbeat while walking

Server timestamps each message on arrival. No client timestamp needed.

### Server Processing (event-driven)

**On each `POST /api/update`:**
1. Authenticate token → resolve to user email
2. Compute calories/time **delta** for the period since this user's LAST update (using the old speed/state)
3. Update user's in-memory state to new values (moving, speed)
4. **Accumulate** delta into `daily_stats` DB (`+= delta`, not overwrite). At midnight `CURRENT_DATE` flips → new row created automatically.
5. Broadcast snapshot of all users to `/ws/live` viewers

No tick loop for data processing. Everything is triggered by incoming events. The server only runs a lightweight 5s timer for disconnect detection (broadcasts updated status when a user goes silent for 15s).

**MET table** (interpolated for speeds between values):
| Speed | MET |
|-------|-----|
| 2 km/h | 2.0 |
| 3 km/h | 2.5 |
| 4 km/h | 3.0 |
| 5 km/h | 3.5 |
| 6 km/h | 4.0 |

**Calorie precision:** accumulated in **micocalories** (`u64`). 1 kcal = 1,000,000 ucal. Integer math, no floating-point drift. Divide by 1,000,000 only at display time.

### Server → Viewer Protocol

**`/ws/live`** — WebSocket, server pushes on every update + every 5s disconnect check. Used by both the dashboard and games.
```json
{
  "users": [
    {"name": "daniel", "status": "walking", "speed_mph": 2.0, "distance_delta_m": 1.4, "calories_kcal": 45.2, "active_secs": 245},
    {"name": "alice", "status": "idle", "distance_delta_m": 0, "calories_kcal": 89.1, "active_secs": 600}
  ]
}
```

- `distance_delta_m` — meters moved since last update (for game movement)
- Only active users are broadcast (idle/disconnected sent once on state change, then omitted) — future optimization

User statuses:
- **walking** — heartbeat received recently, `moving=true`
- **idle** — heartbeat received recently, `moving=false`
- **disconnected** — no heartbeat for 5 seconds

**`GET /api/leaderboard`** — REST, polled by dashboard every 10 seconds:
```json
{
  "today": [{"name": "alice", "calories_kcal": 89.1}, ...],
  "weekly": [{"name": "bob", "calories_kcal": 1204.3}, ...],
  "all_time": [{"name": "bob", "calories_kcal": 8920.1}, ...]
}
```

### Dashboard

Single web page served by the walker server. Shows:

**Live area** (real-time via `/ws/live`):
- All users: name, avatar, status (walking/idle/disconnected), speed, session calories
- Current user highlighted

**Leaderboard area** (polled via `/api/leaderboard` every 10s):
- Today / Weekly / All-time top 10
- Calories as the ranking metric

Dashboard login uses standard browser OAuth redirect (same providers, same email identity).

### Profile Page ("Me" tab)

`GET /api/profile/{email}` — returns user stats for the last 30 days:
- Summary: total calories, distance, active time
- Daily bar chart (calories per day, hover for details)
- Streak counter (consecutive days with activity)
- Clickable from leaderboard names — works as a public profile for any user

### Database (PostgreSQL)

Optional — server works without it (in-memory only). When configured via `DATABASE_URL`, migrations run automatically on startup.

**users** — one row per person:
```sql
email         TEXT PRIMARY KEY
id            TEXT UNIQUE NOT NULL     (UUID, public-facing identifier)
display_name  TEXT
avatar_url    TEXT
weight_kg     REAL DEFAULT 70.0
created_at    TIMESTAMPTZ
```

**tokens** — persisted auth tokens (survive server restarts):
```sql
token         TEXT PRIMARY KEY
user_email    TEXT REFERENCES users(email)
created_at    TIMESTAMPTZ
```

**daily_stats** — one row per user per day, deltas accumulated on every incoming update:
```sql
user_email    TEXT REFERENCES users(email)
date          DATE
calories_ucal BIGINT        (micocalories, integer)
distance_m    REAL
active_secs   INT
idle_secs     INT
updated_at    TIMESTAMPTZ
PRIMARY KEY (user_email, date)
```

Leaderboard queries:
- Today: `WHERE date = CURRENT_DATE GROUP BY user → SUM(calories_ucal) → ORDER DESC → LIMIT 10`
- Weekly: `WHERE date >= CURRENT_DATE - 7 ...`
- All time: `GROUP BY user → SUM(calories_ucal) → ORDER DESC → LIMIT 10`

Local dev: `docker run -d --name walker-postgres -e POSTGRES_PASSWORD=walker -e POSTGRES_DB=walker -p 5432:5432 postgres:16-alpine`

### Why Steps Are Only Used for State Detection

Steps are the **only honest signal** from the treadmill — they stop when you stop walking, unlike distance/calories/speed which keep ticking while the belt runs empty. This makes steps perfect for answering one question: **is the user actually walking?**

But steps are deliberately NOT used for calorie calculation, distance, or game movement speed:

1. **Step length varies with speed.** 10 steps at 1 km/h covers less distance than 10 steps at 4 km/h. There is no reliable constant to convert steps → distance.

2. **Calories depend on speed, not steps.** Walking faster burns more calories per minute even at the same step rate. The MET tables used in exercise science are validated against speed, not step count. Inventing a step-based calorie formula would be unscientific guesswork.

3. **Speed is accurate when walking.** The belt speed is a motor setting — if `moving=true` (proven by steps), the user must be matching the belt. So speed is trustworthy *as long as we gate it with step-based movement detection*.

4. **Not all treadmills report steps.** Standard FTMS protocol doesn't include step count. Relying on steps for calories/distance would exclude most treadmills. Using speed (universally available) keeps the system compatible with any device.

5. **Steps don't cross the wire.** They do all their work client-side and are discarded. This keeps the protocol minimal (two fields) and avoids sending data the server can't use meaningfully.

The design: **steps detect, speed measures, server computes.**

### What the treadmill reports but we DON'T trust

- **Distance** — belt keeps ticking when idle
- **Calories** — treadmill's own estimate, unreliable
- **Speed when idle** — belt speed means nothing if nobody is walking

Steps do all their work client-side (detecting WALKING/IDLE) and never cross the wire.

## Supported Devices

### UREVO SpaceWalk E1L (URTM041)

- **BLE Name**: `URTM041`
- **Protocols**: Standard FTMS (0x1826) + proprietary UREVO protocol (0xFFF0)
- **FTMS Service (0x1826)**: Treadmill Data (0x2ACD) provides speed, distance, energy, heart rate, elapsed time, power.
- **Proprietary Service (0xFFF0)**:
  - Subscribe to notifications on `0xFFF1`, then write `[0x02, 0x51, 0x0B, 0x03]` to `0xFFF2` to activate data stream.
  - 19-byte packets at ~3 Hz:
    - Bytes 0-1: Header (always `02 51`)
    - Byte 2: Status (`00`=Standby, `02`=Starting, `03`=Running, `04`=Pausing, `06`=Off, `0A`=Paused)
    - Byte 3: Speed (0.1 mph increments)
    - Bytes 5-6: Duration in seconds (little-endian u16)
    - Bytes 7-8: Distance in 0.01 km units (little-endian u16) — **confirmed**
    - Bytes 9-10: Calories in 0.1 kcal units (little-endian u16) — **confirmed**
    - Bytes 11-12: Step count (little-endian u16)
    - Byte 17: Checksum
    - Byte 18: Terminator (always `03`)
  - Proprietary protocol gives step counts that stop when you step off the belt (FTMS doesn't report steps).
  - No BLE control of treadmill (start/stop/speed) — read-only for session data.
- **FTMS Control Point (0x2AD9)**: Accepts reset command `[0x08, 0x01]`.

### Device Detection

A device is considered a walking machine if it advertises **FTMS service UUID 0x1826** or has a name matching known treadmill patterns (e.g., `URTM*`).

## Authentication

Auth is centralized on the **walker server**. All clients (CLI, future game) authenticate against the walker server. The server uses external OAuth providers (GitHub, Google) as identity sources but issues its own tokens.

### How It Works End-to-End

```
1. walker login                     2. walker walk
   ┌─────┐    ┌────────┐              ┌─────┐    ┌────────┐
   │ CLI │───→│ Server │              │ CLI │───→│ Server │
   └─────┘    └───┬────┘              └─────┘    └────────┘
                  │                   sends: Authorization: Bearer <token>
              ┌───▼────┐              server looks up token → finds email
              │ GitHub │              → all data tagged with that email
              │/Google │
              └────────┘
```

**Login (once):**
1. `walker login` → CLI requests a device code from the server
2. Browser opens → user picks GitHub or Google → OAuth completes
3. Server extracts the user's **verified email** from the provider
4. Server generates a walker token, associates it with the email
5. CLI saves `{ token, email, display_name }` to `~/.config/walker/auth.json`

**Walk (every session):**
1. `walker walk` reads `auth.json` to get the token
2. Every data packet sent to the server includes `Authorization: Bearer <token>`
3. Server validates the token, resolves it to the user's **email** (the user ID)
4. Walking data is stored/streamed tagged with that email

**Game (future):**
1. Game authenticates against the same server (browser redirect flow instead of device code)
2. Same email = same user = sees the same walking data

### Identity

User identity is **email-based** internally. The user's verified email is the key across all providers and all clients. However, **email is never exposed to the frontend**. Each user gets a **UUID** (generated on first login) that serves as the public-facing identifier in APIs, WebSocket broadcasts, profile URLs, and cookies.

- Login with Google (`daniel@gmail.com`) → user ID = `daniel@gmail.com`
- Login with GitHub (same email) → **same user**, same data
- Switching providers "just works" as long as the email matches
- Different emails = different users (correct: no proof they're the same person)

GitHub: fetches primary verified email via `/user/emails` API (scope `user:email`).
Google: gets email from userinfo endpoint (scope `email`).

### Setting Up OAuth Providers

**GitHub** (1 minute):
1. Go to https://github.com/settings/developers → "New OAuth App"
2. Application name: anything (e.g., `Walker`)
3. Homepage URL: `http://localhost:3000` (or your production URL)
4. Authorization callback URL: `http://localhost:3000/auth/github/callback`
5. Click "Register application"
6. Copy **Client ID**, generate and copy **Client Secret**

**Google** (5 minutes):
1. Go to https://console.cloud.google.com → create a project (or select existing)
2. Navigate to "APIs & Services" → "OAuth consent screen"
3. Choose "External", fill in app name and email, click through
4. Add scopes: `openid`, `profile`, `email`
5. Navigate to "Credentials" → "Create Credentials" → "OAuth client ID"
6. Application type: "Web application"
7. Authorized redirect URIs: `http://localhost:3000/auth/google/callback`
8. Copy **Client ID** and **Client Secret**

Both are optional. The login page only shows buttons for configured providers.

### Token Storage

Credentials stored locally in `~/.config/walker/`:
- `auth.json` — production (default, `https://walker.akerud.se`)
- `auth_dev.json` — local dev (`http://localhost:3000`)

```json
{
  "server": "https://walker.akerud.se",
  "token": "...",
  "email": "daniel@gmail.com",
  "display_name": "daniel"
}
```

The `--dev` flag on `login`, `logout`, `walk`, and `simulate` switches between the two files.

### Supported Providers

- **GitHub** — `WALKER_GITHUB_CLIENT_ID` + `WALKER_GITHUB_CLIENT_SECRET`
- **Google** — `WALKER_GOOGLE_CLIENT_ID` + `WALKER_GOOGLE_CLIENT_SECRET`

Providers are optional. The verify page only shows buttons for configured providers. Both can be enabled simultaneously.

### Server Endpoints (auth)

- `POST /auth/device` — generate device code for CLI login
- `GET /auth/device/verify` — web page where user picks OAuth provider
- `POST /auth/device/token` — CLI polls this to get token after user authorizes
- `GET /auth/github` + `GET /auth/github/callback` — GitHub OAuth flow
- `GET /auth/google` + `GET /auth/google/callback` — Google OAuth flow

### Device Code Flow (CLI)

1. CLI calls `POST /auth/device` → gets `{ device_code, user_code, verification_url, expires_in, interval }`
2. CLI opens browser to `{verification_url}?code={user_code}`
3. User picks a provider (GitHub/Google) on the server's web page
4. OAuth completes, server links the verified email to the device_code
5. CLI polls `POST /auth/device/token` with `{ device_code }` → gets `{ token, user }` when ready

## CLI Commands

Built with `clap` (derive API).

### `login`

```
cargo run -- login              # production (walker.akerud.se)
cargo run -- login --dev        # local dev (localhost:3000, dev token)
```

Authenticates the user via the walker server's device code flow. Opens a browser, waits for authorization, saves the token locally. `--dev` saves to `auth_dev.json` with a hardcoded dev token.

### `logout`

```
cargo run -- logout             # remove production credentials
cargo run -- logout --dev       # remove dev credentials
```

### `enumerate`

```
cargo run -- enumerate
```

Scans for Bluetooth devices and lists them. Walking machine candidates are highlighted in **green**, other devices in **grey**. Output includes device name, address, RSSI, and service UUIDs.

### `walk`

```
cargo run -- walk
```

Scans for a walking machine, connects to it, monitors activity, and sends updates to the server. Displays live data locally. Auto-reconnects on disconnect, keeps scanning if no treadmill found. Press Ctrl+C to stop. Use `--dev` to report to local server.

Both `enumerate` and `walk` accept `--timeout <seconds>` (default: 10) to control scan duration.

### `listen`

```
cargo run -- listen --port 3000
```

Runs the walker server. Handles:
- Authentication (OAuth device code flow, GitHub/Google providers)
- Receives walking data via `POST /api/update`
- Computes calories/distance from speed + MET tables
- Broadcasts live state via `/ws/live` (1s tick)
- Serves dashboard web UI
- Serves leaderboard via `GET /api/leaderboard`
- Stores session data in DB

## Environment Variables

### Server (`walker listen`)

| Variable | Required | Description |
|----------|----------|-------------|
| `WALKER_GITHUB_CLIENT_ID` | No* | GitHub OAuth App client ID |
| `WALKER_GITHUB_CLIENT_SECRET` | No* | GitHub OAuth App client secret |
| `WALKER_GOOGLE_CLIENT_ID` | No* | Google OAuth client ID |
| `WALKER_GOOGLE_CLIENT_SECRET` | No* | Google OAuth client secret |
| `WALKER_BASE_URL` | No | Base URL for callbacks (default: `http://localhost:<port>`) |
| `DATABASE_URL` | No** | PostgreSQL connection string |

\* At least one provider should be configured for production. Use `--dev` for testing without OAuth.

### Local Development

**1. Start Postgres:**
```bash
docker run -d --name walker-postgres \
  -e POSTGRES_PASSWORD=walker \
  -e POSTGRES_DB=walker \
  -p 5432:5432 \
  postgres:16-alpine
```

**2. Create a GitHub OAuth App** (for login testing):
- Go to https://github.com/settings/developers → "New OAuth App"
- Homepage URL: `http://localhost:3000`
- Callback URL: `http://localhost:3000/auth/github/callback`
- Copy Client ID and generate Client Secret

**3. Start the server:**
```bash
DATABASE_URL=postgres://postgres:walker@localhost/walker \
WALKER_GITHUB_CLIENT_ID=your_id \
WALKER_GITHUB_CLIENT_SECRET=your_secret \
cargo run -- listen
```

**4. Login and walk:**
```bash
cargo run -- login                # opens browser, GitHub OAuth
cargo run -- walk                 # connects to treadmill, reports to server
```

**5. Open dashboard:** http://localhost:3000

Migrations run automatically on server startup. To reset the database:
```bash
docker exec -it walker-postgres psql -U postgres walker -c "DROP SCHEMA public CASCADE; CREATE SCHEMA public;"
```

### Dev mode (no OAuth, no Postgres)

`--dev` flag on both `listen` and `login` creates a hardcoded test token (`dev-token-walker`) so you can test without any external services:
```bash
cargo run -- listen --dev         # server with test token, in-memory only
cargo run -- login --dev          # saves test token locally
cargo run -- walk                 # works offline or reports to dev server
```

### Deployment (Render.com)

Production is deployed at `https://walker.akerud.se` via Render.com.

Dockerfile builds server-only (`--no-default-features --features server`) — no BLE dependencies. Docker layer caching means only code changes trigger recompilation.

Set environment variables in Render's dashboard:
```
WALKER_BASE_URL=https://walker.akerud.se
WALKER_GITHUB_CLIENT_ID=...
WALKER_GITHUB_CLIENT_SECRET=...
DATABASE_URL=...  (Render's internal Postgres URL)
```

GitHub OAuth App (production) callback URL: `https://walker.akerud.se/auth/github/callback`
GitHub OAuth App (local dev) callback URL: `http://localhost:3000/auth/github/callback`

Use separate GitHub OAuth Apps for production and local dev.

## BLE Resilience

- Auto-reconnect on device disconnect (3s retry)
- 10s timeout detects silent disconnects (BLE stream hangs)
- Keeps scanning if no treadmill found on startup
- Quick scan (1s) before full scan for fast reconnection
- Step tracker and activity tracker reset baselines on reconnect (no false wraps or stuck IDLE)

## Dependencies

- **clap** — CLI argument parsing (derive, env features)
- **tracing** + **tracing-subscriber** — structured logging
- **btleplug** — cross-platform Bluetooth Low Energy
- **tokio** — async runtime
- **anyhow** — error handling
- **colored** — terminal color output
- **futures** — StreamExt for async streams
- **async-trait** — async methods in TreadmillProfile trait
- **uuid** — BLE UUID handling
- **axum** — HTTP server + WebSocket (json, ws features)
- **tower-http** — CORS middleware
- **serde** + **serde_json** — serialization
- **reqwest** — HTTP client (OAuth token exchange, server reporter)
- **rand** — token/code generation
- **open** — open browser for login flow
- **dirs** — config directory paths
- **sqlx** — async PostgreSQL (with migrations)

## Logging

All logging uses the `tracing` crate. Log levels:
- `info` — session start/stop, device found, connection events
- `debug` — BLE scan details, raw data
- `error` — connection failures, server reporting failures

## Build & Run

```
cargo build
cargo run -- enumerate
cargo run -- walk
cargo run -- walk --dev
cargo run -- login
cargo run -- login --dev
cargo run -- logout
cargo run -- simulate
cargo run -- simulate --dev --count 5
cargo run -- listen --dev
```

## References

- [TreadSpan](https://github.com/blak3r/treadspan) — ESP32 project with UREVO E1L protocol reverse-engineering
- [hassio-ftms](https://github.com/dudanov/hassio-ftms) — Home Assistant FTMS integration
