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

    // Look up email from public ID.
    let user =
        sqlx::query("SELECT email, display_name, avatar_url, weight_kg FROM users WHERE id = $1")
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

    let history = db::user_history(pool, &email, 30).await.unwrap_or_default();
    let streak = db::user_streak(pool, &email).await.unwrap_or(0);

    let total_calories: f64 = history.iter().map(|d| d.calories_kcal).sum();
    let total_distance: f64 = history.iter().map(|d| d.distance_km).sum();
    let total_active: i32 = history.iter().map(|d| d.active_secs).sum();

    Json(serde_json::json!({
        "id": id,
        "name": name,
        "avatar_url": avatar,
        "weight_kg": weight,
        "streak": streak,
        "last_30_days": {
            "total_calories_kcal": total_calories,
            "total_distance_km": total_distance,
            "total_active_secs": total_active,
            "days": history,
        }
    }))
}
