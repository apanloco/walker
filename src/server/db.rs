use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};
use std::collections::HashMap;
use tracing::info;

use super::live::{LiveBroadcast, LiveUser, TokenUser};

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

/// Ensure a user exists in the DB. Upserts display_name and avatar on login.
pub async fn upsert_user(
    pool: &PgPool,
    email: &str,
    display_name: &str,
    avatar_url: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO users (email, display_name, avatar_url)
         VALUES ($1, $2, $3)
         ON CONFLICT (email) DO UPDATE SET display_name = $2, avatar_url = $3",
    )
    .bind(email)
    .bind(display_name)
    .bind(avatar_url)
    .execute(pool)
    .await?;
    Ok(())
}

/// Get a user's public UUID by email.
pub async fn get_user_id(pool: &PgPool, email: &str) -> anyhow::Result<uuid::Uuid> {
    let row = sqlx::query("SELECT id FROM users WHERE email = $1")
        .bind(email)
        .fetch_one(pool)
        .await?;
    Ok(row.get("id"))
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
pub async fn lookup_token(pool: &PgPool, token: &str) -> anyhow::Result<Option<TokenUser>> {
    let row = sqlx::query(
        "SELECT u.id, u.email, u.display_name, u.avatar_url
         FROM tokens t JOIN users u ON t.user_id = u.id
         WHERE t.token = $1 AND t.expires_at > NOW()",
    )
    .bind(hash_token(token))
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| TokenUser {
        id: r.get::<uuid::Uuid, _>("id"),
        email: r.get("email"),
        display_name: r.get("display_name"),
        avatar_url: r.get("avatar_url"),
    }))
}

// -- Segment operations --

/// The current open segment for a user.
pub struct OpenSegment {
    pub id: i64,
    pub moving: bool,
    pub speed_kmh: f64,
}

/// Get the current open segment for a user, if any.
pub async fn get_open_segment(
    pool: &PgPool,
    user_id: uuid::Uuid,
) -> anyhow::Result<Option<OpenSegment>> {
    let row = sqlx::query(
        "SELECT id, moving, speed_kmh FROM segments WHERE user_id = $1 AND open = true",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| OpenSegment {
        id: r.get("id"),
        moving: r.get("moving"),
        speed_kmh: r.get::<f32, _>("speed_kmh") as f64,
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

/// Recompute a segment's duration, calories, and distance from a given end-time expression.
/// All calorie/distance math lives here — one formula, one place.
///   - `end_time`: SQL expression for the segment's end time (e.g. "now()" or "last_heartbeat_at")
///   - `extra_sets`: additional SET clauses (e.g. "open = false" or "weight_kg = $3")
fn open_segment_update_sql(end_time: &str, extra_sets: &str) -> String {
    let comma = if extra_sets.is_empty() { "" } else { "," };
    format!(
        "UPDATE segments SET
            duration_s = EXTRACT(EPOCH FROM {end_time} - started_at),
            calories_kcal = CASE WHEN moving THEN
                ($2 * weight_kg * EXTRACT(EPOCH FROM {end_time} - started_at) / 3600.0)::REAL
                ELSE 0 END,
            distance_m = CASE WHEN moving THEN
                speed_kmh * 1000.0 / 3600.0 * EXTRACT(EPOCH FROM {end_time} - started_at)
                ELSE 0 END,
            last_heartbeat_at = NOW()
            {comma} {extra_sets}
         WHERE id = $1"
    )
}

/// Close a segment: compute final values and mark as closed.
pub async fn close_segment(pool: &PgPool, segment_id: i64, met: f64) -> anyhow::Result<()> {
    sqlx::query(&open_segment_update_sql("now()", "open = false"))
        .bind(segment_id)
        .bind(met as f32)
        .execute(pool)
        .await?;
    Ok(())
}

/// Update an open segment's duration, calories, and distance (heartbeat).
pub async fn update_open_segment(pool: &PgPool, segment_id: i64, met: f64) -> anyhow::Result<()> {
    sqlx::query(&open_segment_update_sql("now()", ""))
        .bind(segment_id)
        .bind(met as f32)
        .execute(pool)
        .await?;
    Ok(())
}

/// Update the weight on a user's open segment and recalculate calories.
pub async fn update_open_segment_weight(
    pool: &PgPool,
    user_id: uuid::Uuid,
    weight_kg: f32,
) -> anyhow::Result<()> {
    if let Some(seg) = get_open_segment(pool, user_id).await? {
        let met = met_for_speed_kmh(seg.speed_kmh);
        sqlx::query(&open_segment_update_sql("now()", "weight_kg = $3"))
            .bind(seg.id)
            .bind(met as f32)
            .bind(weight_kg)
            .execute(pool)
            .await?;
    }
    Ok(())
}

/// Close stale open segments whose last heartbeat is older than `threshold_secs`.
/// Used for both startup crash recovery (60s) and disconnect detection (5s).
pub async fn close_stale_segments(pool: &PgPool, threshold_secs: f64) -> anyhow::Result<u64> {
    let rows = sqlx::query(
        "SELECT id, speed_kmh FROM segments
         WHERE open = true
           AND EXTRACT(EPOCH FROM now() - last_heartbeat_at) > $1",
    )
    .bind(threshold_secs)
    .fetch_all(pool)
    .await?;

    let sql = open_segment_update_sql("last_heartbeat_at", "open = false");
    for row in &rows {
        let id: i64 = row.get("id");
        let speed: f32 = row.get("speed_kmh");
        let met = met_for_speed_kmh(speed as f64);
        if let Err(e) = sqlx::query(&sql)
            .bind(id)
            .bind(met as f32)
            .execute(pool)
            .await
        {
            tracing::error!(error = %e, segment_id = id, "Failed to close stale segment");
        }
    }
    Ok(rows.len() as u64)
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

/// Build a live snapshot of all users with open segments + today's totals.
pub async fn live_snapshot(pool: &PgPool) -> anyhow::Result<LiveBroadcast> {
    let rows = sqlx::query(
        "SELECT u.id, u.display_name, u.avatar_url,
                s.moving, s.speed_kmh,
                COALESCE(t.total_kcal, 0)::REAL AS calories_kcal,
                COALESCE(t.total_dist, 0)::REAL AS distance_m,
                COALESCE(t.total_secs, 0)::REAL AS active_secs
         FROM segments s
         JOIN users u ON u.id = s.user_id
         LEFT JOIN LATERAL (
             SELECT
                 SUM(calories_kcal)::REAL AS total_kcal,
                 SUM(distance_m)::REAL AS total_dist,
                 SUM(CASE WHEN open THEN EXTRACT(EPOCH FROM now() - started_at)
                          ELSE duration_s END)::REAL AS total_secs
             FROM segments
             WHERE user_id = s.user_id AND moving = true AND started_at::date = CURRENT_DATE
         ) t ON true
         WHERE s.open = true",
    )
    .fetch_all(pool)
    .await?;

    let users = rows
        .iter()
        .map(|r| {
            let moving: bool = r.get("moving");
            let status = if moving { "walking" } else { "idle" };
            LiveUser {
                id: r.get::<uuid::Uuid, _>("id"),
                name: r.get("display_name"),
                avatar_url: r.get("avatar_url"),
                status: status.to_string(),
                speed_kmh: r.get::<f32, _>("speed_kmh") as f64,
                calories_kcal: r.get::<f32, _>("calories_kcal") as f64,
                distance_m: r.get::<f32, _>("distance_m") as f64,
                active_secs: r.get::<f32, _>("active_secs") as u64,
            }
        })
        .collect();

    Ok(LiveBroadcast { users })
}

// -- Seed dev data --

/// MET value for a given walking speed (Compendium 2024, treadmill-specific).
pub fn met_for_speed_kmh(speed_kmh: f64) -> f64 {
    if speed_kmh < 1.6 {
        2.1
    } else if speed_kmh <= 3.0 {
        2.8
    } else if speed_kmh <= 3.9 {
        3.0
    } else if speed_kmh <= 4.7 {
        3.5
    } else if speed_kmh <= 5.5 {
        3.8
    } else if speed_kmh <= 6.3 {
        4.8
    } else if speed_kmh <= 7.1 {
        5.8
    } else if speed_kmh <= 7.9 {
        6.8
    } else {
        8.3
    }
}

/// Compute calories in kcal for a segment.
pub fn compute_calories_kcal(speed_kmh: f64, weight_kg: f64, duration_s: f64) -> f64 {
    let met = met_for_speed_kmh(speed_kmh);
    met * weight_kg * duration_s / 3600.0
}

/// Compute distance in meters for a segment.
pub fn compute_distance_m(speed_kmh: f64, duration_s: f64) -> f64 {
    speed_kmh * 1000.0 / 3600.0 * duration_s
}

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
            let calories_kcal = compute_calories_kcal(speed_kmh, weight_kg, duration_s);
            let distance_m = compute_distance_m(speed_kmh, duration_s);

            sqlx::query(
                "INSERT INTO segments (user_id, started_at, moving, speed_kmh, duration_s, open, weight_kg, calories_kcal, distance_m)
                 VALUES ($1,
                         (CURRENT_DATE - ($2 || ' days')::INTERVAL) + ($3 || ' seconds')::INTERVAL,
                         true, $4, $5, false, $6, $7, $8)",
            )
            .bind(user_id)
            .bind(days_ago)
            .bind(start_hour * 3600 + start_minute * 60 + offset_secs)
            .bind(speed_kmh as f32)
            .bind(duration_s as f32)
            .bind(weight_kg as f32)
            .bind(calories_kcal as f32)
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
                COALESCE(SUM(calories_kcal), 0)::REAL AS total_kcal,
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
            let dist: f32 = r.get("total_dist");
            let dur: f32 = r.get("total_dur");
            DailySnapshot {
                date: r.get("date"),
                calories_kcal: kcal as f64,
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
}

async fn query_leaderboard(
    pool: &PgPool,
    date_filter: &str,
) -> anyhow::Result<Vec<LeaderboardEntry>> {
    let sql = format!(
        "SELECT u.id, u.display_name AS name, u.avatar_url,
                COALESCE(SUM(s.calories_kcal), 0)::REAL AS total_kcal
         FROM users u LEFT JOIN segments s ON u.id = s.user_id AND s.moving = true {date_filter}
         GROUP BY u.id, u.display_name, u.avatar_url
         HAVING COALESCE(SUM(s.calories_kcal), 0) > 0
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
