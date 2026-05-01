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
        .route("/api/history/{id}", get(get_closed_segments))
        .route("/api/activity/raw/{activity_id}", get(get_raw_activity))
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
        return (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({"error": "invalid user id"})),
        )
            .into_response();
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
            "SELECT s.started_at::TEXT, s.moving, s.speed_kmh, s.incline_percent, s.duration_s, s.weight_kg,
                    active_calories(s.speed_kmh, s.incline_percent, s.weight_kg, s.duration_s) AS active_calories_kcal,
                    s.distance_m, s.source, s.activity_id,
                    ia.name AS activity_name, ia.source_url
             FROM segments s
             LEFT JOIN imported_activities ia ON ia.id = s.activity_id
             WHERE s.user_id = $1 AND s.started_at::date = CURRENT_DATE AND s.open = false
             ORDER BY s.started_at ASC",
        )
        .bind(id)
        .fetch_all(pool.as_ref())
        .await
    } else {
        sqlx::query(
            "SELECT s.started_at::TEXT, s.moving, s.speed_kmh, s.incline_percent, s.duration_s, s.weight_kg,
                    active_calories(s.speed_kmh, s.incline_percent, s.weight_kg, s.duration_s) AS active_calories_kcal,
                    s.distance_m, s.source, s.activity_id,
                    ia.name AS activity_name, ia.source_url
             FROM segments s
             LEFT JOIN imported_activities ia ON ia.id = s.activity_id
             WHERE s.user_id = $1 AND s.started_at::date = $2::date AND s.open = false
             ORDER BY s.started_at ASC",
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
            tracing::error!(error = %e, "history closed segments query failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// Raw JSON for an imported activity. Own-only: verifies the activity belongs to the caller.
async fn get_raw_activity(
    State(ctx): State<SharedLive>,
    headers: axum::http::HeaderMap,
    Path(activity_id): Path<i64>,
) -> impl IntoResponse {
    let pool = &ctx.db_pool;

    let Some(caller) = super::cookie_user_id(&headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };

    // Ownership check: the activity must belong to the caller via a segment.
    let row = sqlx::query(
        "SELECT ia.raw_data
         FROM imported_activities ia
         JOIN segments s ON s.activity_id = ia.id
         WHERE ia.id = $1 AND s.user_id = $2
         LIMIT 1",
    )
    .bind(activity_id)
    .bind(caller)
    .fetch_optional(pool.as_ref())
    .await;

    match row {
        Ok(Some(r)) => {
            let raw: serde_json::Value = r.get("raw_data");
            let body = serde_json::to_string_pretty(&raw).unwrap_or_default();
            axum::response::Response::builder()
                .header("Content-Type", "application/json")
                .body(axum::body::Body::from(body))
                .unwrap()
                .into_response()
        }
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "raw activity query failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

fn segment_json(r: &sqlx::postgres::PgRow, open: bool) -> serde_json::Value {
    serde_json::json!({
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
        "open": open,
    })
}
