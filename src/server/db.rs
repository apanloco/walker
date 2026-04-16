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
    /// How many seconds ago this segment started (computed by the DB).
    pub age_secs: f64,
}

/// Get the current open segment for a user, if any.
pub async fn get_open_segment(
    pool: &PgPool,
    user_id: uuid::Uuid,
) -> anyhow::Result<Option<OpenSegment>> {
    let row = sqlx::query(
        "SELECT id, moving, speed_kmh,
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
        age_secs: r.get::<f32, _>("age_secs") as f64,
    }))
}

/// Insert a new open segment. Returns the segment ID.
pub async fn open_segment(
    pool: &PgPool,
    user_id: uuid::Uuid,
    moving: bool,
    speed_kmh: f64,
    weight_kg: f64,
) -> anyhow::Result<i64> {
    let row = sqlx::query(
        "INSERT INTO segments (user_id, started_at, moving, speed_kmh, weight_kg)
         VALUES ($1, NOW(), $2, $3, $4)
         RETURNING id",
    )
    .bind(user_id)
    .bind(moving)
    .bind(speed_kmh as f32)
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
/// active recently and at the same speed. Used to absorb false idle blips.
/// Returns true if a segment was reopened.
pub async fn reopen_previous_walking_segment(
    pool: &PgPool,
    user_id: uuid::Uuid,
    speed_kmh: f64,
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
             ORDER BY started_at DESC LIMIT 1
         )",
    )
    .bind(user_id)
    .bind(MAX_ABSORB_REOPEN_WINDOW_SECS)
    .bind(speed_kmh as f32)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Recompute a segment's duration and distance from a given end-time expression.
/// Calories are computed at query time via SQL functions — not stored.
///   - `end_time`: SQL expression for the segment's end time (e.g. "now()" or "last_heartbeat_at")
///   - `extra_sets`: additional SET clauses (e.g. "open = false")
/// Close a segment: compute final values and mark as closed.
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

/// Get live status (moving, speed) for all users with open segments.
pub async fn get_live_statuses(pool: &PgPool) -> anyhow::Result<HashMap<uuid::Uuid, (bool, f64)>> {
    let rows = sqlx::query("SELECT user_id, moving, speed_kmh FROM segments WHERE open = true")
        .fetch_all(pool)
        .await?;

    Ok(rows
        .iter()
        .map(|r| {
            let user_id: uuid::Uuid = r.get("user_id");
            let moving: bool = r.get("moving");
            let speed: f32 = r.get("speed_kmh");
            (user_id, (moving, speed as f64))
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
        "SELECT started_at::TEXT, moving, speed_kmh, duration_s, weight_kg,
                total_calories(speed_kmh, weight_kg, duration_s) AS calories_kcal,
                active_calories(speed_kmh, weight_kg, duration_s) AS active_calories_kcal,
                met_for_speed(speed_kmh) AS met,
                distance_m
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
                    "duration_s": r.get::<f32, _>("duration_s"),
                    "weight_kg": r.get::<f32, _>("weight_kg"),
                    "calories_kcal": r.get::<f32, _>("calories_kcal"),
                    "active_calories_kcal": r.get::<f32, _>("active_calories_kcal"),
                    "met": r.get::<f32, _>("met"),
                    "distance_m": r.get::<f32, _>("distance_m"),
                    "open": true,
                }
            })
        }
        None => serde_json::json!({"segment": null}),
    };
    Ok(json.to_string())
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

    info!("Seeded dev history for {user_id}");
    Ok(())
}

// -- Query helpers --

#[derive(serde::Serialize, Clone)]
pub struct DailySnapshot {
    pub date: String,
    pub calories_kcal: f64,
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
                COALESCE(SUM(total_calories(speed_kmh, weight_kg, duration_s)), 0)::REAL AS total_kcal,
                COALESCE(SUM(active_calories(speed_kmh, weight_kg, duration_s)), 0)::REAL AS active_kcal,
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
            let kcal: f32 = r.get("total_kcal");
            let active_kcal: f32 = r.get("active_kcal");
            let dist: f32 = r.get("total_dist");
            let dur: f32 = r.get("total_dur");
            DailySnapshot {
                date: r.get("date"),
                calories_kcal: kcal as f64,
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
    pub calories_kcal: f64,
    pub active_calories_kcal: f64,
}

async fn query_leaderboard(
    pool: &PgPool,
    date_filter: &str,
) -> anyhow::Result<Vec<LeaderboardEntry>> {
    let sql = format!(
        "SELECT u.id, u.display_name AS name, u.avatar_url,
                COALESCE(SUM(total_calories(s.speed_kmh, s.weight_kg, s.duration_s)), 0)::REAL AS total_kcal,
                COALESCE(SUM(active_calories(s.speed_kmh, s.weight_kg, s.duration_s)), 0)::REAL AS active_kcal
         FROM users u LEFT JOIN segments s ON u.id = s.user_id AND s.moving = true {date_filter}
         GROUP BY u.id, u.display_name, u.avatar_url
         HAVING COALESCE(SUM(total_calories(s.speed_kmh, s.weight_kg, s.duration_s)), 0) > 0
         ORDER BY total_kcal DESC LIMIT 10"
    );

    let rows = sqlx::query(&sql).fetch_all(pool).await?;

    Ok(rows
        .iter()
        .map(|r| LeaderboardEntry {
            id: r.get::<uuid::Uuid, _>("id"),
            name: r.get("name"),
            avatar_url: r.get("avatar_url"),
            calories_kcal: r.get::<f32, _>("total_kcal") as f64,
            active_calories_kcal: r.get::<f32, _>("active_kcal") as f64,
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
}

pub async fn leaderboard_daily_winners(pool: &PgPool) -> anyhow::Result<Vec<DailyWinnerEntry>> {
    let rows = sqlx::query(
        "SELECT sub.date, sub.user_id, sub.name, sub.avatar_url, sub.active_kcal
         FROM (
           SELECT s.started_at::date::TEXT AS date, u.id AS user_id,
                  u.display_name AS name, u.avatar_url,
                  COALESCE(SUM(active_calories(s.speed_kmh, s.weight_kg, s.duration_s)), 0)::REAL AS active_kcal,
                  ROW_NUMBER() OVER (
                    PARTITION BY s.started_at::date
                    ORDER BY SUM(active_calories(s.speed_kmh, s.weight_kg, s.duration_s)) DESC
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
        })
        .collect())
}
