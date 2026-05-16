use axum::{
    Router,
    extract::{Query, State},
    response::IntoResponse,
    routing::get,
};
use serde::Deserialize;
use sqlx::Row;
use std::collections::BTreeMap;

use super::live::SharedLive;

pub fn routes() -> Router<SharedLive> {
    Router::new().route("/api/alltime", get(get_alltime))
}

#[derive(Deserialize)]
struct AlltimeParams {
    from: Option<String>, // YYYY-MM-DD inclusive lower bound
    to: Option<String>,   // YYYY-MM-DD inclusive upper bound
}

/// Daily walking totals per user. Public.
/// Optional ?from=YYYY-MM-DD&to=YYYY-MM-DD to restrict the date range.
async fn get_alltime(
    State(ctx): State<SharedLive>,
    Query(params): Query<AlltimeParams>,
) -> impl IntoResponse {
    let pool = &ctx.db_pool;

    let rows = match sqlx::query(
        "SELECT (s.started_at AT TIME ZONE 'UTC')::date::TEXT AS date,
                u.id, u.display_name AS name, u.avatar_url,
                SUM(active_calories(s.speed_kmh, s.incline_percent, s.weight_kg, s.duration_s))::FLOAT8 AS kcal
         FROM segments s
         JOIN users u ON u.id = s.user_id
         WHERE s.moving = true
           AND ($1::date IS NULL OR (s.started_at AT TIME ZONE 'UTC')::date >= $1::date)
           AND ($2::date IS NULL OR (s.started_at AT TIME ZONE 'UTC')::date <= $2::date)
         GROUP BY (s.started_at AT TIME ZONE 'UTC')::date, u.id, u.display_name, u.avatar_url
         ORDER BY date ASC",
    )
    .bind(params.from.as_deref())
    .bind(params.to.as_deref())
    .fetch_all(pool.as_ref())
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "alltime query failed");
            return axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let mut by_date: BTreeMap<String, Vec<serde_json::Value>> = BTreeMap::new();
    for r in &rows {
        let date: String = r.get("date");
        let entry = by_date.entry(date).or_default();
        entry.push(serde_json::json!({
            "id": r.get::<uuid::Uuid, _>("id"),
            "name": r.get::<String, _>("name"),
            "avatar_url": r.get::<Option<String>, _>("avatar_url"),
            "kcal": r.get::<f64, _>("kcal"),
        }));
    }

    let result: Vec<serde_json::Value> = by_date
        .into_iter()
        .map(|(date, users)| serde_json::json!({"date": date, "users": users}))
        .collect();

    axum::Json(result).into_response()
}
