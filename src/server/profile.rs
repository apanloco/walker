use axum::{
    Json, Router,
    extract::{Path, State},
    routing::get,
};

use super::db;
use super::live::SharedLive;

pub fn routes() -> Router<SharedLive> {
    Router::new().route("/api/profile/{id}", get(get_profile))
}

async fn get_profile(
    State(ctx): State<SharedLive>,
    Path(id): Path<String>,
) -> Json<serde_json::Value> {
    let Some(ref pool) = ctx.db_pool else {
        return Json(serde_json::json!({"error": "no database"}));
    };

    let user =
        sqlx::query("SELECT email, display_name, avatar_url, weight_kg, created_at::TEXT FROM users WHERE id = $1")
            .bind(&id)
            .fetch_optional(pool.as_ref())
            .await
            .ok()
            .flatten();

    let Some(user) = user else {
        return Json(serde_json::json!({"error": "user not found"}));
    };

    let email: String = sqlx::Row::get(&user, "email");
    let name: String = sqlx::Row::get(&user, "display_name");
    let avatar: Option<String> = sqlx::Row::get(&user, "avatar_url");
    let weight: f32 = sqlx::Row::get(&user, "weight_kg");
    let member_since: String = sqlx::Row::get(&user, "created_at");

    // Full year for heatmap + 30 days for stats.
    let year_history = db::user_history(pool, &email, 365)
        .await
        .unwrap_or_default();
    let streak = db::user_streak(pool, &email).await.unwrap_or(0);

    // Compute totals and records.
    let total_calories: f64 = year_history.iter().map(|d| d.calories_kcal).sum();
    let total_distance: f64 = year_history.iter().map(|d| d.distance_km).sum();
    let total_active: i32 = year_history.iter().map(|d| d.active_secs).sum();
    let total_days = year_history.len();
    let best_day_calories = year_history
        .iter()
        .map(|d| d.calories_kcal)
        .fold(0.0f64, f64::max);
    let best_day_distance = year_history
        .iter()
        .map(|d| d.distance_km)
        .fold(0.0f64, f64::max);
    let best_day_active = year_history
        .iter()
        .map(|d| d.active_secs)
        .max()
        .unwrap_or(0);

    // Last 7 days for the weekly breakdown.
    let last_7: Vec<_> = year_history.iter().rev().take(7).rev().cloned().collect();

    // Check if currently walking.
    let live_status = {
        let state = ctx.state.read().await;
        state
            .users
            .values()
            .find(|u| u.id == id)
            .map(|u| serde_json::json!({"status": u.status(), "speed_mph": u.speed_mph}))
    };

    Json(serde_json::json!({
        "id": id,
        "name": name,
        "avatar_url": avatar,
        "weight_kg": weight,
        "member_since": member_since,
        "streak": streak,
        "live": live_status,
        "totals": {
            "calories_kcal": total_calories,
            "distance_km": total_distance,
            "active_secs": total_active,
            "active_days": total_days,
        },
        "records": {
            "best_day_calories_kcal": best_day_calories,
            "best_day_distance_km": best_day_distance,
            "best_day_active_secs": best_day_active,
        },
        "last_7_days": last_7,
        "heatmap": year_history,
    }))
}
