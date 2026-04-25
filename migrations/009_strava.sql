-- Per-user Strava credentials. Each user supplies their own Strava API app
-- credentials (client_id + client_secret from strava.com/settings/api) so
-- Walker does not need a central Strava app with multi-athlete quota approval.
-- Access tokens expire every 6 hours; Walker refreshes them inline using the
-- per-user client_id/client_secret before any API call.
CREATE TABLE strava_connections (
    user_id        UUID PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    athlete_id     BIGINT NOT NULL,
    client_id      TEXT NOT NULL,
    client_secret  TEXT NOT NULL,
    access_token   TEXT NOT NULL,
    refresh_token  TEXT NOT NULL,
    expires_at     TIMESTAMPTZ NOT NULL,
    connected_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_synced_at TIMESTAMPTZ
);

-- One row per imported external activity (e.g. a Strava activity).
-- Segments reference this table via activity_id for deduplication and metadata.
-- raw_data stores the full API response so the user can inspect what was imported.
CREATE TABLE imported_activities (
    id          BIGSERIAL PRIMARY KEY,
    source      TEXT NOT NULL,
    external_id TEXT NOT NULL,
    name        TEXT,
    source_url  TEXT,
    raw_data    JSONB NOT NULL DEFAULT '{}',
    imported_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (source, external_id)
);

-- Source provenance on segments. 'ble' for treadmill, 'strava' for imported.
-- activity_id references the imported_activities row for Strava segments.
ALTER TABLE segments ADD COLUMN source TEXT NOT NULL DEFAULT 'ble';
ALTER TABLE segments ADD COLUMN activity_id BIGINT REFERENCES imported_activities(id);

-- Deduplication: at most one segment per (user, imported activity).
CREATE UNIQUE INDEX idx_segments_activity_dedup
    ON segments (user_id, activity_id) WHERE activity_id IS NOT NULL;
