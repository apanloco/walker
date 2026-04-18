# Walker

## Development Philosophy

This project is **spec-driven**. This file (CLAUDE.md) is the absolute source of truth for how the program works. All requirements, commands, and behavior must be documented here before implementation. Implementation details that are too granular for this file may live as comments in code.

**Simplicity is a hard requirement.** If something feels complex, stop and simplify before continuing. Prefer deleting code over adding abstractions. Prefer the browser's built-in behavior over reimplementing it in JavaScript. Prefer one SQL query over an in-memory cache. If the simple approach has a tradeoff (e.g., a page flash on navigation), accept it.

**Look broadly before implementing.** Every new feature is an opportunity to simplify what's already there. Before writing new code, check existing structs, queries, and patterns — consolidate, remove dead code, and unify duplicates. Don't add a new thing next to an old thing that does almost the same job.

**Discuss before implementing.** Always propose an approach and get agreement before writing code. Don't jump straight into implementation — discuss the idea and alternatives first. Never enter planning mode unless explicitly asked.

**CLAUDE.md must be updated as part of every task.** Any change to behavior, architecture, protocol, or UI must be reflected here before the task is considered done. This file is what future conversations read first — if it's wrong, everything built on it will be wrong.

## TODO

1. **Activity page: rolling history** — Today's activity shows full segment detail (including live). Below it, show the past 7 days as summarized daily cards (segment count, total active kcal, distance, time — no individual segments). Top card = "Today" (live), below = "Past 7 Days" (not live, fetched once). Reuses the existing `/api/activity/{id}?date=` endpoint per day, or a new summary endpoint.
2. **Parameterize leaderboard date filter** — `query_leaderboard` in `db.rs` uses `format!()` to interpolate the date filter into SQL. The filter values are hardcoded server-side so this isn't injectable, but it breaks the "all queries parameterized" pattern. Refactor to use parameterized queries consistently.
3. **Dashboard session auth** — The `walker_id` cookie stores the raw user UUID, which is publicly visible in the leaderboard API. Anyone who knows a UUID can impersonate that user by setting the cookie. Fix: use a real session token (random, hashed in DB) instead of the UUID. The existing `tokens` table could be reused. This also affects `/ws/live/{id}` — it currently pushes `weight_kg` (needed for live calorie display), so simply removing auth isn't enough. Needs a design that either: (a) secures the WebSocket with a real token, (b) computes calories server-side and strips weight from the push, or (c) accepts that live calorie data implies weight within a range.
4. **Dashboard: stop polling when idle** — The leaderboard polls every 5s unconditionally via `setInterval`, even when the tab is backgrounded or nobody is walking. The WebSocket already notifies on state changes. Consider only polling as a fallback when the WebSocket is disconnected, or pausing the interval when the tab is not visible.
5. **Phase out total kcal** — The platform should show active kcal only. Remove total kcal from: leaderboard hover tooltip, profile stats, and any other UI surfaces. Active kcal is the meaningful number; total kcal includes resting metabolic rate which just adds noise. Server can keep computing both for API backwards compatibility, but the dashboard should stop exposing total.

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
  activity.rs      — ActivityTracker: infers walking/idle from step changes
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
    live.rs        — /ws/live + /ws/live/{id} WebSockets, simulate register, disconnect checker
    activity.rs    — GET /api/activity/{id} segment timeline
    dashboard.rs   — serves dashboard (include_str! in prod, ServeDir in dev)
    leaderboard.rs — GET /api/leaderboard with live status merge
    profile.rs     — GET /api/profile/{id} with year history, records, periods
dashboard/
  index.html       — Tailwind CSS (CDN) + theme system (CSS variables), nav, leaderboard, profile, activity pages
  app.js           — leaderboard, profile with heatmap, activity timeline, WebSocket, theme switcher
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
   - **INIT** → **WALKING**: first step change detected. Only transition out of INIT.
   - **WALKING** → **IDLE**: no step change for idle timeout (see [Timeouts & Intervals](#timeouts--intervals)).
   - **IDLE** → **WALKING**: step change detected.
   - **INIT → IDLE**: impossible. Can't claim idle without first confirming walking.
   - **Any reset** (Pausing/Paused/Standby/Off/BLE reconnect) → **INIT**.

   The client does not report to the server during INIT. The first report is always a confirmed state. This prevents false idle segments at startup when the treadmill has a non-zero step counter from a previous session.

   `StepTracker` returns a `StepChange` enum: `Baseline` (first reading, no comparison yet), `Changed` (raw value differs), `Unchanged` (same as previous). `ActivityTracker` matches on this directly.

3. **Segments** — the source of truth in the database. See [Segment-Based Tracking](#segment-based-tracking).

Steps detect, speed measures, server computes. Steps never cross the wire.

### System Architecture

```
Walker CLI  ──→  POST /api/update    (HTTP, stateless, authenticated)
                      ↓
                 Server (segment-based)
                   ├─ on state change: close old segment, open new one
                   ├─ on heartbeat: update open segment duration + last_heartbeat_at
                   ├─ notifies /ws/live viewers (state changes + disconnect checks)
                   ├─ pushes segment data to /ws/live/{id} subscribers
                   ↓
Dashboard   ←──  /ws/live            (WebSocket, notification-only, triggers REST refetch)
Dashboard   ←──  /ws/live/{id}       (WebSocket, per-user live segment push)
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

Sent on state change + every heartbeat interval while connected. Client does **not** report during treadmill `Pausing`/`Paused` state — these trigger an immediate `stopped` + tracker reset instead.

**Rate limiting:** Authenticated endpoints are rate-limited per Bearer token using `tower_governor` (GCRA algorithm). 10 requests/second sustained, burst of 20. Requests without a Bearer token share a single "unauthenticated" bucket (the handler rejects them with 401 anyway, but this prevents unauthenticated spam). Rate limit is per-token, not per-IP, so multiple users behind the same NAT are not affected.

### Segment-Based Tracking

#### Why

Segments are the source of truth for all tracking data. Each segment stores its raw inputs (speed, duration, weight) alongside computed values (calories, distance), making every number **auditable** (verify the math from a single row), **recomputable** (fix a formula and reprocess), and **drift-free** (one multiplication per segment, not thousands of tiny additions).

#### What Is a Segment

A **segment** is a continuous period where the user's state doesn't change:

- **Walking segment** (`moving = true`) — user is on the belt and stepping.
- **Idle segment** (`moving = false`) — belt is running but user isn't stepping.

No segments are created for time offline, disconnected, or with treadmill off. Those are just gaps.

#### Server Behavior

No in-memory state. The database (segments table) is the single source of truth. Each request queries the DB for the user's open segment.

**On state change** (`walking→idle`, `idle→walking`, speed change):
1. Close current segment (set duration, calories, distance, `open = false`).
2. Insert new segment with `open = true`.

**False idle absorption** (`idle→walking` when idle segment is very short):
Light users (e.g. kids) can cause flaky step detection, creating spurious idle segments. When the server receives a `walking` update and the current open segment is idle with age below the false idle max age:
1. Delete the short idle segment.
2. Reopen the previous walking segment (if it has a recent `last_heartbeat_at` within the false idle reopen window and matching speed).
3. If no eligible previous walking segment exists, fall through to the normal path (open a new walking segment).
This keeps idle detection fast on the client while cleaning up sensor noise on the server.

**On stopped:**
1. Close current segment. No new segment.

**On heartbeat** (same state, nothing changed):
1. Update current segment's duration/distance + `last_heartbeat_at` in DB.

**On disconnect** (no heartbeat for the disconnect threshold):
1. Close current segment using `last_heartbeat_at` as end time (so duration is accurate regardless of detection delay).

#### Crash Recovery

On server startup, close any stale open segments where `last_heartbeat_at` exceeds the crash recovery threshold. Duration was kept fresh by heartbeats, so data is accurate to ~1 second.

#### Daily Totals

All totals computed by `SUM` over segments for a given date. No separate accumulation table.

#### Auditability

Every closed segment stores: `speed_kmh`, `duration_s`, `weight_kg`, `distance_m`. Anyone can verify: duration × speed = distance. Calories are computed at query time via SQL functions — never stored.

### Calorie Formula

Two calorie values computed at query time via PostgreSQL functions (`total_calories()`, `active_calories()`):

- **Total:** `MET(speed_kmh) × weight_kg × duration_s / 3600` — full energy expenditure including resting metabolic rate
- **Active:** `(MET(speed_kmh) - 1) × weight_kg × duration_s / 3600` — exercise-only contribution above resting (MET=1)

Both values are returned in all API responses. The dashboard shows active as primary, total as secondary context.

Calories are **not stored** in the database — they're pure functions of speed, weight, and duration, computed on read. This means formula changes apply retroactively to all historical data with no migration.

### MET Calculation (Compendium of Physical Activities, 2024, treadmill-specific)

The MET lookup is defined once as a PostgreSQL function (`met_for_speed()`) in `migrations/005_interpolate_met.sql`. No duplication in Rust or JavaScript. Uses piecewise linear interpolation between anchor points derived from the Compendium (0% grade, normal gait, no load entries). The anchor points, Compendium codes, and interpolation logic are documented in the migration file.

Source: [Compendium of Physical Activities — Walking](https://pacompendium.com/walking/)

### Weight

Default 70.0 kg. Stored on each segment at creation time so historical calories remain accurate if weight changes. The Activity page shows weight per segment, making users aware it affects their numbers.

### Timezone

UTC everywhere. All timestamps are stored as `TIMESTAMPTZ` (UTC internally). All date boundaries — "today", "this week", heatmap cells — use UTC. The dashboard JavaScript uses `getUTC*()` methods to match the server's `CURRENT_DATE`.

This means for users east of UTC, there's a window after local midnight where "today" on the dashboard still shows the previous UTC day. This is an accepted tradeoff for simplicity — no per-user timezone config, no timezone threading through queries, and the client and server always agree on what "today" means.

### Timeouts & Intervals

All timing constants in one place. Referenced throughout this doc.

| Name | Value | Where | Purpose |
|------|-------|-------|---------|
| Client heartbeat | ~1s | reporter.rs | How often the client sends updates to the server |
| Client idle detection | 3s (≥2 km/h), 5s (<2 km/h) | activity.rs | Speed-dependent: no step change → IDLE |
| BLE silent disconnect | 10s | ble.rs | Detect treadmill that stopped sending data |
| BLE reconnect retry | 3s | ble.rs | Delay before scanning again after disconnect |
| Server disconnect check interval | 5s | live.rs | How often the server checks for stale heartbeats |
| Server disconnect threshold | 30s | live.rs | No heartbeat for this long → close segment |
| Crash recovery threshold | 60s | mod.rs | On startup, close segments stale longer than this |
| False idle max age | 10s | update.rs | Short idle segments below this are absorbed |
| False idle reopen window | 15s | update.rs | Previous walking segment must be this recent to reopen |
| Session gap | 60 min | app.js | Gap between segments that creates a new session |
| Dashboard leaderboard poll | 5s | app.js | Client-side polling interval for leaderboard |
| Token expiry | 180 days | db.rs | Bearer tokens expire after this |
| Update rate limit | 10 req/s, burst 20 | update.rs | Per-token rate limit on authenticated endpoints |

### Server → Viewer Protocol

**`/ws/live`** — notification-only WebSocket. Fires on state changes (segment open/close) + on each disconnect check interval. Sends the string `"update"` with no data — dashboard refetches leaderboard and closed segments via REST on receipt.

**`/ws/live/{id}`** — per-user WebSocket. **Requires login** (`walker_id` cookie). **Own-only:** returns 403 unless the caller is the target user. Pushes the open segment JSON on every heartbeat and state change. Dashboard subscribes when viewing a user's activity page, unsubscribes when navigating away.
```json
{"segment": {"started_at": "...", "moving": true, "speed_kmh": 3.2, "duration_s": 120.5,
             "weight_kg": 70.0, "calories_kcal": 12.3, "active_calories_kcal": 8.5,
             "met": 3.5, "distance_m": 107.1, "open": true}}
```
Returns `{"segment": null}` when the user has no open segment.

**`GET /api/leaderboard`** — sums segments, merges with live status:
```json
{
  "today": [{"id": "uuid", "name": "alice", "calories_kcal": 89.1, "active_calories_kcal": 63.2, "status": "walking", "speed_kmh": 4.0, "met": 3.5}],
  "weekly": [...],
  "all_time": [...],
  "daily_winners": [{"date": "2026-04-16", "id": "uuid", "name": "alice", "avatar_url": "...", "active_calories_kcal": 63.2, "status": "walking", "speed_kmh": 4.0, "met": 3.5}, ...]
}
```

**`GET /api/profile/{id}`** — full year history, records, period calories. **Requires login** (`walker_id` cookie).

**`GET /api/activity/{id}?date=YYYY-MM-DD`** — segments for a given date (defaults to today). **Requires login** (`walker_id` cookie). **Own-only:** returns 403 unless the caller is the target user (weight is shown per segment, so activity is private).

### Dashboard

Single-page app served by the walker server. Files in `dashboard/` directory:
- **Production:** embedded via `include_str!` (single binary)
- **Dev mode (`--dev`):** served from disk (edit, save, refresh — no rebuild)

**Tech stack:** Tailwind CSS (CDN), Twemoji (consistent emoji rendering), Google Fonts (Pixelify Sans, Inter)

**Theming:** Three themes selectable via avatar dropdown menu. Choice persisted in `walker_theme` cookie (1 year, client-side only, no database). Default: Gruvbox.

All colors defined as CSS custom properties (space-separated RGB triplets) on theme classes (`theme-gruvbox`, `theme-c64`, `theme-material`) applied to `<html>`. Tailwind config references these via `rgb(var(--color) / <alpha-value>)` format, enabling opacity modifiers. Theme detection runs in `<head>` before render to prevent flash.

Color categories:
- `surface` (800, 900, 950) — backgrounds
- `gray` (50–950) — text hierarchy
- `walker` (50, 100, 500, 600, 700) — accent color
- `heat` (0–4, gold) — heatmap intensity levels
- `status` (walking, idle) — live status indicators

Sizing that varies by theme uses CSS variables with defaults: `--hm-label-w` (heatmap day label width), `--bar-day-w` / `--bar-kcal-w` (weekly bar column widths).

| Theme | Accent | Font | Extras |
|-------|--------|------|--------|
| Gruvbox (default) | Bright orange `#fe8019` | Inter | Warm charcoal surfaces, cream text |
| C64 | Light blue `#A0A0E0` | Pixelify Sans | No border-radius, scanline overlay, pixel-blink animation, two-color text |
| Material | Purple `#D0BCFF` | Inter | M3 dark palette, elevation shadows, smooth-pulse animation |

Theme-specific CSS handles: font-family, border-radius overrides, animations (pixel-blink vs smooth-pulse), panel styles, scanline overlay (C64 only), font-size scaling (C64 uses 18px root).

**Page code (`app.js`) is theme-unaware.** It uses semantic Tailwind classes (`bg-walker-500`, `bg-heat-3`, `bg-status-walking`) that resolve to different colors per theme via CSS variables. No theme conditionals in page rendering code.

**Navigation:** Logo + tabs (Leaderboard, Activity, FAQ) on the left. Avatar dropdown on the right (Profile, Theme picker, Logout). Activity tab only visible when logged in; Leaderboard and FAQ are public. Profile is accessed via avatar menu (your profile) or by clicking a user on the leaderboard (their profile).

**Leaderboard tab** (default, public — no login required):
- Today / Last 7 Days / All Time top 10
- Daily Winners: 4th panel showing the top active-kcal user for each of the last 7 days. Today's entry updates live with walking status. Each row: day label, avatar, name (links to profile), active kcal.
- Live status indicators (themed walking/idle dots with theme-appropriate animation)
- Clickable names → profile page (redirects to leaderboard if not logged in)
- Polls server on the dashboard leaderboard poll interval + refetches on `/ws/live` notifications

**Profile page** (login required):
- Hero: avatar, name, streak, live walking badge
- Last 7 days: horizontal bar chart with live indicator (blinking dot next to today when walking/idle). Bars show "active kcal" label. Refetched on `/ws/live` notifications so bars update live while walking.
- GitHub-style daily heatmap: full year, themed intensity + gold for 8+ km days, clickable cells → activity page for that date
- Stats grid: total kcal, km, active time, active days
- Personal records: best day for calories, distance, time
- "You Burned" section: food emoji equivalents (greedy coin-change algorithm)

**FAQ page** (public — no login required):
- Static Q&A explaining how Walker works (active vs total kcal, MET, segments, steps, privacy, etc.)
- Grouped into three sections: The Numbers / How Tracking Works / Platform
- Uses native `<details>`/`<summary>` for expand/collapse — no JavaScript
- Route: `/faq`. All content lives inline in `index.html`; theme-unaware (uses semantic Tailwind classes)

**Activity page** (login required, own-only — 403 for other users, no admin override):
- Segments for a given date, grouped into sessions (gap > 60 min = separate session)
- Supports `?date=YYYY-MM-DD` query param, defaults to today
- Newest session first, newest segment first within each session
- Each segment is a mini-card: time range, duration, distance, calories, speed, MET, weight
- Gaps between segments shown as "paused X min Y sec" dividers
- **Two-channel architecture** for smart DOM updates (today only):
  - `GET /api/activity/{id}?date=` — closed segments, fetched on page load + `/ws/live` notifications
  - `/ws/live/{id}` — live segment pushed by server on every heartbeat and state change
  - Closed segments rendered once into `#activity-closed`, not replaced on heartbeat
  - Live segment updated in `#activity-live` without touching closed segments
  - Per-user WebSocket connected on page load only for today, auto-reconnects on disconnect
  - Historical dates show closed segments only (no WebSocket)

**Login:** navigating to a login-required page while logged out redirects to `/login`. Login page is server-rendered with buttons for configured providers. After OAuth, `walker_id` cookie is set and user is redirected to `/`. Dev mode: "Dev Login" button available (no auto-login).

**URL routing:** Full page navigation with real URLs (`/`, `/profile/<id>`, `/activity/<id>`). No client-side routing — all navigation uses `<a href>` links and full page loads. Server catch-all serves `index.html` for all non-API paths. `initPage()` reads `location.pathname` once on load and shows the right content. Legacy `#hash` URLs redirect automatically.

### Database (PostgreSQL)

Required. Migrations run automatically on startup. The server will not start without `DATABASE_URL`.

**users:** `id` (UUID PK, auto-generated), `email` (unique), `display_name` (max 100 chars), `avatar_url`, `weight_kg` (default 70.0), `is_admin` (default false), `created_at`

**tokens:** `token` (PK, SHA-256 hashed), `user_id` (UUID FK → users), `created_at`, `expires_at` (default 180 days). Token lookup queries DB directly on each request — no in-memory cache.

**segments:** source of truth for all tracking data
- `id` BIGSERIAL PK, `user_id` UUID FK, `started_at` TIMESTAMPTZ
- `moving` BOOLEAN, `speed_kmh` REAL, `duration_s` REAL, `open` BOOLEAN
- `weight_kg` REAL (snapshot at creation), `distance_m` REAL
- `last_heartbeat_at` TIMESTAMPTZ (updated on every heartbeat, used for disconnect detection)
- Unique partial index enforces at most one open segment per user
- Composite index on `(user_id, started_at)` for history queries

**Dev seed data:** `--dev` mode generates ~250 random walking days over the past year on first startup.

### Identity

- **Primary key:** UUID (auto-generated, immutable, used everywhere)
- **Email:** unique, used for OAuth provider matching, changeable
- **Email never exposed** to frontend — unless the viewer is an admin
- Same email from different OAuth providers = same user
- **Admin:** `is_admin` flag on the users table. Admins see extra info (e.g. email) on profile pages. Set via direct SQL: `UPDATE users SET is_admin = true WHERE email = '...'`

### Why Steps Are Only Used for State Detection

Steps are the only honest signal. But they're NOT used for calories/distance because:
1. Step length varies with speed
2. Calories depend on speed (MET tables), not step count
3. Not all treadmills report steps (FTMS doesn't)
4. Speed is accurate when walking (user must match belt)

Design: **steps detect, speed measures, server computes.**

## Supported Devices

All UREVO devices share the same proprietary data stream and control protocol (model matched by `URTM` name prefix). Per-model capabilities are declared in `capabilities_for()` in `src/device/urevo.rs` — add new URTM models there.

| Model | BLE Name | Speed control | Incline control | Speed range |
|-------|----------|---------------|-----------------|-------------|
| UREVO SpaceWalk E1L | `URTM041` | ✓ | — | 1.0–6.0 km/h (verified via `walker probe`) |
| UREVO CyberPad | `URTM051` | ✓ | ✓ (not wired in CLI yet) | 1.0–6.0 km/h (unverified — run `walker probe`) |

### Proprietary data stream (all URTM models)

- **Service `0xFFF0`:** subscribe to `0xFFF1`, write `[0x02, 0x51, 0x0B, 0x03]` to `0xFFF2` to activate streaming
- **19-byte packets at ~3 Hz:** status, speed (0.1 km/h), duration, distance, calories, steps
- **6-byte packets:** status only (off/standby/starting)
- Matched by name prefix only — FTMS UUID alone would claim bikes/rowers too
- Steps stop when you step off the belt; distance/calories keep ticking

### Proprietary control commands (same channel the iOS app uses)

Speed, start, and stop are written to the same `0xFFF2` characteristic that activates the data stream. Protocol reverse-engineered from PacketLogger captures of the UREVO iOS app against a URTM041 (raw `.pklg` files live in `captures/` for reference; `build_set_speed_cmd()` in `urevo.rs` encodes the speed frame):

- **Frame:** `0x02 <cmd> [data…] <checksum> 0x03`
- **Checksum:** `sum(cmd + data bytes) XOR 0x5A` — single byte, immediately before the `0x03` terminator
- **Set Speed:** `02 53 02 <u16 LE in 0.1 km/h> <checksum> 03`. Example for 1.5 km/h: `02 53 02 0F 00 3E 03`.
- **Start session:** requires a short handshake the iOS app performs — sending the start command alone is silently accepted but ignored. `UrevoProfile::start()` replays the app's exact sequence:
  1. Write `02 50 03 09 03` (query — device responds; contents irrelevant, the exchange itself is what the treadmill's state machine needs to see).
  2. Wait ~200ms.
  3. Write `02 53 01 00 00 00 00 00 00 00 00 0E 03` (8 zero data bytes, presumed workout targets — time/distance/calories set to none).
- **Stop:** `02 53 0A <checksum> 03`. Example: `02 53 0A 07 03`.
- **Activation (stream start):** `02 51 0B 03` — the short 4-byte variant has no checksum; it appears to be a fixed bootstrap command.
- **Command acknowledgements:** when the treadmill accepts a `02 53 …` control write, it echoes the frame back on the `0xFFF1` notification channel (identical bytes for speed writes; a 6-byte `02 53 01 03 0D 03`-style "success" frame for start). The parser recognizes these and surfaces them as `TreadmillEvent::CommandAck`, which the walk loop silently discards.

All writes use `WriteType::WithoutResponse`. Values are clamped to the model's declared speed range before sending.

**Why not FTMS?** UREVO devices expose the standard FTMS Control Point at `0x2AD9` and it accepts writes, but using it on the E1L has two compounding problems: (1) any FTMS write silences the proprietary `0xFFF1` notification stream until re-activated, and (2) re-activating too soon cancels the pending speed change. The only workable FTMS flow required a 1.5s delay, which added visible lag. The proprietary channel has neither issue — changes apply instantly, the stream keeps flowing. The iOS app uses this channel too, so it's clearly the intended path.

### TreadmillProfile trait

Profile impls live in `src/device/urevo.rs` (or a new file per brand). Each profile exposes:
- `full_name(device_name)` — human-friendly model name for the startup banner
- `capabilities(device_name)` — what controls are available for this specific BLE-advertised model
- `set_speed(device, kmh)` — writes the proprietary speed command to `0xFFF2` (default impl: errors)
- `start(device)` — sends the query + start handshake so `walker walk --start` can begin a session without the user reaching for the remote (default impl: errors)
- `parse_notification(uuid, data)` — parses notification bytes into `TreadmillEvent`s (`Data`, `StatusOnly`, `CommandAck`, or `Unknown`)

Capabilities are resolved at connect time from the BLE `local_name` so one profile can describe multiple sibling models.

## Authentication

### Login Page (`/login`)

Server-rendered page at `/login`. Shows tagline, buttons for each configured provider, and a GitHub link for onboarding. In dev mode, also shows a "Dev Login" button. The same page handles both flows:

- **Dashboard (web) login:** user navigates to `/login` (or is redirected there). No `cli_port` param. After OAuth, sets `walker_id` cookie and redirects to `/`.
- **CLI login:** user runs `walker login`, CLI starts a local HTTP server on a random port, opens browser to `/login?cli_port=P`. After OAuth, server redirects browser to `http://localhost:P/callback?token=...&email=...&name=...`. CLI receives it, saves credentials, done.

Only one login page, one template, one place to add/remove providers.

### OAuth Flow (localhost callback)

Each provider has one callback URL (e.g., `/auth/github/callback`, `/auth/google/callback`). The `state` parameter distinguishes CLI from dashboard:
- CLI: `state=cli:<port>` → server redirects browser to `http://localhost:<port>/callback?token=...` after auth
- Dashboard: `state=web` → sets `walker_id` cookie, redirects to `/`

**CLI login lifecycle:**
1. CLI starts local HTTP server on random port `P`
2. CLI opens browser to `<server>/login?cli_port=P`
3. User clicks a provider, completes OAuth normally
4. Server creates user + token, redirects browser to `http://localhost:P/callback?token=XXX&email=...&name=...`
5. CLI's local server receives the request, saves credentials to `auth.json`, serves "Success! Return to your terminal."
6. CLI shuts down local server, prints confirmation

No polling, no device codes, no in-memory state. The OAuth secrets stay on the server (CLI never sees them). `ServerState` is read-only config behind `Arc` — no `RwLock` needed.

### Providers

All optional. Login page shows only configured/available providers.

- **Dev:** available only in `--dev` mode. No external service — `/auth/dev/callback` creates/upserts a dev user (`dev@walker.local` / "Dev User") and completes the flow using the same code paths as real providers (upsert, token creation, redirect).

**GitHub setup:**
1. Go to GitHub → Settings → Developer Settings → OAuth Apps → New OAuth App
2. Set "Authorization callback URL" to `https://walker.akerud.se/auth/github/callback` (prod) or `http://localhost:3000/auth/github/callback` (dev)
3. Set `WALKER_GITHUB_CLIENT_ID` and `WALKER_GITHUB_CLIENT_SECRET`

**Google setup:**
1. Go to [Google Cloud Console](https://console.cloud.google.com/) → APIs & Services → Credentials
2. Create Credentials → OAuth 2.0 Client ID → Web application
3. Under "Authorized redirect URIs", add `https://walker.akerud.se/auth/google/callback` (prod) and/or `http://localhost:3000/auth/google/callback` (dev)
4. Set `WALKER_GOOGLE_CLIENT_ID` and `WALKER_GOOGLE_CLIENT_SECRET`

### Stale Cookie Recovery

Middleware checks the `walker_id` cookie on every request. If the cookie references a user that doesn't exist in the database (e.g., after `reset_db.sh`), the cookie is cleared and the request continues as unauthenticated. No error page — the user just sees the logged-out state and can log in again.

### Token Security

**Client-side:** `~/.config/walker/auth.json` (production) and `auth_dev.json` (dev):
```json
{"server": "https://walker.akerud.se", "token": "...", "email": "...", "display_name": "..."}
```

**Server-side:** tokens stored as SHA-256 hashes. Plaintext only exists in the client's auth file and in memory during requests. Tokens expire after the token expiry period.

`--dev` flag on `login`, `logout`, `walk`, `simulate` switches between files.

### XSS & SQL Injection

**XSS:** User-controlled data (names, avatar URLs, emails) comes from OAuth providers and is stored raw in the database. The `esc()` helper in `app.js` escapes all user-controlled strings before HTML insertion (uses `textContent`→`innerHTML` to escape `<`, `>`, `&`, `"`). Escaping happens on render, not on storage — this preserves the original data and lets each rendering context (HTML, JSON) escape appropriately.

**SQL injection:** All database queries use parameterized bindings (`$1`, `$2` via sqlx). User input never touches SQL strings. No dynamic SQL construction from user data.

### Dev Mode Auth

Dev mode requires full login, same as production. No auto-injected cookies or hardcoded tokens. The only difference is the dev provider is available:

1. Start server: `cargo run -- listen --dev` (seeds dev user + history, but no auto-login)
2. Dashboard: go to `localhost:3000` → see login page → click "Dev Login" → logged in
3. CLI: `walker login --dev` → opens browser to login page → click "Dev Login" → token saved

This exercises the full auth pipeline (upsert, token creation, cookie/redirect) with zero external dependencies.

## CLI Commands

### `login` / `logout`
```
walker login              # production (walker.akerud.se)
walker login --dev        # local dev (localhost:3000, opens browser to login page)
walker logout             # remove production credentials
walker logout --dev       # remove dev credentials
```

### `enumerate`
```
walker enumerate          # scan for BLE treadmills (green = matched, grey = other)
```

### `probe`
```
walker probe              # connect to the first matched treadmill and dump its FTMS capabilities
```

Read-only. Reports the device's advertised speed range, inclination range, and Machine Feature bitmask — useful when adding a new model's capabilities to `capabilities_for()`. Does not activate the data stream or issue any write.

### `walk`
```
walker walk               # connect to treadmill, report to production server
walker walk --dev         # report to local dev server
walker walk --offline     # run without reporting (no login required)
walker walk --start       # auto-start the belt after connect (safety: only use when ready to walk)
```

On connect, prints a banner like `Connected to device: UREVO SpaceWalk E1L (URTM041)` followed by control hints for the model.

**Speed control (when supported by the device).** On connect, if the profile's `capabilities().speed_control` is true, the terminal enters raw mode and captures arrow keys:
- `↑` / `↓` adjust target speed by 0.1 km/h, clamped to the model's `speed_range_kmh`
- `Ctrl+C` or `q` stops the command and restores cooked mode
- **Target mirrors the device's reported speed on every Running data packet**, except for a short ~750 ms grace window after we issue a `set_speed` write (the treadmill takes ~300 ms to reflect the new target, so a stale in-flight data packet would otherwise undo our press). This covers the initial sync at session start AND any remote-induced changes mid-session. The one other exception is Pausing — the device reports a decreasing speed as the belt winds down, but we bail out of the sync path in that branch anyway.
- **Arrow keys are only honoured when `last_status == Running`**. The treadmill silently ignores speed writes in other states, so we suppress the keypress entirely (no write, no print, no beep) rather than faking a target change.
- Target is *not* reset on Pausing/Paused — it still reflects the last commanded speed, which is useful context. Only a transition to Standby/Off (real session end) resets it to 1.0.

Each press sends the proprietary speed command to `0xFFF2`. Speed changes are not reported to the server directly — the observed speed from the proprietary stream is what gets logged.

When the profile has no speed control, raw mode is not entered and the command behaves as before.

Auto-reconnects on disconnect. Keeps scanning if no treadmill found. macOS: checks Bluetooth permission before init (prevents CoreBluetooth segfault). See [Timeouts & Intervals](#timeouts--intervals) for BLE timing.

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
cargo run -- listen --dev             # auto-connects to local Postgres, seeds history
cargo run -- login --dev              # opens browser → click "Dev Login"
cargo run -- simulate --dev --count 5
# Dashboard: open http://localhost:3000 → click "Dev Login" on login page
```

Dev mode: dashboard served from disk (edit HTML/JS, refresh browser, no rebuild). Fake historical data seeded on first startup. Full login required (no auto-injected cookies) — use the "Dev Login" button on the login page.

## Deployment (Render.com)

Production at `https://walker.akerud.se`. Dockerfile builds server-only with dependency caching.

## BLE Resilience

- Auto-reconnect on disconnect (see [Timeouts & Intervals](#timeouts--intervals) for timing)
- Keeps scanning if no treadmill found
- Checks adapter's peripheral cache before scanning (instant hit on reconnects)
- Step and activity trackers reset on Pausing/Paused/Standby/Off and on BLE reconnect
- macOS: Bluetooth permission pre-check prevents CoreBluetooth segfault

## Future Features

Roughly priority-ordered. Nothing here is committed — just ideas worth considering.

### Web BLE: Walk from the Browser
Connect to a treadmill directly from the browser using the Web Bluetooth API, no CLI needed. A dedicated `/walk` page opens in a separate tab, handles BLE scanning/connection, protocol parsing, activity detection, and POSTs updates to the server. The user browses the dashboard normally in other tabs.

**Requirements:** Chromium-only (no Firefox/Safari). HTTPS or localhost. User gesture required to trigger BLE scan. Requires reimplementing UREVO protocol parsing and StepChange/ActivityTracker state machine in JavaScript (duplication with Rust client). Tab must stay open — browser may throttle/disconnect BLE if the tab is backgrounded too long. Best as a "quick start" option alongside the CLI, not a full replacement.

### BLE Device Control: Incline from CLI
Wire the URTM051 (CyberPad) incline capability to key bindings in `walker walk` (e.g. left/right arrows). The FTMS opcode is `0x03 <i16 LE in 0.1%>`. Note the CyberPad physically stops on incline change, so we'd need to auto-resume at the previous speed afterward (see `set_incline` in the reference Python script).

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
