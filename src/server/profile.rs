use axum::{
    Router,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
};

use super::db;
use super::live::SharedLive;

fn chrono_today() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let days = now / 86400;
    let (y, m, d) = days_to_ymd(days);
    format!("{y}-{m:02}-{d:02}")
}

fn days_to_ymd(days_since_epoch: u64) -> (u64, u64, u64) {
    // Adapted from Howard Hinnant's algorithm.
    let z = days_since_epoch + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

pub fn routes() -> Router<SharedLive> {
    Router::new().route("/api/profile/{id}", get(get_profile))
}

async fn get_profile(
    State(ctx): State<SharedLive>,
    headers: axum::http::HeaderMap,
    Path(id_str): Path<String>,
) -> impl IntoResponse {
    let pool = &ctx.db_pool;

    // Require login: caller must have a valid walker_id cookie.
    let Some(caller) = super::cookie_user_id(&headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let caller = match db::get_user(pool, caller).await {
        Ok(Some(u)) => u,
        Ok(None) => return StatusCode::UNAUTHORIZED.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "get_user failed");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let Ok(id) = uuid::Uuid::parse_str(&id_str) else {
        return axum::Json(serde_json::json!({"error": "invalid user id"})).into_response();
    };

    let user = match sqlx::query(
        "SELECT display_name, avatar_url, weight_kg, email, created_at::TEXT FROM users WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool.as_ref())
    .await
    {
        Ok(Some(row)) => row,
        Ok(None) => {
            return axum::Json(serde_json::json!({"error": "user not found"})).into_response();
        }
        Err(e) => {
            tracing::error!(error = %e, "profile user query failed");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let name: String = sqlx::Row::get(&user, "display_name");
    let avatar: Option<String> = sqlx::Row::get(&user, "avatar_url");
    let weight: f32 = sqlx::Row::get(&user, "weight_kg");
    let email: String = sqlx::Row::get(&user, "email");
    let member_since: String = sqlx::Row::get(&user, "created_at");
    let id_str = id.to_string();

    // Full year for heatmap + 30 days for stats.
    let year_history = match db::user_history(pool, id, 365).await {
        Ok(h) => h,
        Err(e) => {
            tracing::error!(error = %e, "user_history query failed");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let streak = match db::user_streak(pool, id).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "user_streak query failed");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // All-time totals (could be longer than 365 days).
    let all_time = match db::user_history(pool, id, 99999).await {
        Ok(h) => h,
        Err(e) => {
            tracing::error!(error = %e, "user_history all_time query failed");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let total_active_calories: f64 = all_time.iter().map(|d| d.active_calories_kcal).sum();
    let total_distance: f64 = all_time.iter().map(|d| d.distance_km).sum();
    let total_active: i32 = all_time.iter().map(|d| d.active_secs).sum();
    let total_days = all_time.len();

    // Records.
    let best_day_active_calories = all_time
        .iter()
        .map(|d| d.active_calories_kcal)
        .fold(0.0f64, f64::max);
    let best_day_distance = all_time
        .iter()
        .map(|d| d.distance_km)
        .fold(0.0f64, f64::max);
    let best_day_active = all_time.iter().map(|d| d.active_secs).max().unwrap_or(0);

    // Period calories for "you burned" section.
    let today_str = chrono_today();
    let today_active_cal: f64 = all_time
        .iter()
        .filter(|d| d.date == today_str)
        .map(|d| d.active_calories_kcal)
        .sum();
    let last_7: Vec<_> = year_history.iter().rev().take(7).rev().cloned().collect();
    let week_active_cal: f64 = last_7.iter().map(|d| d.active_calories_kcal).sum();
    let month_active_cal: f64 = year_history
        .iter()
        .rev()
        .take(30)
        .map(|d| d.active_calories_kcal)
        .sum();
    let year_active_cal: f64 = year_history.iter().map(|d| d.active_calories_kcal).sum();

    let strava_connected = db::strava_connected(pool, id).await.unwrap_or(false);

    // Check if currently walking via open segment in DB.
    let live_status = match db::get_open_segment(pool, id).await {
        Ok(Some(seg)) => {
            let status = if seg.moving { "walking" } else { "idle" };
            Some(serde_json::json!({"status": status, "speed_kmh": seg.speed_kmh}))
        }
        _ => None,
    };

    let mut resp = serde_json::json!({
        "id": id_str,
        "name": name,
        "strava_connected": strava_connected,
        "avatar_url": avatar,
        "weight_kg": weight,
        "member_since": member_since,
        "streak": streak,
        "live": live_status,
        "totals": {
            "active_calories_kcal": total_active_calories,
            "distance_km": total_distance,
            "active_secs": total_active,
            "active_days": total_days,
        },
        "periods": {
            "today_active_kcal": today_active_cal,
            "week_active_kcal": week_active_cal,
            "month_active_kcal": month_active_cal,
            "year_active_kcal": year_active_cal,
            "all_time_active_kcal": total_active_calories,
        },
        "records": {
            "best_day_active_calories_kcal": best_day_active_calories,
            "best_day_distance_km": best_day_distance,
            "best_day_active_secs": best_day_active,
        },
        "last_7_days": last_7,
        "heatmap": year_history,
    });
    if caller.is_admin {
        resp["email"] = serde_json::json!(email);
    }
    axum::Json(resp).into_response()
}
