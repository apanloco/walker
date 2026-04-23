use axum::{
    Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
};
use serde::Deserialize;
use sqlx::Row;

use super::{db, live::SharedLive};

pub fn routes() -> Router<SharedLive> {
    Router::new().route("/api/history/{id}", get(get_closed_segments))
}

#[derive(Deserialize)]
struct HistoryQuery {
    date: Option<String>,
}

/// Closed segments for a given date (defaults to today).
async fn get_closed_segments(
    State(ctx): State<SharedLive>,
    headers: axum::http::HeaderMap,
    Path(id_str): Path<String>,
    Query(query): Query<HistoryQuery>,
) -> impl IntoResponse {
    let pool = &ctx.db_pool;

    let Some(caller) = super::cookie_user_id(&headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    match db::get_user(pool, caller).await {
        Ok(Some(_)) => {}
        Ok(None) => return StatusCode::UNAUTHORIZED.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "get_user failed");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }

    let Ok(id) = uuid::Uuid::parse_str(&id_str) else {
        return axum::Json(serde_json::json!({"error": "invalid user id"})).into_response();
    };

    // History is private — only the user themselves can view it.
    if caller != id {
        return StatusCode::FORBIDDEN.into_response();
    }

    // Parse date or default to today. Validate format to prevent SQL injection.
    let date_filter = match &query.date {
        Some(d) if d.len() == 10 && d.chars().all(|c| c.is_ascii_digit() || c == '-') => d.clone(),
        _ => String::new(),
    };

    let rows = if date_filter.is_empty() {
        sqlx::query(
            "SELECT started_at::TEXT, moving, speed_kmh, incline_percent, duration_s, weight_kg,
                    active_calories(speed_kmh, incline_percent, weight_kg, duration_s) AS active_calories_kcal,
                    distance_m
             FROM segments
             WHERE user_id = $1 AND started_at::date = CURRENT_DATE AND open = false
             ORDER BY started_at ASC",
        )
        .bind(id)
        .fetch_all(pool.as_ref())
        .await
    } else {
        sqlx::query(
            "SELECT started_at::TEXT, moving, speed_kmh, incline_percent, duration_s, weight_kg,
                    active_calories(speed_kmh, incline_percent, weight_kg, duration_s) AS active_calories_kcal,
                    distance_m
             FROM segments
             WHERE user_id = $1 AND started_at::date = $2::date AND open = false
             ORDER BY started_at ASC",
        )
        .bind(id)
        .bind(&date_filter)
        .fetch_all(pool.as_ref())
        .await
    };

    match rows {
        Ok(rows) => {
            let segments: Vec<serde_json::Value> = rows.iter().map(segment_json).collect();
            axum::Json(serde_json::json!({ "segments": segments })).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "history closed segments query failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

fn segment_json(r: &sqlx::postgres::PgRow) -> serde_json::Value {
    serde_json::json!({
        "started_at": r.get::<String, _>("started_at"),
        "moving": r.get::<bool, _>("moving"),
        "speed_kmh": r.get::<f32, _>("speed_kmh"),
        "incline_percent": r.get::<Option<f32>, _>("incline_percent"),
        "duration_s": r.get::<f32, _>("duration_s"),
        "weight_kg": r.get::<f32, _>("weight_kg"),
        "active_calories_kcal": r.get::<f32, _>("active_calories_kcal"),
        "distance_m": r.get::<f32, _>("distance_m"),
        "open": false,
    })
}
