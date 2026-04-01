use axum::{
    Json, Router,
    extract::{Path, State},
    routing::get,
};
use sqlx::Row;

use super::live::SharedLive;

pub fn routes() -> Router<SharedLive> {
    Router::new()
        .route("/api/activity/{id}", get(get_closed_segments))
        .route("/api/activity/{id}/current", get(get_current_segment))
}

/// Closed segments for today — historical record, doesn't change between state changes.
async fn get_closed_segments(
    State(ctx): State<SharedLive>,
    Path(id_str): Path<String>,
) -> Json<serde_json::Value> {
    let pool = &ctx.db_pool;

    let Ok(id) = uuid::Uuid::parse_str(&id_str) else {
        return Json(serde_json::json!({"error": "invalid user id"}));
    };

    let rows = sqlx::query(
        "SELECT started_at::TEXT, moving, speed_kmh, duration_s, weight_kg,
                calories_kcal, distance_m
         FROM segments
         WHERE user_id = $1 AND started_at::date = CURRENT_DATE AND open = false
         ORDER BY started_at ASC",
    )
    .bind(id)
    .fetch_all(pool.as_ref())
    .await;

    let segments: Vec<serde_json::Value> = match rows {
        Ok(rows) => rows.iter().map(|r| segment_json(r, false)).collect(),
        Err(e) => {
            tracing::error!(error = %e, "activity closed segments query failed");
            vec![]
        }
    };

    Json(serde_json::json!({ "segments": segments }))
}

/// The one open segment — live data, polled by the client on its own schedule.
async fn get_current_segment(
    State(ctx): State<SharedLive>,
    Path(id_str): Path<String>,
) -> Json<serde_json::Value> {
    let pool = &ctx.db_pool;

    let Ok(id) = uuid::Uuid::parse_str(&id_str) else {
        return Json(serde_json::json!({"error": "invalid user id"}));
    };

    let row = sqlx::query(
        "SELECT started_at::TEXT, moving, speed_kmh, duration_s, weight_kg,
                calories_kcal, distance_m
         FROM segments
         WHERE user_id = $1 AND open = true",
    )
    .bind(id)
    .fetch_optional(pool.as_ref())
    .await;

    match row {
        Ok(Some(r)) => Json(serde_json::json!({ "segment": segment_json(&r, true) })),
        Ok(None) => Json(serde_json::json!({ "segment": null })),
        Err(e) => {
            tracing::error!(error = %e, "activity current segment query failed");
            Json(serde_json::json!({ "segment": null }))
        }
    }
}

fn segment_json(r: &sqlx::postgres::PgRow, open: bool) -> serde_json::Value {
    serde_json::json!({
        "started_at": r.get::<String, _>("started_at"),
        "moving": r.get::<bool, _>("moving"),
        "speed_kmh": r.get::<f32, _>("speed_kmh"),
        "duration_s": r.get::<f32, _>("duration_s"),
        "weight_kg": r.get::<f32, _>("weight_kg"),
        "calories_kcal": r.get::<f32, _>("calories_kcal"),
        "distance_m": r.get::<f32, _>("distance_m"),
        "open": open,
    })
}
