CREATE TABLE IF NOT EXISTS users (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    email        TEXT UNIQUE NOT NULL,
    display_name VARCHAR(100) NOT NULL,
    avatar_url   TEXT,
    weight_kg    REAL NOT NULL DEFAULT 70.0,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS tokens (
    token       TEXT PRIMARY KEY, -- SHA-256 hash of the plaintext token; plaintext never stored
    user_id     UUID NOT NULL REFERENCES users(id),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at  TIMESTAMPTZ NOT NULL DEFAULT NOW() + INTERVAL '180 days'
);

CREATE INDEX IF NOT EXISTS idx_tokens_user ON tokens(user_id);

-- Segments: each contiguous period of walking or idle.
CREATE TABLE IF NOT EXISTS segments (
    id           BIGSERIAL PRIMARY KEY,
    user_id      UUID NOT NULL REFERENCES users(id),
    started_at   TIMESTAMPTZ NOT NULL,
    moving       BOOLEAN NOT NULL,
    speed_kmh    REAL NOT NULL,
    duration_s   REAL NOT NULL DEFAULT 0,
    open         BOOLEAN NOT NULL DEFAULT true,
    weight_kg    REAL NOT NULL, -- snapshot of user weight at segment creation; allows accurate recalculation if formula changes
    calories_kcal REAL NOT NULL DEFAULT 0,
    distance_m   REAL NOT NULL DEFAULT 0,
    last_heartbeat_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- User history / heatmap / profile queries: WHERE user_id = ? AND started_at >= ?
CREATE INDEX IF NOT EXISTS idx_segments_user_started ON segments (user_id, started_at);

-- Enforce at most one open segment per user (also serves as fast lookup for /api/update)
CREATE UNIQUE INDEX IF NOT EXISTS idx_segments_one_open ON segments (user_id) WHERE open = true;
