use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{post, put},
};
use serde::Deserialize;
use tracing::{error, warn};

use super::db;
use super::live::{self, SharedLive};

pub fn routes() -> Router<SharedLive> {
    Router::new()
        .route("/api/update", post(handle_update))
        .route("/api/weight", put(handle_set_weight))
}

#[derive(Deserialize)]
struct UpdatePayload {
    state: String, // "walking", "idle", "stopped"
    speed_kmh: f64,
}

async fn handle_update(
    State(ctx): State<SharedLive>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<UpdatePayload>,
) -> impl IntoResponse {
    // Authenticate.
    let Some(token) = extract_bearer_token(&headers) else {
        return StatusCode::UNAUTHORIZED;
    };
    let pool = &ctx.db_pool;
    let Ok(Some(user)) = db::lookup_token(pool, token).await else {
        return StatusCode::UNAUTHORIZED;
    };

    let moving = payload.state == "walking";
    let stopped = payload.state == "stopped";

    // Query DB for current open segment.
    let open_seg = db::get_open_segment(pool, user.id).await.unwrap_or(None);

    // Determine if state changed.
    let state_changed = match &open_seg {
        Some(seg) => {
            seg.moving != moving
                || (moving && (seg.speed_kmh - payload.speed_kmh).abs() > 0.05)
                || stopped
        }
        None => !stopped, // No open segment and not stopped → need to open one
    };

    if state_changed {
        // Close current segment if any.
        if let Some(seg) = &open_seg {
            let met = db::met_for_speed_kmh(seg.speed_kmh);
            if let Err(e) = db::close_segment(pool, seg.id, met).await {
                error!(error = %e, segment_id = seg.id, "Failed to close segment");
            }
        }

        // Open new segment unless stopped.
        if !stopped {
            let weight = db::get_user_weight(pool, user.id).await.unwrap_or(70.0);
            if let Err(e) = db::open_segment(pool, user.id, moving, payload.speed_kmh, weight).await
            {
                warn!(error = %e, "Failed to open segment");
            }
        }

        // Notify all viewers (leaderboard refresh, closed segments refresh).
        let _ = ctx.broadcast_tx.send(());

        // Push live segment to per-user subscribers.
        live::push_user_segment(&ctx, user.id).await;
    } else if let Some(seg) = &open_seg {
        // Heartbeat: update segment duration + last_heartbeat_at.
        let met = db::met_for_speed_kmh(seg.speed_kmh);
        if let Err(e) = db::update_open_segment(pool, seg.id, met).await {
            error!(error = %e, segment_id = seg.id, "Failed to update segment");
        }

        // Push updated segment to per-user subscribers.
        live::push_user_segment(&ctx, user.id).await;
    }

    StatusCode::OK
}

// -- PUT /api/weight --

#[derive(Deserialize)]
struct SetWeightPayload {
    weight_kg: f32,
}

async fn handle_set_weight(
    State(ctx): State<SharedLive>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<SetWeightPayload>,
) -> impl IntoResponse {
    let Some(token) = extract_bearer_token(&headers) else {
        return StatusCode::UNAUTHORIZED;
    };
    let pool = &ctx.db_pool;
    let Ok(Some(user)) = db::lookup_token(pool, token).await else {
        return StatusCode::UNAUTHORIZED;
    };

    if let Err(e) = db::set_user_weight(pool, user.id, payload.weight_kg).await {
        error!(error = %e, "Failed to set weight");
        return StatusCode::INTERNAL_SERVER_ERROR;
    }

    // Update weight on open segment if any.
    if let Err(e) = db::update_open_segment_weight(pool, user.id, payload.weight_kg).await {
        error!(error = %e, "Failed to update open segment weight");
    }

    // Push updated segment to per-user subscribers.
    live::push_user_segment(&ctx, user.id).await;

    StatusCode::OK
}

fn extract_bearer_token(headers: &axum::http::HeaderMap) -> Option<&str> {
    headers
        .get("authorization")?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}
