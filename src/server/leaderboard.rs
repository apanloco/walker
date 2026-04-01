use axum::{Json, Router, extract::State, routing::get};
use tracing::error;

use super::db;
use super::live::SharedLive;

pub fn routes() -> Router<SharedLive> {
    Router::new().route("/api/leaderboard", get(get_leaderboard))
}

async fn get_leaderboard(State(ctx): State<SharedLive>) -> Json<serde_json::Value> {
    let pool = &ctx.db_pool;

    let today = db::leaderboard_today(pool).await.unwrap_or_else(|e| {
        error!(error = %e, "leaderboard_today query failed");
        vec![]
    });
    let weekly = db::leaderboard_weekly(pool).await.unwrap_or_else(|e| {
        error!(error = %e, "leaderboard_weekly query failed");
        vec![]
    });
    let all_time = db::leaderboard_all_time(pool).await.unwrap_or_else(|e| {
        error!(error = %e, "leaderboard_all_time query failed");
        vec![]
    });

    // Merge live status from open segments in DB.
    let live_statuses = db::get_live_statuses(pool).await.unwrap_or_default();

    let enrich = |mut entries: Vec<db::LeaderboardEntry>| -> Vec<serde_json::Value> {
        entries
            .drain(..)
            .map(|e| {
                let (status, speed) = live_statuses
                    .get(&e.id)
                    .map(|(moving, spd)| {
                        let s = if *moving { "walking" } else { "idle" };
                        (s.to_string(), *spd)
                    })
                    .unwrap_or(("offline".to_string(), 0.0));
                serde_json::json!({
                    "id": e.id,
                    "name": e.name,
                    "avatar_url": e.avatar_url,
                    "calories_kcal": e.calories_kcal,
                    "status": status,
                    "speed_kmh": speed,
                })
            })
            .collect()
    };

    Json(serde_json::json!({
        "today": enrich(today),
        "weekly": enrich(weekly),
        "all_time": enrich(all_time),
    }))
}
