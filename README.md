# Walker

Real-time treadmill tracking. Connect a Bluetooth walking machine, track your walks honestly, and compete on a live leaderboard.

**Production:** https://walker.akerud.se

## What It Does

Walker connects to your Bluetooth treadmill, detects when you're actually walking (not just standing on the belt), and computes honest calories using the ACSM walking equation. Data streams to a server that powers a live dashboard with leaderboards, profiles, heatmaps, and history.

**Steps detect, speed measures, server computes.**

## Quick Start

```bash
walker login                 # authenticate (opens browser)
walker walk                  # connect to treadmill and start tracking
```

Open the dashboard at https://walker.akerud.se to see your stats, compete on the leaderboard, and explore your walking history.

## Supported Devices

| Device | Protocol | Status |
|--------|----------|--------|
| UREVO SpaceWalk E1L (URTM041) | Proprietary BLE (0xFFF0) | Supported |
| FTMS treadmills | Bluetooth FTMS | Planned |

## CLI Commands

```
walker login [--dev]       # authenticate (opens browser)
walker logout [--dev]      # remove credentials
walker walk [--dev]        # connect to treadmill and report
walker simulate [--dev]    # simulate walking without hardware
walker enumerate           # scan for BLE treadmills
walker set-weight 78       # set weight in kg
walker listen [--dev]      # run the server
```

## How Calories Work

Walker uses the ACSM walking equation (incline-aware, linear in speed). Calories are computed from speed, incline, weight, and duration — never stored, always calculated fresh. This means formula improvements apply retroactively to all historical data.

Only **active kcal** is shown — the exercise-only number above resting metabolic rate. Resting calories are deliberately excluded: they inflate the number without reflecting effort, which turns leaderboards into a "who sat on the belt longest" contest.

## Architecture

Rust server + vanilla JavaScript dashboard. No frameworks, no build step.

- **Client** sends speed + walking/idle state to the server via HTTP
- **Server** manages segments (continuous periods of walking or idle) in PostgreSQL
- **Dashboard** is a single-page app served as static files, themed with CSS variables
- **WebSocket** pushes live updates to connected viewers

See [CLAUDE.md](CLAUDE.md) for the full spec — it's the source of truth for how everything works.

## Environment Variables

| Variable | Description |
|----------|-------------|
| `DATABASE_URL` | PostgreSQL connection string (defaults to local Postgres in `--dev`) |
| `WALKER_BASE_URL` | Base URL for OAuth callbacks |
| `WALKER_GITHUB_CLIENT_ID` | GitHub OAuth App client ID |
| `WALKER_GITHUB_CLIENT_SECRET` | GitHub OAuth App client secret |
| `WALKER_GOOGLE_CLIENT_ID` | Google OAuth client ID |
| `WALKER_GOOGLE_CLIENT_SECRET` | Google OAuth client secret |

## Local Development

```bash
./reset_db.sh                        # fresh Postgres in Docker
cargo run -- listen --dev             # server with seed data at localhost:3000
cargo run -- login --dev              # click "Dev Login" in browser
cargo run -- simulate --dev --count 5 # simulate walkers
```

## Deployment

Production runs on Render.com. The Dockerfile builds a server-only binary (no BLE dependencies) with dependency caching.

## License

MIT
