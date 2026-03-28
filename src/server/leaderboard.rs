use axum::{Json, Router, extract::State, routing::get};

use super::db;
use super::live::SharedLive;

pub fn routes() -> Router<SharedLive> {
    Router::new().route("/api/leaderboard", get(get_leaderboard))
}

async fn get_leaderboard(State(ctx): State<SharedLive>) -> Json<serde_json::Value> {
    let Some(ref pool) = ctx.db_pool else {
        return Json(serde_json::json!({"today": [], "weekly": [], "all_time": []}));
    };

    let today = db::leaderboard_today(pool).await.unwrap_or_default();
    let weekly = db::leaderboard_weekly(pool).await.unwrap_or_default();
    let all_time = db::leaderboard_all_time(pool).await.unwrap_or_default();

    // Merge live status from in-memory state.
    let state = ctx.state.read().await;
    let enrich = |mut entries: Vec<db::LeaderboardEntry>| -> Vec<serde_json::Value> {
        entries
            .drain(..)
            .map(|e| {
                // Find live user by matching ID.
                let (status, speed) = state
                    .users
                    .values()
                    .find(|u| u.id == e.id)
                    .map(|u| (u.status().to_string(), u.speed_mph))
                    .unwrap_or(("offline".to_string(), 0.0));
                serde_json::json!({
                    "id": e.id,
                    "name": e.name,
                    "avatar_url": e.avatar_url,
                    "calories_kcal": e.calories_kcal,
                    "status": status,
                    "speed_mph": speed,
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
