# Walker

## Development Philosophy

This project is **spec-driven**. This file (CLAUDE.md) is the absolute source of truth for how the program works. All requirements, commands, and behavior must be documented here before implementation. Implementation details that are too granular for this file may live as comments in code.

## License

MIT. Use super permissive licenses for all code and dependencies where possible.

## Overview

Walker is a real-time treadmill tracking platform. It connects to Bluetooth walking machines, detects actual walking activity, computes honest calories, and serves live data to a dashboard and API for apps and games.

Production: `https://walker.akerud.se`

## Architecture

### File Structure

```
src/
  main.rs          — CLI (clap) + command orchestration
  activity.rs      — ActivityTracker: infers walking/idle from step deltas
  auth.rs          — client-side auth: login flow, token storage (client-only)
  ble.rs           — BLE adapter, scanning, Bluetooth permission check (client-only)
  reporter.rs      — sends updates to server via HTTP POST (client-only)
  device/
    mod.rs         — TreadmillProfile trait, StepTracker, ProfileRegistry (client-only)
    urevo.rs       — UREVO profile implementation (client-only)
  display.rs       — terminal output formatting (client-only)
  server/
    mod.rs         — server startup, wiring, dev setup, startup health checks
    auth.rs        — OAuth: device code flow (CLI) + web login (dashboard), GitHub/Google
    db.rs          — PostgreSQL: migrations, segment CRUD, MET table, queries, dev seed data
    update.rs      — POST /api/update, segment lifecycle (open/close/heartbeat)
    live.rs        — /ws/live WebSocket, simulate register, disconnect checker
    state.rs       — in-memory user state, live broadcast structs
    activity.rs    — GET /api/activity/{id} segment timeline
    dashboard.rs   — serves dashboard (include_str! in prod, ServeDir in dev)
    leaderboard.rs — GET /api/leaderboard with live status merge
    profile.rs     — GET /api/profile/{id} with year history, records, periods
dashboard/
  index.html       — Tailwind CSS + Inter font + Twemoji, nav, leaderboard, profile, activity pages
  app.js           — SPA: leaderboard, profile with heatmap, activity timeline, WebSocket
migrations/
  001_initial.sql  — users, tokens, segments tables
deny.toml          — cargo-deny license/advisory config
Dockerfile         — multi-stage: server-only build with dep caching
reset_db.sh        — recreate local Postgres container
```

### Feature Flags

Two features: `client` (BLE, terminal UI) and `server` (HTTP, WebSocket, DB). Both enabled by default.

- `cargo build` — builds everything (local dev)
- `cargo build --no-default-features --features server` — server only (Docker/production, no BLE deps)

### Data Layers

1. **Raw device data** (`TreadmillData`) — what the treadmill reports. The treadmill lies: distance/calories keep ticking when you step off the belt, but steps stop.

2. **Activity state** — three-phase state machine inferred from step data by the client:
   - **INIT** → **WALKING**: first step increase detected. Only transition out of INIT.
   - **WALKING** → **IDLE**: no step increase for idle timeout (speed-dependent: 10s below 2 km/h, 5s at 2+ km/h).
   - **IDLE** → **WALKING**: step increase detected.
   - **INIT → IDLE**: impossible. Can't claim idle without first confirming walking.
   - **Any reset** (Pausing/Paused/Standby/Off/BLE reconnect) → **INIT**.

   The client does not report to the server during INIT. The first report is always a confirmed state. This prevents false idle segments at startup when the treadmill has a non-zero step counter from a previous session.

   Step counts use `Option<u64>`: `None` = no baseline yet (first reading establishes baseline without triggering WALKING).

3. **Segments** — the source of truth in the database. See [Segment-Based Tracking](#segment-based-tracking).

Steps detect, speed measures, server computes. Steps never cross the wire.

### System Architecture

```
Walker CLI  ──→  POST /api/update    (HTTP, stateless, authenticated)
                      ↓
                 Server (segment-based)
                   ├─ on state change: close old segment, open new one
                   ├─ on heartbeat: update open segment duration + last_seen
                   ├─ broadcasts to /ws/live viewers
                   ↓
Dashboard   ←──  /ws/live            (WebSocket, triggers page refresh)
Dashboard   ←──  GET /api/leaderboard (REST)
Dashboard   ←──  GET /api/profile/{id} (REST)
Dashboard   ←──  GET /api/activity/{id} (REST, segment timeline)
Games       ←──  /ws/live            (same WebSocket)
```

### Client → Server Protocol

```json
POST /api/update
Authorization: Bearer <token>

{"state": "walking", "speed_kmh": 2.0}
```

`state` is one of `walking`, `idle`, or `stopped`. Speed is always in **km/h**.

Sent on state change + every ~1 second heartbeat while connected. Client does **not** report during treadmill `Pausing`/`Paused` state — these trigger an immediate `stopped` + tracker reset instead.

### Segment-Based Tracking

#### Why

Segments are the source of truth for all tracking data. Each segment stores its raw inputs (speed, duration, weight) alongside computed values (calories, distance), making every number **auditable** (verify the math from a single row), **recomputable** (fix a formula and reprocess), and **drift-free** (one multiplication per segment, not thousands of tiny additions).

#### What Is a Segment

A **segment** is a continuous period where the user's state doesn't change:

- **Walking segment** (`moving = true`) — user is on the belt and stepping.
- **Idle segment** (`moving = false`) — belt is running but user isn't stepping.

No segments are created for time offline, disconnected, or with treadmill off. Those are just gaps.

#### Server Behavior

The server keeps the current open segment ID per user in memory.

**On state change** (`walking→idle`, `idle→walking`, speed change):
1. Close current segment (set duration, calories, distance, `open = false`).
2. Insert new segment with `open = true`.

**On stopped:**
1. Close current segment. No new segment.

**On heartbeat** (same state, nothing changed):
1. Update `last_seen` in memory for disconnect detection.
2. Update current segment's duration/calories/distance in DB (crash safety).

**On disconnect** (no heartbeat for 5 seconds):
1. Close current segment using `last_seen` as end time.

#### Crash Recovery

On server startup, close any stale open segments where `started_at + duration_s` is more than 1 minute in the past. Duration was kept fresh by heartbeats, so data is accurate to ~1 second.

#### Daily Totals

All totals computed by `SUM` over segments for a given date. No separate accumulation table.

#### Auditability

Every closed segment stores: `speed_kmh`, `duration_s`, `weight_kg`, `calories_kcal`, `distance_m`. Anyone can verify: duration × speed = distance. MET(speed) × weight × duration / 3600 = calories.

### Calorie Formula

`calories_kcal = MET(speed_kmh) × weight_kg × duration_s / 3600`

Stored on each segment. Weight is also stored per segment — historical segments retain the weight used at the time.

### MET Table (Compendium of Physical Activities, 2024, treadmill-specific)

| km/h | MET |
|------|-----|
| <1.6 | 2.1 |
| 1.6–3.0 | 2.8 |
| 3.2–3.9 | 3.0 |
| 4.0–4.7 | 3.5 |
| 4.8–5.5 | 3.8 |
| 5.6–6.3 | 4.8 |
| 6.4–7.1 | 5.8 |
| 7.2–7.9 | 6.8 |
| ≥8.0 | 8.3 |

Source: [Compendium of Physical Activities — Walking](https://pacompendium.com/walking/)

### Weight

Default 70.0 kg. Stored on each segment at creation time so historical calories remain accurate if weight changes. The Activity page shows weight per segment, making users aware it affects their numbers.

### Server → Viewer Protocol

**`/ws/live`** — broadcasts on state changes (segment open/close) + every 5s disconnect check (not on heartbeats):
```json
{
  "users": [
    {"id": "uuid", "name": "daniel", "avatar_url": "...", "status": "walking", "speed_kmh": 3.2,
     "calories_kcal": 45.2, "distance_m": 2414, "active_secs": 245}
  ]
}
```

**`GET /api/leaderboard`** — sums segments, merges with live status:
```json
{
  "today": [{"id": "uuid", "name": "alice", "calories_kcal": 89.1, "status": "walking", "speed_kmh": 4.0}],
  "weekly": [...],
  "all_time": [...]
}
```

**`GET /api/profile/{id}`** — full year history, records, period calories.

**`GET /api/activity/{id}`** — today's segments for the activity page.

### Dashboard

Single-page app served by the walker server. Files in `dashboard/` directory:
- **Production:** embedded via `include_str!` (single binary)
- **Dev mode (`--dev`):** served from disk (edit, save, refresh — no rebuild)

**Tech stack:** Tailwind CSS (CDN), Inter font (Google Fonts), Twemoji (consistent emoji rendering)

**Leaderboard tab** (default):
- Today / This Week / All Time top 10
- Live status indicators (pulsing green dot for walking, yellow for idle)
- Clickable names → profile page
- Polls server every 5 seconds (client-side `setInterval`, independent of WebSocket)

**Profile page:**
- Hero: avatar, name, streak, live walking badge
- Stats grid: total kcal, km, active time, active days
- Personal records: best day for calories, distance, time
- "You Burned" section: food emoji equivalents (greedy coin-change algorithm)
- GitHub-style daily heatmap: full year, green intensity + gold for 8+ km days
- Last 7 days: horizontal bar chart
- Fetched on page load only (summary data, not live)

**Activity page:**
- Today's segments grouped into sessions (gap > 60 min = separate session)
- Each segment is a mini-card: time range, duration, distance, calories, speed, MET, weight
- Gaps between segments shown as "paused X min Y sec" dashed dividers
- **Two-endpoint architecture** for smart DOM updates:
  - `GET /api/activity/{id}` — closed segments, fetched on page load + WebSocket state changes
  - `GET /api/activity/{id}/current` — the one open segment, polled every 1 second
  - Closed segments rendered once into `#activity-closed`, not replaced on heartbeat
  - Live segment updated in `#activity-live` without touching closed segments
  - No scroll reset, no text selection loss, no full page rebuild

**Login:** dashboard OAuth via cookie (`walker_id`), same callback URL as CLI (state=web distinguishes). Dev mode: visit `/dev/login` to auto-set cookie.

**URL routing:** hash-based SPA (`#leaderboard`, `#profile/<id>`, `#activity/<id>`)

### Database (PostgreSQL)

Required. Migrations run automatically on startup. The server will not start without `DATABASE_URL`.

**users:** `id` (UUID PK, auto-generated), `email` (unique), `display_name` (max 100 chars), `avatar_url`, `weight_kg` (default 70.0), `created_at`

**tokens:** `token` (PK, SHA-256 hashed), `user_id` (UUID FK → users), `created_at`, `expires_at` (default 180 days). Token lookup queries DB directly on each request — no in-memory cache.

**segments:** source of truth for all tracking data
- `id` BIGSERIAL PK, `user_id` UUID FK, `started_at` TIMESTAMPTZ
- `moving` BOOLEAN, `speed_kmh` REAL, `duration_s` REAL, `open` BOOLEAN
- `weight_kg` REAL (snapshot at creation), `calories_kcal` REAL, `distance_m` REAL
- Unique partial index enforces at most one open segment per user
- Composite index on `(user_id, started_at)` for history queries

**Dev seed data:** `--dev` mode generates ~250 random walking days over the past year on first startup.

### Identity

- **Primary key:** UUID (auto-generated, immutable, used everywhere)
- **Email:** unique, used for OAuth provider matching, changeable
- **Email never exposed** to frontend
- Same email from different OAuth providers = same user

### Why Steps Are Only Used for State Detection

Steps are the only honest signal. But they're NOT used for calories/distance because:
1. Step length varies with speed
2. Calories depend on speed (MET tables), not step count
3. Not all treadmills report steps (FTMS doesn't)
4. Speed is accurate when walking (user must match belt)

Design: **steps detect, speed measures, server computes.**

## Supported Devices

### UREVO SpaceWalk E1L (URTM041)

- **BLE Name:** `URTM041` (matched by name prefix, not FTMS UUID — avoids claiming bikes/rowers)
- **Proprietary Service (0xFFF0):** subscribe `0xFFF1` only, write `[0x02, 0x51, 0x0B, 0x03]` to `0xFFF2`
- **19-byte packets at ~3 Hz:** status, speed (0.1 km/h), duration, distance, calories, steps
- **6-byte packets:** status only (off/standby/starting)
- **Also advertises FTMS** (0x1826) and other services — we only subscribe to the proprietary characteristic
- Steps stop when you step off the belt; distance/calories keep ticking

## Authentication

### OAuth Flow (unified callback)

Both CLI (device code flow) and dashboard (web login) use the **same callback URL**: `/auth/github/callback`. The `state` parameter distinguishes them:
- CLI: `state=<user_code>` → completes device code auth
- Dashboard: `state=web` → sets `walker_id` cookie, redirects to `/`

### Providers

- **GitHub:** `WALKER_GITHUB_CLIENT_ID` + `WALKER_GITHUB_CLIENT_SECRET`
- **Google:** `WALKER_GOOGLE_CLIENT_ID` + `WALKER_GOOGLE_CLIENT_SECRET`

Both optional. Login page shows only configured providers.

### Token Security

**Client-side:** `~/.config/walker/auth.json` (production) and `auth_dev.json` (dev):
```json
{"server": "https://walker.akerud.se", "token": "...", "email": "...", "display_name": "..."}
```

**Server-side:** tokens stored as SHA-256 hashes. Plaintext only exists in the client's auth file and in memory during requests. Tokens expire after 180 days.

`--dev` flag on `login`, `logout`, `walk`, `simulate` switches between files.

## CLI Commands

### `login` / `logout`
```
walker login              # production (walker.akerud.se)
walker login --dev        # local dev (localhost:3000, dev token)
walker logout             # remove production credentials
walker logout --dev       # remove dev credentials
```

### `enumerate`
```
walker enumerate          # scan for BLE treadmills (green = matched, grey = other)
```

### `walk`
```
walker walk               # connect to treadmill, report to production server
walker walk --dev         # report to local dev server
```

Auto-reconnects on disconnect. Keeps scanning if no treadmill found. 10s timeout for silent disconnects. Quick 1s scan before full scan. macOS: checks Bluetooth permission before init (prevents CoreBluetooth segfault).

### `simulate`
```
walker simulate                      # simulate as logged-in user at 4.0 km/h
walker simulate --speed 5.0          # custom speed in km/h
walker simulate --dev --count 20     # 20 fake users against local server
```

### `listen`
```
walker listen --dev                  # dev mode, auto-connects to local Postgres
walker listen --port 3000            # production, requires DATABASE_URL
```

Requires `DATABASE_URL` (or `--dev` which defaults to `postgres://postgres:walker@localhost/walker`).

### `set-weight`
```
walker set-weight 78                 # set weight to 78 kg (production)
walker set-weight 78 --dev           # set weight on local dev server
```

Requires login. Updates `users.weight_kg` on the server. New segments use the updated weight.

### Global options
```
walker -v trace walk                 # set log verbosity (trace, debug, info, warn, error)
```

## Environment Variables

| Variable | Description |
|----------|-------------|
| `DATABASE_URL` | PostgreSQL connection string (required; defaults to local Postgres in `--dev`) |
| `WALKER_BASE_URL` | Base URL for OAuth callbacks (default: `http://localhost:<port>`) |
| `WALKER_GITHUB_CLIENT_ID` | GitHub OAuth App client ID |
| `WALKER_GITHUB_CLIENT_SECRET` | GitHub OAuth App client secret |
| `WALKER_GOOGLE_CLIENT_ID` | Google OAuth client ID |
| `WALKER_GOOGLE_CLIENT_SECRET` | Google OAuth client secret |

## Local Development

```bash
./reset_db.sh                        # fresh Postgres in Docker
cargo run -- listen --dev             # auto-connects to local Postgres
cargo run -- login --dev
cargo run -- simulate --dev --count 5
# Open http://localhost:3000/dev/login (sets dashboard cookie)
```

Dev mode: dashboard served from disk (edit HTML/JS, refresh browser, no rebuild). Fake historical data seeded on first startup. Dev login URL logged on server startup.

## Deployment (Render.com)

Production at `https://walker.akerud.se`. Dockerfile builds server-only with dependency caching.

## BLE Resilience

- Auto-reconnect on disconnect (3s retry)
- 10s timeout detects silent disconnects
- Keeps scanning if no treadmill found
- Quick scan (1s) before full scan
- Step and activity trackers reset on Pausing/Paused/Standby/Off and on BLE reconnect
- macOS: Bluetooth permission pre-check prevents CoreBluetooth segfault

## Future Features

Roughly priority-ordered. Nothing here is committed — just ideas worth considering.

### FTMS Device Support
Generic FTMS (Fitness Machine Service) BLE profile. The `TreadmillProfile` trait already supports multiple devices — one new profile unlocks dozens of treadmill brands.

### Goals & Streaks on Leaderboard
Daily/weekly calorie or distance targets. Streaks on the leaderboard (fire emoji next to names).

### Challenges Between Users
Time-boxed duels: "walk 10km this week against a friend."

### Achievements
Milestone-based rewards: "First 100 kcal", "Marathon distance", "30-day streak", etc.

### Live Reactions
Dashboard viewers send quick reactions to someone currently walking.

### Trends & Comparisons
"You walked 15% more this week than last."

### Mobile-Friendly Dashboard
Dashboard should work well on phone screens.

### Push Notifications
"Your streak is about to break!" via service worker / web push.

## References

- [TreadSpan](https://github.com/blak3r/treadspan) — UREVO E1L protocol reverse-engineering
- [hassio-ftms](https://github.com/dudanov/hassio-ftms) — Home Assistant FTMS integration
- [Compendium of Physical Activities — Walking](https://pacompendium.com/walking/) — MET values for treadmill walking speeds
- [2024 Adult Compendium Update (PMC)](https://pmc.ncbi.nlm.nih.gov/articles/PMC10818145/) — Latest revision of the Compendium
