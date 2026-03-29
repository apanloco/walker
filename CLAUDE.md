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
  auth.rs          — client-side auth: login flow, token storage (client-only feature)
  ble.rs           — BLE adapter, scanning, profile-aware device discovery (client-only)
  reporter.rs      — sends updates to server via HTTP POST (client-only)
  device/
    mod.rs         — TreadmillProfile trait, common types, ProfileRegistry (client-only)
    urevo.rs       — UREVO profile implementation (client-only)
  display.rs       — terminal output formatting (client-only)
  server/
    mod.rs         — server startup, wiring, startup health checks (server-only)
    auth.rs        — OAuth: device code flow (CLI) + web login (dashboard), GitHub/Google
    db.rs          — PostgreSQL: migrations, token/user/daily_stats, leaderboard queries, dev seed data
    live.rs        — POST /api/update (event-driven), /ws/live WebSocket, simulate register
    state.rs       — in-memory user state, MET-based calorie computation (micocalories)
    dashboard.rs   — serves dashboard (include_str! in prod, ServeDir in dev)
    leaderboard.rs — GET /api/leaderboard with live status merge
    profile.rs     — GET /api/profile/{id} with year history, records, periods
dashboard/
  index.html       — Tailwind CSS + Inter font + Twemoji, nav, leaderboard, profile pages
  app.js           — SPA: leaderboard, profile with heatmap, food equivalents, WebSocket
migrations/
  001_initial.sql  — users, tokens, daily_stats tables
Dockerfile         — multi-stage: server-only build with dep caching
reset_db.sh        — recreate local Postgres container
```

### Feature Flags

```toml
[features]
default = ["client", "server"]
client = ["btleplug", "colored", "futures", "async-trait", "dirs", "open"]
server = ["axum", "tower-http", "sqlx"]
```

- `cargo build` — builds everything (local dev)
- `cargo build --no-default-features --features server` — server only (Docker/production, no BLE deps)

### Data Layers

1. **Raw device data** (`TreadmillData`) — what the treadmill reports. The treadmill lies: distance/calories keep ticking when you step off the belt, but steps stop.

2. **Activity state** (`ActivityState`) — inferred from raw data. The truth.
   - **Any step increase** → immediately **WALKING**
   - **No step increase for 3 seconds** → **IDLE**

Steps detect, speed measures, server computes. Steps never cross the wire.

### System Architecture

```
Walker CLI  ──→  POST /api/update    (HTTP, stateless, authenticated)
                      ↓
                 Server (event-driven)
                   ├─ computes calories/distance delta
                   ├─ accumulates into daily_stats DB
                   ├─ broadcasts to /ws/live viewers
                   ↓
Dashboard   ←──  /ws/live            (WebSocket, triggers leaderboard refresh)
Dashboard   ←──  GET /api/leaderboard (REST, throttled to every 5s)
Dashboard   ←──  GET /api/profile/{id} (REST, on demand)
Games       ←──  /ws/live            (same WebSocket)
```

### Client → Server Protocol

```json
POST /api/update
Authorization: Bearer <token>

{"moving": true, "speed_mph": 2.0}
```

Two fields only. Sent immediately on state change + every ~1 second heartbeat while walking.

### Server Processing (event-driven)

On each `POST /api/update`:
1. Authenticate token → resolve to user email
2. Compute calories/time/distance **delta** since last update (using old speed/state)
3. Update in-memory state to new values
4. **Accumulate** delta into `daily_stats` DB (at midnight, new row automatically)
5. Broadcast snapshot to `/ws/live` viewers

No tick loop. 5s lightweight timer for disconnect detection only.

**Calorie formula:** `MET(speed) × weight_kg × 1,000,000 / 3600 × elapsed_secs` (micocalories, integer, no drift)

**MET table:** 2 km/h→2.0, 3→2.5, 4→3.0, 5→3.5, 6→4.0 (linearly interpolated)

**Disconnect:** no heartbeat for 5 seconds → status = `disconnected`

### Server → Viewer Protocol

**`/ws/live`** — broadcasts on every update + every 5s disconnect check:
```json
{
  "users": [
    {"id": "uuid", "name": "daniel", "status": "walking", "speed_mph": 2.0,
     "distance_delta_m": 1.4, "calories_kcal": 45.2, "active_secs": 245}
  ]
}
```

**`GET /api/leaderboard`** — merges DB totals with live status:
```json
{
  "today": [{"id": "uuid", "name": "alice", "calories_kcal": 89.1, "status": "walking", "speed_mph": 2.5}],
  "weekly": [...],
  "all_time": [...]
}
```

**`GET /api/profile/{id}`** — full year history, records, period calories:
```json
{
  "id": "uuid", "name": "daniel", "avatar_url": "...", "streak": 5,
  "live": {"status": "walking", "speed_mph": 2.5},
  "totals": {"calories_kcal": 56983, "distance_km": 1025, "active_secs": 108000, "active_days": 270},
  "records": {"best_day_calories_kcal": 450, "best_day_distance_km": 9.09, "best_day_active_secs": 7140},
  "periods": {"today_kcal": 12.8, "week_kcal": 1822, "month_kcal": 6709, "year_kcal": 56984, "all_time_kcal": 56984},
  "heatmap": [{"date": "2025-03-29", "calories_kcal": 12.8, "distance_km": 0.24, "active_secs": 480}, ...],
  "last_7_days": [...]
}
```

### Dashboard

Single-page app served by the walker server. Files in `dashboard/` directory:
- **Production:** embedded via `include_str!` (single binary)
- **Dev mode (`--dev`):** served from disk via `tower_http::ServeDir` (edit, save, refresh — no rebuild)

**Tech stack:** Tailwind CSS (CDN), Inter font (Google Fonts), Twemoji (consistent emoji rendering)

**Leaderboard tab** (default):
- Today / This Week / All Time top 10
- Live status indicators (pulsing green dot for walking, yellow for idle)
- Clickable names → profile page
- Throttled refresh (max every 5s via WebSocket trigger)

**Profile page** (via clicking name or "Profile" nav link):
- Hero: avatar, name, streak (fire emoji), live walking badge
- Stats grid: total kcal, km, active time, active days
- Personal records: best day for calories, distance, time (trophy styling)
- "You Burned" section: food emoji equivalents for Today/Week/Month/Year/All Time
  - Greedy coin-change algorithm: biggest items first (🥤139 kcal → 🍪53 → 🍬23 → 🍭11)
  - Compact mode for large values (🥤×409 instead of 409 individual emojis)
  - Hover any emoji to see name and calories
- GitHub-style heatmap: full year, 18px cells, CSS grid, Monday-start weeks
  - Green intensity (5 levels), gold squares for 8+ km days
  - Hover tooltips with date, calories, distance, food emojis
- Last 7 days: horizontal bar chart

**Login:** dashboard OAuth via cookie (`walker_id`), same callback URL as CLI (state=web distinguishes)

**URL routing:** hash-based SPA (`#leaderboard`, `#profile/<id>`), persists across refresh

### Database (PostgreSQL)

Optional — server works without it (in-memory only). Migrations run automatically on startup.

**users:** `email` (PK), `id` (UUID, public), `display_name`, `avatar_url`, `weight_kg`, `created_at`

**tokens:** `token` (PK), `user_email` (FK), `created_at`

**daily_stats:** `(user_email, date)` (PK), `calories_ucal`, `distance_m`, `active_secs`, `idle_secs`, `updated_at`
- One row per user per day
- Each update ADDS delta (not overwrites)
- At midnight, `CURRENT_DATE` flips → new row automatically

**Dev seed data:** `--dev` mode generates ~250 random walking days over the past year on first startup.

### Identity

- **Internal key:** email (cross-provider matching)
- **Public ID:** UUID (APIs, WebSocket, cookies, profile URLs)
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
- **Proprietary Service (0xFFF0):** subscribe `0xFFF1`, write `[0x02, 0x51, 0x0B, 0x03]` to `0xFFF2`
- **19-byte packets at ~3 Hz:** status, speed (0.1 mph), duration (secs), distance (0.01 km), calories (0.1 kcal), steps
- **6-byte packets:** status only (off/standby/starting)
- Steps stop when you step off the belt; distance/calories keep ticking

## Authentication

### OAuth Flow (unified callback)

Both CLI (device code flow) and dashboard (web login) use the **same callback URL**: `/auth/github/callback`. The `state` parameter distinguishes them:
- CLI: `state=<user_code>` → completes device code auth
- Dashboard: `state=web` → sets `walker_id` cookie, redirects to `/`

One GitHub OAuth App handles both flows.

### Providers

- **GitHub:** `WALKER_GITHUB_CLIENT_ID` + `WALKER_GITHUB_CLIENT_SECRET`
- **Google:** `WALKER_GOOGLE_CLIENT_ID` + `WALKER_GOOGLE_CLIENT_SECRET`

Both optional. Login page shows only configured providers.

### Token Storage

`~/.config/walker/auth.json` (production) and `auth_dev.json` (dev):
```json
{"server": "https://walker.akerud.se", "token": "...", "email": "...", "display_name": "..."}
```

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

Auto-reconnects on disconnect. Keeps scanning if no treadmill found. 10s timeout for silent disconnects. Quick 1s scan before full scan.

### `simulate`
```
walker simulate                      # simulate as logged-in user at 2.5 mph
walker simulate --speed 4.0          # custom speed
walker simulate --dev --count 20     # 20 fake users against local server
```

20 unique names (alice–tara), each with distinct base speed (1.0–5.0 mph spread), ±0.5 mph random variation. Simulate register endpoint gated behind `--dev`.

### `listen`
```
walker listen --dev                  # dev mode, in-memory, test token
walker listen --port 3000            # with env vars for OAuth + DB
```

Startup health checks log configuration status (OAuth, DB, dev mode).

## Environment Variables

| Variable | Description |
|----------|-------------|
| `DATABASE_URL` | PostgreSQL connection string |
| `WALKER_BASE_URL` | Base URL for OAuth callbacks (default: `http://localhost:<port>`) |
| `WALKER_GITHUB_CLIENT_ID` | GitHub OAuth App client ID |
| `WALKER_GITHUB_CLIENT_SECRET` | GitHub OAuth App client secret |
| `WALKER_GOOGLE_CLIENT_ID` | Google OAuth client ID |
| `WALKER_GOOGLE_CLIENT_SECRET` | Google OAuth client secret |

## Local Development

```bash
./reset_db.sh                        # fresh Postgres in Docker
DATABASE_URL=postgres://postgres:walker@localhost/walker cargo run -- listen --dev
cargo run -- login --dev
cargo run -- simulate --dev --count 5
# Open http://localhost:3000
```

Dev mode: dashboard served from disk (edit HTML/JS, refresh browser, no rebuild). Fake historical data seeded on first startup.

## Deployment (Render.com)

Production at `https://walker.akerud.se`. Dockerfile builds server-only with dependency caching (fast rebuilds when only source changes).

```
WALKER_BASE_URL=https://walker.akerud.se
WALKER_GITHUB_CLIENT_ID=...
WALKER_GITHUB_CLIENT_SECRET=...
DATABASE_URL=...  (Render's internal Postgres URL)
```

Separate GitHub OAuth Apps for production and local dev. Same callback URL pattern: `/auth/github/callback`.

## BLE Resilience

- Auto-reconnect on disconnect (3s retry)
- 10s timeout detects silent disconnects
- Keeps scanning if no treadmill found
- Quick scan (1s) before full scan
- Step tracker and activity tracker reset on reconnect

## References

- [TreadSpan](https://github.com/blak3r/treadspan) — UREVO E1L protocol reverse-engineering
- [hassio-ftms](https://github.com/dudanov/hassio-ftms) — Home Assistant FTMS integration
