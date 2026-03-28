CREATE TABLE IF NOT EXISTS users (
    email        TEXT PRIMARY KEY,
    id           TEXT UNIQUE NOT NULL,
    display_name TEXT NOT NULL,
    avatar_url   TEXT,
    weight_kg    REAL NOT NULL DEFAULT 70.0,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS tokens (
    token       TEXT PRIMARY KEY,
    user_email  TEXT NOT NULL REFERENCES users(email),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS daily_stats (
    user_email    TEXT NOT NULL REFERENCES users(email),
    date          DATE NOT NULL DEFAULT CURRENT_DATE,
    calories_ucal BIGINT NOT NULL DEFAULT 0,
    distance_m    REAL NOT NULL DEFAULT 0,
    active_secs   INTEGER NOT NULL DEFAULT 0,
    idle_secs     INTEGER NOT NULL DEFAULT 0,
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (user_email, date)
);

CREATE INDEX IF NOT EXISTS idx_tokens_user ON tokens(user_email);
