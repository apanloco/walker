use sqlx::{PgPool, Row};
use tracing::info;

use super::live::TokenUser;

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
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO users (email, id, display_name, avatar_url)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (email) DO UPDATE SET display_name = $3, avatar_url = $4",
    )
    .bind(email)
    .bind(id)
    .bind(display_name)
    .bind(avatar_url)
    .execute(pool)
    .await?;
    Ok(())
}

/// Get a user's public ID by email.
pub async fn get_user_id(pool: &PgPool, email: &str) -> anyhow::Result<String> {
    let row = sqlx::query("SELECT id FROM users WHERE email = $1")
        .bind(email)
        .fetch_one(pool)
        .await?;
    Ok(row.get("id"))
}

/// Store a token in the DB.
pub async fn store_token(pool: &PgPool, token: &str, email: &str) -> anyhow::Result<()> {
    sqlx::query("INSERT INTO tokens (token, user_email) VALUES ($1, $2)")
        .bind(token)
        .bind(email)
        .execute(pool)
        .await?;
    Ok(())
}

/// Load all tokens from DB into the in-memory token map.
pub async fn load_tokens(pool: &PgPool) -> anyhow::Result<Vec<(String, TokenUser)>> {
    let rows = sqlx::query(
        "SELECT t.token, u.id, u.email, u.display_name, u.avatar_url
         FROM tokens t JOIN users u ON t.user_email = u.email",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .iter()
        .map(|r| {
            (
                r.get::<String, _>("token"),
                TokenUser {
                    id: r.get("id"),
                    email: r.get("email"),
                    display_name: r.get("display_name"),
                    avatar_url: r.get("avatar_url"),
                },
            )
        })
        .collect())
}

/// Accumulate a delta into the daily stats for a user.
/// One row per user per day. Each call ADDs to the totals (not overwrites).
/// At midnight, CURRENT_DATE flips and a new row is created automatically.
pub async fn accumulate_daily_stats(
    pool: &PgPool,
    email: &str,
    calories_ucal: u64,
    active_secs: u64,
    idle_secs: u64,
    distance_m: f64,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO daily_stats (user_email, date, calories_ucal, active_secs, idle_secs, distance_m, updated_at)
         VALUES ($1, CURRENT_DATE, $2, $3, $4, $5, NOW())
         ON CONFLICT (user_email, date) DO UPDATE SET
           calories_ucal = daily_stats.calories_ucal + $2,
           active_secs = daily_stats.active_secs + $3,
           idle_secs = daily_stats.idle_secs + $4,
           distance_m = daily_stats.distance_m + $5,
           updated_at = NOW()",
    )
    .bind(email)
    .bind(calories_ucal as i64)
    .bind(active_secs as i32)
    .bind(idle_secs as i32)
    .bind(distance_m as f32)
    .execute(pool)
    .await?;
    Ok(())
}

/// Profile data: last 30 days of daily stats for a user.
#[derive(serde::Serialize)]
pub struct DailySnapshot {
    pub date: String,
    pub calories_kcal: f64,
    pub distance_km: f64,
    pub active_secs: i32,
}

pub async fn user_history(
    pool: &PgPool,
    email: &str,
    days: i32,
) -> anyhow::Result<Vec<DailySnapshot>> {
    let rows = sqlx::query(
        "SELECT date::TEXT, calories_ucal, distance_m, active_secs FROM daily_stats
         WHERE user_email = $1 AND date >= CURRENT_DATE - ($2 || ' days')::INTERVAL
         ORDER BY date ASC",
    )
    .bind(email)
    .bind(days)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .iter()
        .map(|r| {
            let ucal: i64 = r.get("calories_ucal");
            let dist: f32 = r.get("distance_m");
            DailySnapshot {
                date: r.get("date"),
                calories_kcal: ucal as f64 / 1_000_000.0,
                distance_km: dist as f64 / 1000.0,
                active_secs: r.get("active_secs"),
            }
        })
        .collect())
}

/// Count consecutive days with activity ending today (streak).
pub async fn user_streak(pool: &PgPool, email: &str) -> anyhow::Result<i32> {
    let row = sqlx::query(
        "WITH dates AS (
           SELECT date, ROW_NUMBER() OVER (ORDER BY date DESC) AS rn
           FROM daily_stats
           WHERE user_email = $1 AND active_secs > 0
         )
         SELECT COUNT(*) AS streak FROM dates
         WHERE date = CURRENT_DATE - (rn - 1)::INT",
    )
    .bind(email)
    .fetch_one(pool)
    .await?;

    Ok(row.get::<i64, _>("streak") as i32)
}

#[derive(serde::Serialize)]
pub struct LeaderboardEntry {
    pub id: String,
    pub name: String,
    pub avatar_url: Option<String>,
    pub calories_kcal: f64,
}

/// Load today's accumulated stats for a user (to seed in-memory state on first connect).
pub async fn load_daily_stats(
    pool: &PgPool,
    email: &str,
) -> anyhow::Result<Option<(i64, i32, i32)>> {
    let row = sqlx::query(
        "SELECT calories_ucal, active_secs, idle_secs FROM daily_stats
         WHERE user_email = $1 AND date = CURRENT_DATE",
    )
    .bind(email)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| {
        (
            r.get("calories_ucal"),
            r.get("active_secs"),
            r.get("idle_secs"),
        )
    }))
}

async fn query_leaderboard(
    pool: &PgPool,
    date_filter: &str,
) -> anyhow::Result<Vec<LeaderboardEntry>> {
    let sql = format!(
        "SELECT u.id, u.display_name AS name, u.avatar_url, COALESCE(SUM(s.calories_ucal), 0)::BIGINT AS total_ucal
         FROM users u LEFT JOIN daily_stats s ON u.email = s.user_email {date_filter}
         GROUP BY u.id, u.display_name, u.avatar_url
         HAVING COALESCE(SUM(s.calories_ucal), 0) > 0
         ORDER BY total_ucal DESC LIMIT 10"
    );

    let rows = sqlx::query(&sql).fetch_all(pool).await?;

    Ok(rows
        .iter()
        .map(|r| LeaderboardEntry {
            id: r.get("id"),
            name: r.get("name"),
            avatar_url: r.get("avatar_url"),
            calories_kcal: r.get::<i64, _>("total_ucal") as f64 / 1_000_000.0,
        })
        .collect())
}

pub async fn leaderboard_today(pool: &PgPool) -> anyhow::Result<Vec<LeaderboardEntry>> {
    query_leaderboard(pool, "AND s.date = CURRENT_DATE").await
}

pub async fn leaderboard_weekly(pool: &PgPool) -> anyhow::Result<Vec<LeaderboardEntry>> {
    query_leaderboard(pool, "AND s.date >= CURRENT_DATE - INTERVAL '7 days'").await
}

pub async fn leaderboard_all_time(pool: &PgPool) -> anyhow::Result<Vec<LeaderboardEntry>> {
    query_leaderboard(pool, "").await
}
