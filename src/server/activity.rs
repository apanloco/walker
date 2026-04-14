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
    Router::new()
        .route("/api/activity/{id}", get(get_closed_segments))
        .route("/api/activity/{id}/current", get(get_current_segment))
}

#[derive(Deserialize)]
struct ActivityQuery {
    date: Option<String>,
}

/// Closed segments for a given date (defaults to today).
async fn get_closed_segments(
    State(ctx): State<SharedLive>,
    headers: axum::http::HeaderMap,
    Path(id_str): Path<String>,
    Query(query): Query<ActivityQuery>,
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

    // Activity is private — only the user themselves can view it.
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
            "SELECT started_at::TEXT, moving, speed_kmh, duration_s, weight_kg,
                    total_calories(speed_kmh, weight_kg, duration_s) AS calories_kcal,
                    active_calories(speed_kmh, weight_kg, duration_s) AS active_calories_kcal,
                    met_for_speed(speed_kmh) AS met,
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
            "SELECT started_at::TEXT, moving, speed_kmh, duration_s, weight_kg,
                    total_calories(speed_kmh, weight_kg, duration_s) AS calories_kcal,
                    active_calories(speed_kmh, weight_kg, duration_s) AS active_calories_kcal,
                    met_for_speed(speed_kmh) AS met,
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
            let segments: Vec<serde_json::Value> =
                rows.iter().map(|r| segment_json(r, false)).collect();
            axum::Json(serde_json::json!({ "segments": segments })).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "activity closed segments query failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// The one open segment — live data, polled by the client on its own schedule.
async fn get_current_segment(
    State(ctx): State<SharedLive>,
    headers: axum::http::HeaderMap,
    Path(id_str): Path<String>,
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

    if caller != id {
        return StatusCode::FORBIDDEN.into_response();
    }

    let row = sqlx::query(
        "SELECT started_at::TEXT, moving, speed_kmh, duration_s, weight_kg,
                total_calories(speed_kmh, weight_kg, duration_s) AS calories_kcal,
                active_calories(speed_kmh, weight_kg, duration_s) AS active_calories_kcal,
                met_for_speed(speed_kmh) AS met,
                distance_m
         FROM segments
         WHERE user_id = $1 AND open = true",
    )
    .bind(id)
    .fetch_optional(pool.as_ref())
    .await;

    match row {
        Ok(Some(r)) => {
            axum::Json(serde_json::json!({ "segment": segment_json(&r, true) })).into_response()
        }
        Ok(None) => axum::Json(serde_json::json!({ "segment": null })).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "activity current segment query failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
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
        "active_calories_kcal": r.get::<f32, _>("active_calories_kcal"),
        "met": r.get::<f32, _>("met"),
        "distance_m": r.get::<f32, _>("distance_m"),
        "open": open,
    })
}
