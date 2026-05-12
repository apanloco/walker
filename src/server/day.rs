use axum::{
    Router,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
};
use sqlx::Row;
use std::collections::BTreeMap;

use super::live::SharedLive;

pub fn routes() -> Router<SharedLive> {
    Router::new().route("/api/day/{date}", get(get_day))
}

/// All users with walking segments for a given UTC date. Public.
/// Idle segments are omitted — they contribute 0 kcal.
async fn get_day(State(ctx): State<SharedLive>, Path(date_str): Path<String>) -> impl IntoResponse {
    let pool = &ctx.db_pool;

    // Validate date format (YYYY-MM-DD).
    if date_str.len() != 10 || !date_str.chars().all(|c| c.is_ascii_digit() || c == '-') {
        return (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({"error": "invalid date"})),
        )
            .into_response();
    }

    let rows = match sqlx::query(
        "SELECT u.id, u.display_name AS name, u.avatar_url,
                s.started_at::TEXT AS started_at, s.duration_s, s.open,
                active_calories(s.speed_kmh, s.incline_percent, s.weight_kg, s.duration_s) AS active_calories_kcal
         FROM users u
         JOIN segments s ON s.user_id = u.id
         WHERE s.started_at::date = $1::date AND s.moving = true
         ORDER BY u.id, s.started_at ASC",
    )
    .bind(&date_str)
    .fetch_all(pool.as_ref())
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "day query failed");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // Group rows by user. BTreeMap iteration order is sorted by user id (matches the
    // SQL ORDER BY u.id), so the legend on the chart is deterministic across reloads.
    let mut users: BTreeMap<uuid::Uuid, (String, Option<String>, Vec<serde_json::Value>)> =
        BTreeMap::new();
    for r in &rows {
        let id: uuid::Uuid = r.get("id");
        let name: String = r.get("name");
        let avatar: Option<String> = r.get("avatar_url");
        let entry = users.entry(id).or_insert((name, avatar, Vec::new()));
        entry.2.push(serde_json::json!({
            "started_at": r.get::<String, _>("started_at"),
            "duration_s": r.get::<f32, _>("duration_s"),
            "open": r.get::<bool, _>("open"),
            "active_calories_kcal": r.get::<f32, _>("active_calories_kcal"),
        }));
    }

    let users_json: Vec<serde_json::Value> = users
        .into_iter()
        .map(|(id, (name, avatar_url, segments))| {
            serde_json::json!({
                "id": id,
                "name": name,
                "avatar_url": avatar_url,
                "segments": segments,
            })
        })
        .collect();

    axum::Json(serde_json::json!({
        "date": date_str,
        "users": users_json,
    }))
    .into_response()
}
