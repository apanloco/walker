use axum::{Router, extract::State, http::StatusCode, response::IntoResponse, routing::get};
use tracing::error;

use super::db;
use super::live::SharedLive;

pub fn routes() -> Router<SharedLive> {
    Router::new().route("/api/leaderboard", get(get_leaderboard))
}

async fn get_leaderboard(State(ctx): State<SharedLive>) -> impl IntoResponse {
    let pool = &ctx.db_pool;

    let today = match db::leaderboard_today(pool).await {
        Ok(v) => v,
        Err(e) => {
            error!(error = %e, "leaderboard_today query failed");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let weekly = match db::leaderboard_weekly(pool).await {
        Ok(v) => v,
        Err(e) => {
            error!(error = %e, "leaderboard_weekly query failed");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let all_time = match db::leaderboard_all_time(pool).await {
        Ok(v) => v,
        Err(e) => {
            error!(error = %e, "leaderboard_all_time query failed");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let daily_winners = match db::leaderboard_daily_winners(pool).await {
        Ok(v) => v,
        Err(e) => {
            error!(error = %e, "leaderboard_daily_winners query failed");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // Merge live status from open segments in DB.
    let live_statuses = match db::get_live_statuses(pool).await {
        Ok(v) => v,
        Err(e) => {
            error!(error = %e, "get_live_statuses query failed");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

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
                    "active_calories_kcal": e.active_calories_kcal,
                    "status": status,
                    "speed_kmh": speed,
                })
            })
            .collect()
    };

    let daily_winners_json: Vec<serde_json::Value> = daily_winners
        .into_iter()
        .map(|w| {
            let (status, speed) = live_statuses
                .get(&w.id)
                .map(|(moving, spd)| {
                    let s = if *moving { "walking" } else { "idle" };
                    (s.to_string(), *spd)
                })
                .unwrap_or(("offline".to_string(), 0.0));
            serde_json::json!({
                "date": w.date,
                "id": w.id,
                "name": w.name,
                "avatar_url": w.avatar_url,
                "active_calories_kcal": w.active_calories_kcal,
                "status": status,
                "speed_kmh": speed,
            })
        })
        .collect();

    axum::Json(serde_json::json!({
        "today": enrich(today),
        "weekly": enrich(weekly),
        "all_time": enrich(all_time),
        "daily_winners": daily_winners_json,
    }))
    .into_response()
}
