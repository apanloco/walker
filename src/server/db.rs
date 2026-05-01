use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};
use std::collections::HashMap;
use tracing::info;

/// Hash a token with SHA-256 for storage. Tokens are high-entropy random
/// strings, so a fast hash is sufficient (no need for bcrypt/argon2).
pub fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Run migrations and return the pool.
pub async fn connect(database_url: &str) -> anyhow::Result<PgPool> {
    let pool = PgPool::connect(database_url).await?;
    info!("Connected to database");

    sqlx::migrate!("./migrations").run(&pool).await?;
    info!("Migrations applied");

    Ok(pool)
}

#[derive(Clone)]
pub struct User {
    pub id: uuid::Uuid,
    pub display_name: String,
    pub is_admin: bool,
}

/// Look up a user by ID. Returns Ok(None) if not found, Err on DB failure.
pub async fn get_user(pool: &PgPool, id: uuid::Uuid) -> anyhow::Result<Option<User>> {
    let Some(row) = sqlx::query("SELECT id, display_name, is_admin FROM users WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await?
    else {
        return Ok(None);
    };
    Ok(Some(User {
        id: row.get("id"),
        display_name: row.get("display_name"),
        is_admin: row.get("is_admin"),
    }))
}

/// Upsert user and return their UUID in a single atomic query.
pub async fn upsert_user_returning_id(
    pool: &PgPool,
    email: &str,
    display_name: &str,
    avatar_url: Option<&str>,
) -> anyhow::Result<uuid::Uuid> {
    let row = sqlx::query(
        "INSERT INTO users (email, display_name, avatar_url)
         VALUES ($1, $2, $3)
         ON CONFLICT (email) DO UPDATE SET display_name = $2, avatar_url = $3
         RETURNING id",
    )
    .bind(email)
    .bind(display_name)
    .bind(avatar_url)
    .fetch_one(pool)
    .await?;
    Ok(row.get("id"))
}

/// Check if a user exists by ID.
pub async fn user_exists(pool: &PgPool, id: uuid::Uuid) -> anyhow::Result<bool> {
    let row = sqlx::query("SELECT EXISTS(SELECT 1 FROM users WHERE id = $1) as exists")
        .bind(id)
        .fetch_one(pool)
        .await?;
    Ok(row.get("exists"))
}

/// Set a user's weight.
pub async fn set_user_weight(
    pool: &PgPool,
    user_id: uuid::Uuid,
    weight_kg: f32,
) -> anyhow::Result<()> {
    sqlx::query("UPDATE users SET weight_kg = $2 WHERE id = $1")
        .bind(user_id)
        .bind(weight_kg)
        .execute(pool)
        .await?;
    Ok(())
}

/// Get a user's weight_kg by their public ID.
pub async fn get_user_weight(pool: &PgPool, user_id: uuid::Uuid) -> anyhow::Result<f64> {
    let row = sqlx::query("SELECT weight_kg FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(pool)
        .await?;
    let w: f32 = row.get("weight_kg");
    Ok(w as f64)
}

/// Store a token in the DB (hashed with SHA-256).
pub async fn store_token(pool: &PgPool, token: &str, user_id: uuid::Uuid) -> anyhow::Result<()> {
    sqlx::query("INSERT INTO tokens (token, user_id) VALUES ($1, $2)")
        .bind(hash_token(token))
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Look up a token (by its SHA-256 hash) and return the associated user.
pub async fn find_user_from_token(pool: &PgPool, token: &str) -> anyhow::Result<Option<User>> {
    let row = sqlx::query(
        "SELECT u.id, u.display_name, u.is_admin
         FROM tokens t JOIN users u ON t.user_id = u.id
         WHERE t.token = $1 AND t.expires_at > NOW()",
    )
    .bind(hash_token(token))
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| User {
        id: r.get::<uuid::Uuid, _>("id"),
        display_name: r.get("display_name"),
        is_admin: r.get("is_admin"),
    }))
}

// -- Segment operations --

/// The current open segment for a user.
pub struct OpenSegment {
    pub id: i64,
    pub moving: bool,
    pub speed_kmh: f64,
    pub incline_percent: Option<f64>,
    /// How many seconds ago this segment started (computed by the DB).
    pub age_secs: f64,
}

/// Get the current open segment for a user, if any.
pub async fn get_open_segment(
    pool: &PgPool,
    user_id: uuid::Uuid,
) -> anyhow::Result<Option<OpenSegment>> {
    let row = sqlx::query(
        "SELECT id, moving, speed_kmh, incline_percent,
                EXTRACT(EPOCH FROM now() - started_at)::REAL AS age_secs
         FROM segments WHERE user_id = $1 AND open = true",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| OpenSegment {
        id: r.get("id"),
        moving: r.get("moving"),
        speed_kmh: r.get::<f32, _>("speed_kmh") as f64,
        incline_percent: r.get::<Option<f32>, _>("incline_percent").map(|v| v as f64),
        age_secs: r.get::<f32, _>("age_secs") as f64,
    }))
}

/// Insert a new open segment. Returns the segment ID.
pub async fn open_segment(
    pool: &PgPool,
    user_id: uuid::Uuid,
    moving: bool,
    speed_kmh: f64,
    incline_percent: Option<f64>,
    weight_kg: f64,
) -> anyhow::Result<i64> {
    let row = sqlx::query(
        "INSERT INTO segments (user_id, started_at, moving, speed_kmh, incline_percent, weight_kg)
         VALUES ($1, NOW(), $2, $3, $4, $5)
         RETURNING id",
    )
    .bind(user_id)
    .bind(moving)
    .bind(speed_kmh as f32)
    .bind(incline_percent.map(|v| v as f32))
    .bind(weight_kg as f32)
    .fetch_one(pool)
    .await?;
    Ok(row.get("id"))
}

/// Maximum age (seconds) of a false idle segment that can be absorbed back into
/// the previous walking segment. Keeps idle detection fast on the client while
/// cleaning up sensor noise on the server.
pub const MAX_ABSORB_FLAKY_IDLE_SEGMENT_SECS: f64 = 10.0;

/// Maximum age (seconds) of the previous walking segment's last heartbeat for it
/// to be eligible for reopening during idle absorption.
const MAX_ABSORB_REOPEN_WINDOW_SECS: f64 = 15.0;

/// Delete a segment by ID. Returns true if a row was deleted.
pub async fn delete_segment(pool: &PgPool, segment_id: i64) -> anyhow::Result<bool> {
    let result = sqlx::query("DELETE FROM segments WHERE id = $1")
        .bind(segment_id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// Try to reopen the most recent closed walking segment for a user, if it was
/// active recently and at the same speed and incline. Used to absorb false
/// idle blips. Returns true if a segment was reopened.
pub async fn reopen_previous_walking_segment(
    pool: &PgPool,
    user_id: uuid::Uuid,
    speed_kmh: f64,
    incline_percent: Option<f64>,
) -> anyhow::Result<bool> {
    let result = sqlx::query(
        "UPDATE segments
         SET open = true, last_heartbeat_at = now()
         WHERE id = (
             SELECT id FROM segments
             WHERE user_id = $1
               AND open = false
               AND moving = true
               AND last_heartbeat_at > now() - make_interval(secs => $2)
               AND abs(speed_kmh - $3) < 0.05
               AND abs(COALESCE(incline_percent, 0.0) - COALESCE($4, 0.0)) < 0.05
             ORDER BY started_at DESC LIMIT 1
         )",
    )
    .bind(user_id)
    .bind(MAX_ABSORB_REOPEN_WINDOW_SECS)
    .bind(speed_kmh as f32)
    .bind(incline_percent.map(|v| v as f32))
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Close a segment by computing its final duration and distance, then marking it
/// closed. Calories are computed at query time via SQL functions rather than
/// stored on the row.
pub async fn close_segment(pool: &PgPool, segment_id: i64) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE segments SET
            duration_s = EXTRACT(EPOCH FROM now() - started_at),
            distance_m = CASE WHEN moving THEN
                speed_kmh * 1000.0 / 3600.0 * EXTRACT(EPOCH FROM now() - started_at)
                ELSE 0 END,
            last_heartbeat_at = NOW(),
            open = false
         WHERE id = $1",
    )
    .bind(segment_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Update an open segment's duration, calories, and distance (heartbeat).
pub async fn update_open_segment(pool: &PgPool, segment_id: i64) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE segments SET
            duration_s = EXTRACT(EPOCH FROM now() - started_at),
            distance_m = CASE WHEN moving THEN
                speed_kmh * 1000.0 / 3600.0 * EXTRACT(EPOCH FROM now() - started_at)
                ELSE 0 END,
            last_heartbeat_at = NOW()
         WHERE id = $1",
    )
    .bind(segment_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Update the weight on a user's open segment and recalculate calories.
/// Two-step: set weight first, then recalculate — because PostgreSQL
/// evaluates all SET expressions against pre-update values.
pub async fn update_open_segment_weight(
    pool: &PgPool,
    user_id: uuid::Uuid,
    weight_kg: f32,
) -> anyhow::Result<()> {
    if let Some(seg) = get_open_segment(pool, user_id).await? {
        sqlx::query("UPDATE segments SET weight_kg = $2 WHERE id = $1")
            .bind(seg.id)
            .bind(weight_kg)
            .execute(pool)
            .await?;
        update_open_segment(pool, seg.id).await?;
    }
    Ok(())
}

/// Close stale open segments whose last heartbeat is older than `threshold_secs`.
/// Returns the user IDs of affected users (for notifying viewers).
/// Used for both startup crash recovery (60s) and disconnect detection (5s).
pub async fn close_stale_segments(
    pool: &PgPool,
    threshold_secs: f64,
) -> anyhow::Result<Vec<uuid::Uuid>> {
    let rows = sqlx::query(
        "SELECT id, user_id FROM segments
         WHERE open = true
           AND EXTRACT(EPOCH FROM now() - last_heartbeat_at) > $1",
    )
    .bind(threshold_secs)
    .fetch_all(pool)
    .await?;

    let mut user_ids = Vec::new();
    for row in &rows {
        let id: i64 = row.get("id");
        let user_id: uuid::Uuid = row.get("user_id");
        if let Err(e) = sqlx::query(
            "UPDATE segments SET
                duration_s = EXTRACT(EPOCH FROM last_heartbeat_at - started_at),
                distance_m = CASE WHEN moving THEN
                    speed_kmh * 1000.0 / 3600.0 * EXTRACT(EPOCH FROM last_heartbeat_at - started_at)
                    ELSE 0 END,
                last_heartbeat_at = NOW(),
                open = false
             WHERE id = $1",
        )
        .bind(id)
        .execute(pool)
        .await
        {
            tracing::error!(error = %e, segment_id = id, "Failed to close stale segment");
        } else {
            user_ids.push(user_id);
        }
    }
    Ok(user_ids)
}

/// Live status for one user with an open segment.
pub struct LiveStatus {
    pub moving: bool,
    pub speed_kmh: f64,
    pub active_kcal_per_h: f64,
    /// `None` when the device doesn't report incline.
    pub incline_percent: Option<f64>,
}

/// Get live status for all users with open segments.
pub async fn get_live_statuses(pool: &PgPool) -> anyhow::Result<HashMap<uuid::Uuid, LiveStatus>> {
    let rows = sqlx::query(
        "SELECT s.user_id, s.moving, s.speed_kmh, s.incline_percent,
                active_calories(s.speed_kmh, s.incline_percent, s.weight_kg, 3600.0::REAL)::DOUBLE PRECISION
                  AS active_kcal_per_h
         FROM segments s WHERE s.open = true",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .iter()
        .map(|r| {
            let user_id: uuid::Uuid = r.get("user_id");
            (
                user_id,
                LiveStatus {
                    moving: r.get("moving"),
                    speed_kmh: r.get::<f32, _>("speed_kmh") as f64,
                    active_kcal_per_h: r.get("active_kcal_per_h"),
                    incline_percent: r.get::<Option<f32>, _>("incline_percent").map(|v| v as f64),
                },
            )
        })
        .collect())
}

/// Get the current open segment for a user as JSON (for WebSocket push).
/// Returns `{"segment": {...}}` or `{"segment": null}`.
pub async fn get_current_segment_json(
    pool: &PgPool,
    user_id: uuid::Uuid,
) -> anyhow::Result<String> {
    let row = sqlx::query(
        "SELECT started_at::TEXT, moving, speed_kmh, incline_percent, duration_s, weight_kg,
                active_calories(speed_kmh, incline_percent, weight_kg, duration_s) AS active_calories_kcal,
                distance_m, source,
                NULL::BIGINT AS activity_id,
                NULL::TEXT   AS activity_name,
                NULL::TEXT   AS source_url
         FROM segments
         WHERE user_id = $1 AND open = true",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;

    let json = match row {
        Some(r) => {
            serde_json::json!({
                "segment": {
                    "started_at": r.get::<String, _>("started_at"),
                    "moving": r.get::<bool, _>("moving"),
                    "speed_kmh": r.get::<f32, _>("speed_kmh"),
                    "incline_percent": r.get::<Option<f32>, _>("incline_percent"),
                    "duration_s": r.get::<f32, _>("duration_s"),
                    "weight_kg": r.get::<f32, _>("weight_kg"),
                    "active_calories_kcal": r.get::<f32, _>("active_calories_kcal"),
                    "distance_m": r.get::<f32, _>("distance_m"),
                    "source": r.get::<String, _>("source"),
                    "activity_id": r.get::<Option<i64>, _>("activity_id"),
                    "activity_name": r.get::<Option<String>, _>("activity_name"),
                    "source_url": r.get::<Option<String>, _>("source_url"),
                    "open": true,
                }
            })
        }
        None => serde_json::json!({"segment": null}),
    };
    Ok(json.to_string())
}

// -- Strava --

pub struct StravaConnection {
    pub client_id: String,
    pub client_secret: String,
    pub access_token: String,
    pub refresh_token: String,
    /// True when expires_at < NOW() + 5 minutes (computed by the DB).
    pub needs_refresh: bool,
}

#[allow(clippy::too_many_arguments)]
pub async fn upsert_strava_connection(
    pool: &PgPool,
    user_id: uuid::Uuid,
    athlete_id: i64,
    client_id: &str,
    client_secret: &str,
    access_token: &str,
    refresh_token: &str,
    expires_at_unix: i64,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO strava_connections
             (user_id, athlete_id, client_id, client_secret, access_token, refresh_token, expires_at, last_synced_at)
         VALUES ($1, $2, $3, $4, $5, $6, to_timestamp($7), NOW())
         ON CONFLICT (user_id) DO UPDATE SET
             athlete_id      = $2,
             client_id       = $3,
             client_secret   = $4,
             access_token    = $5,
             refresh_token   = $6,
             expires_at      = to_timestamp($7),
             connected_at    = NOW(),
             last_synced_at  = NOW()",
    )
    .bind(user_id)
    .bind(athlete_id)
    .bind(client_id)
    .bind(client_secret)
    .bind(access_token)
    .bind(refresh_token)
    .bind(expires_at_unix)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_strava_connection(
    pool: &PgPool,
    user_id: uuid::Uuid,
) -> anyhow::Result<Option<StravaConnection>> {
    let row = sqlx::query(
        "SELECT client_id, client_secret, access_token, refresh_token,
                expires_at < NOW() + INTERVAL '5 minutes' AS needs_refresh
         FROM strava_connections WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| StravaConnection {
        client_id: r.get("client_id"),
        client_secret: r.get("client_secret"),
        access_token: r.get("access_token"),
        refresh_token: r.get("refresh_token"),
        needs_refresh: r.get("needs_refresh"),
    }))
}

/// Returns the Unix timestamp of the most recent Strava-sourced segment for a user.
/// Used as the sync baseline so we only fetch activities we haven't seen yet.
pub async fn get_latest_strava_segment_unix(
    pool: &PgPool,
    user_id: uuid::Uuid,
) -> anyhow::Result<Option<i64>> {
    let row = sqlx::query(
        "SELECT EXTRACT(EPOCH FROM MAX(started_at))::BIGINT AS ts
         FROM segments WHERE user_id = $1 AND source = 'strava'",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await?;
    Ok(row.get("ts"))
}

pub async fn update_strava_tokens(
    pool: &PgPool,
    user_id: uuid::Uuid,
    access_token: &str,
    refresh_token: &str,
    expires_at_unix: i64,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE strava_connections SET access_token = $2, refresh_token = $3, expires_at = to_timestamp($4)
         WHERE user_id = $1",
    )
    .bind(user_id)
    .bind(access_token)
    .bind(refresh_token)
    .bind(expires_at_unix)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn update_strava_last_synced(pool: &PgPool, user_id: uuid::Uuid) -> anyhow::Result<()> {
    sqlx::query("UPDATE strava_connections SET last_synced_at = NOW() WHERE user_id = $1")
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn get_strava_last_synced(
    pool: &PgPool,
    user_id: uuid::Uuid,
) -> anyhow::Result<Option<i64>> {
    let row = sqlx::query(
        "SELECT EXTRACT(EPOCH FROM last_synced_at)::BIGINT AS ts FROM strava_connections WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.and_then(|r| r.get("ts")))
}

/// Returns (user_id, last_synced_at as Unix timestamp) for all connected users.
/// last_synced_at is None if the column is NULL (shouldn't happen after migration).
pub async fn get_strava_users_for_sync(
    pool: &PgPool,
) -> anyhow::Result<Vec<(uuid::Uuid, Option<i64>)>> {
    let rows = sqlx::query(
        "SELECT user_id, EXTRACT(EPOCH FROM last_synced_at)::BIGINT AS last_synced_unix
         FROM strava_connections",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| {
            let id: uuid::Uuid = r.get("user_id");
            let ts: Option<i64> = r.get("last_synced_unix");
            (id, ts)
        })
        .collect())
}

pub async fn delete_strava_connection(pool: &PgPool, user_id: uuid::Uuid) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM strava_connections WHERE user_id = $1")
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn strava_connected(pool: &PgPool, user_id: uuid::Uuid) -> anyhow::Result<bool> {
    let row = sqlx::query(
        "SELECT EXISTS(SELECT 1 FROM strava_connections WHERE user_id = $1) AS connected",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await?;
    Ok(row.get("connected"))
}

/// Insert or update an entry in imported_activities. Returns the row ID.
pub async fn upsert_imported_activity(
    pool: &PgPool,
    source: &str,
    external_id: &str,
    name: Option<&str>,
    source_url: Option<&str>,
    raw_data: &serde_json::Value,
) -> anyhow::Result<i64> {
    let row = sqlx::query(
        "INSERT INTO imported_activities (source, external_id, name, source_url, raw_data)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (source, external_id) DO UPDATE SET
             name       = EXCLUDED.name,
             source_url = EXCLUDED.source_url,
             raw_data   = EXCLUDED.raw_data
         RETURNING id",
    )
    .bind(source)
    .bind(external_id)
    .bind(name)
    .bind(source_url)
    .bind(raw_data)
    .fetch_one(pool)
    .await?;
    Ok(row.get("id"))
}

/// Insert a closed segment imported from an external source (e.g. Strava).
/// Returns true if inserted, false if a duplicate (ON CONFLICT DO NOTHING).
#[allow(clippy::too_many_arguments)]
pub async fn insert_imported_segment(
    pool: &PgPool,
    user_id: uuid::Uuid,
    started_at: &str,
    speed_kmh: f32,
    incline_percent: Option<f32>,
    duration_s: f32,
    distance_m: f32,
    weight_kg: f32,
    activity_id: i64,
) -> anyhow::Result<bool> {
    let result = sqlx::query(
        "INSERT INTO segments
             (user_id, started_at, moving, speed_kmh, incline_percent, duration_s, distance_m, weight_kg,
              open, source, activity_id, last_heartbeat_at)
         VALUES ($1, $2::timestamptz, true, $3, $4, $5, $6, $7, false, 'strava', $8, NOW())
         ON CONFLICT (user_id, activity_id) WHERE activity_id IS NOT NULL DO NOTHING",
    )
    .bind(user_id)
    .bind(started_at)
    .bind(speed_kmh)
    .bind(incline_percent)
    .bind(duration_s)
    .bind(distance_m)
    .bind(weight_kg)
    .bind(activity_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

// -- Seed dev data --

/// Seed fake historical data for dev mode using segments.
pub async fn seed_dev_history(pool: &PgPool, user_id: uuid::Uuid) -> anyhow::Result<()> {
    use rand::RngExt;
    let mut rng = rand::rng();

    // Check if already seeded.
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM segments WHERE user_id = $1")
        .bind(user_id)
        .fetch_one(pool)
        .await?;
    if count.0 > 5 {
        return Ok(()); // Already has data.
    }

    info!("Seeding dev history for {user_id}...");

    let weight_kg: f64 = 70.0;

    for days_ago in 1..365 {
        // ~70% chance of walking on any given day.
        if rng.random_range(0..100) > 70 {
            continue;
        }

        // Random start hour (7am-6pm).
        let start_hour = rng.random_range(7..18);
        let start_minute = rng.random_range(0..60);

        // 1-3 walking segments per day.
        let num_segments = rng.random_range(1..=3);
        let mut offset_secs: i32 = 0;

        for _ in 0..num_segments {
            let duration_s: f64 = rng.random_range(600..3600) as f64;
            let speed_kmh: f64 = 2.0 + rng.random_range(0..30) as f64 * 0.1;
            let distance_m = speed_kmh * 1000.0 / 3600.0 * duration_s;

            sqlx::query(
                "INSERT INTO segments (user_id, started_at, moving, speed_kmh, duration_s, open, weight_kg, distance_m)
                 VALUES ($1,
                         (CURRENT_DATE - ($2 || ' days')::INTERVAL) + ($3 || ' seconds')::INTERVAL,
                         true, $4, $5, false, $6, $7)",
            )
            .bind(user_id)
            .bind(days_ago)
            .bind(start_hour * 3600 + start_minute * 60 + offset_secs)
            .bind(speed_kmh as f32)
            .bind(duration_s as f32)
            .bind(weight_kg as f32)
            .bind(distance_m as f32)
            .execute(pool)
            .await?;

            offset_secs += duration_s as i32;

            // Add an idle gap between walking segments.
            if num_segments > 1 {
                let idle_secs = rng.random_range(60..300);
                offset_secs += idle_secs;
            }
        }
    }

    // Strava activities. Raw JSON loaded from strava_seed/ at compile time via
    // include_str!. Dates as relative offsets from today so the heatmap stays
    // populated year-over-year. (external_id, name, days_ago, time_secs,
    // speed_kmh, duration_s, distance_m, raw_json)
    #[allow(clippy::type_complexity)]
    let strava_seed: &[(&str, &str, i32, i32, f32, f32, f32, &str)] = &[(
        "18214327032",
        "Afternoon Run",
        1,
        55566,
        12.58,
        3825.0,
        13364.8,
        include_str!("strava_seed/18214327032.json"),
    )];

    for &(ext_id, name, days_ago, time_secs, speed_kmh, duration_s, distance_m, raw_json) in
        strava_seed
    {
        let source_url = format!("https://www.strava.com/activities/{ext_id}");
        let raw_data: serde_json::Value = serde_json::from_str(raw_json).unwrap_or_default();
        let incline_percent: Option<f32> = raw_data["average_grade"].as_f64().map(|g| g as f32);
        let act_row = sqlx::query(
            "INSERT INTO imported_activities (source, external_id, name, source_url, raw_data)
             VALUES ('strava', $1, $2, $3, $4)
             ON CONFLICT (source, external_id) DO UPDATE SET name = EXCLUDED.name, raw_data = EXCLUDED.raw_data
             RETURNING id",
        )
        .bind(ext_id)
        .bind(name)
        .bind(&source_url)
        .bind(&raw_data)
        .fetch_one(pool)
        .await?;
        let activity_id: i64 = act_row.get("id");

        sqlx::query(
            "INSERT INTO segments
             (user_id, started_at, moving, speed_kmh, incline_percent, duration_s, distance_m, weight_kg, open, source, activity_id)
             VALUES ($1, (CURRENT_DATE - ($2 || ' days')::INTERVAL) + ($3 || ' seconds')::INTERVAL,
                     true, $4, $5, $6, $7, $8, false, 'strava', $9)
             ON CONFLICT (user_id, activity_id) WHERE activity_id IS NOT NULL DO NOTHING",
        )
        .bind(user_id)
        .bind(days_ago)
        .bind(time_secs)
        .bind(speed_kmh)
        .bind(incline_percent)
        .bind(duration_s)
        .bind(distance_m)
        .bind(weight_kg as f32)
        .bind(activity_id)
        .execute(pool)
        .await?;
    }

    info!("Seeded dev history for {user_id}");
    Ok(())
}

// -- Query helpers --

#[derive(serde::Serialize, Clone)]
pub struct DailySnapshot {
    pub date: String,
    pub active_calories_kcal: f64,
    pub distance_km: f64,
    pub active_secs: i32,
}

pub async fn user_history(
    pool: &PgPool,
    user_id: uuid::Uuid,
    days: i32,
) -> anyhow::Result<Vec<DailySnapshot>> {
    let rows = sqlx::query(
        "SELECT started_at::date::TEXT AS date,
                COALESCE(SUM(active_calories(speed_kmh, incline_percent, weight_kg, duration_s)), 0)::REAL AS active_kcal,
                COALESCE(SUM(distance_m), 0)::REAL AS total_dist,
                COALESCE(SUM(duration_s), 0)::REAL AS total_dur
         FROM segments
         WHERE user_id = $1 AND moving = true
           AND started_at >= CURRENT_DATE - ($2 || ' days')::INTERVAL
         GROUP BY started_at::date
         ORDER BY date ASC",
    )
    .bind(user_id)
    .bind(days)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .iter()
        .map(|r| {
            let active_kcal: f32 = r.get("active_kcal");
            let dist: f32 = r.get("total_dist");
            let dur: f32 = r.get("total_dur");
            DailySnapshot {
                date: r.get("date"),
                active_calories_kcal: active_kcal as f64,
                distance_km: dist as f64 / 1000.0,
                active_secs: dur as i32,
            }
        })
        .collect())
}

/// Count consecutive days with activity ending today (streak).
pub async fn user_streak(pool: &PgPool, user_id: uuid::Uuid) -> anyhow::Result<i32> {
    let row = sqlx::query(
        "WITH dates AS (
           SELECT DISTINCT started_at::date AS date
           FROM segments
           WHERE user_id = $1 AND moving = true
         ),
         ranked AS (
           SELECT date, ROW_NUMBER() OVER (ORDER BY date DESC) AS rn
           FROM dates
         )
         SELECT COUNT(*) AS streak FROM ranked
         WHERE date = CURRENT_DATE - (rn - 1)::INT",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await?;

    Ok(row.get::<i64, _>("streak") as i32)
}

#[derive(serde::Serialize)]
pub struct LeaderboardEntry {
    pub id: uuid::Uuid,
    pub name: String,
    pub avatar_url: Option<String>,
    pub active_calories_kcal: f64,
    pub distance_km: f64,
}

async fn query_leaderboard(
    pool: &PgPool,
    date_filter: &str,
) -> anyhow::Result<Vec<LeaderboardEntry>> {
    let sql = format!(
        "SELECT u.id, u.display_name AS name, u.avatar_url,
                COALESCE(SUM(active_calories(s.speed_kmh, s.incline_percent, s.weight_kg, s.duration_s)), 0)::REAL AS active_kcal,
                (COALESCE(SUM(s.distance_m), 0) / 1000.0)::REAL AS distance_km
         FROM users u LEFT JOIN segments s ON u.id = s.user_id AND s.moving = true {date_filter}
         GROUP BY u.id, u.display_name, u.avatar_url
         HAVING COALESCE(SUM(active_calories(s.speed_kmh, s.incline_percent, s.weight_kg, s.duration_s)), 0) > 0
         ORDER BY active_kcal DESC LIMIT 10"
    );

    let rows = sqlx::query(&sql).fetch_all(pool).await?;

    Ok(rows
        .iter()
        .map(|r| LeaderboardEntry {
            id: r.get::<uuid::Uuid, _>("id"),
            name: r.get("name"),
            avatar_url: r.get("avatar_url"),
            active_calories_kcal: r.get::<f32, _>("active_kcal") as f64,
            distance_km: r.get::<f32, _>("distance_km") as f64,
        })
        .collect())
}

pub async fn leaderboard_today(pool: &PgPool) -> anyhow::Result<Vec<LeaderboardEntry>> {
    query_leaderboard(pool, "AND s.started_at::date = CURRENT_DATE").await
}

pub async fn leaderboard_weekly(pool: &PgPool) -> anyhow::Result<Vec<LeaderboardEntry>> {
    query_leaderboard(pool, "AND s.started_at >= CURRENT_DATE - INTERVAL '7 days'").await
}

pub async fn leaderboard_all_time(pool: &PgPool) -> anyhow::Result<Vec<LeaderboardEntry>> {
    query_leaderboard(pool, "").await
}

pub struct DailyWinnerEntry {
    pub date: String,
    pub id: uuid::Uuid,
    pub name: String,
    pub avatar_url: Option<String>,
    pub active_calories_kcal: f64,
    pub distance_km: f64,
}

pub async fn leaderboard_daily_winners(pool: &PgPool) -> anyhow::Result<Vec<DailyWinnerEntry>> {
    let rows = sqlx::query(
        "SELECT sub.date, sub.user_id, sub.name, sub.avatar_url, sub.active_kcal, sub.distance_km
         FROM (
           SELECT s.started_at::date::TEXT AS date, u.id AS user_id,
                  u.display_name AS name, u.avatar_url,
                  COALESCE(SUM(active_calories(s.speed_kmh, s.incline_percent, s.weight_kg, s.duration_s)), 0)::REAL AS active_kcal,
                  (COALESCE(SUM(s.distance_m), 0) / 1000.0)::REAL AS distance_km,
                  ROW_NUMBER() OVER (
                    PARTITION BY s.started_at::date
                    ORDER BY SUM(active_calories(s.speed_kmh, s.incline_percent, s.weight_kg, s.duration_s)) DESC
                  ) AS rn
           FROM segments s JOIN users u ON u.id = s.user_id
           WHERE s.moving = true
             AND s.started_at::date >= CURRENT_DATE - 6
           GROUP BY s.started_at::date, u.id, u.display_name, u.avatar_url
         ) sub
         WHERE sub.rn = 1
         ORDER BY sub.date DESC",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .iter()
        .map(|r| DailyWinnerEntry {
            date: r.get("date"),
            id: r.get("user_id"),
            name: r.get("name"),
            avatar_url: r.get("avatar_url"),
            active_calories_kcal: r.get::<f32, _>("active_kcal") as f64,
            distance_km: r.get::<f32, _>("distance_km") as f64,
        })
        .collect())
}
