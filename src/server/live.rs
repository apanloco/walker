use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, broadcast};
use tracing::{error, info, warn};

use super::db;

#[derive(Clone)]
#[allow(dead_code)]
pub struct TokenUser {
    pub id: uuid::Uuid,
    pub email: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
}

pub struct LiveContext {
    /// Notification-only broadcast: fires on state changes + disconnect checks.
    /// Carries no data — dashboard refetches via REST on receipt.
    pub broadcast_tx: broadcast::Sender<()>,
    /// Per-user channels: push open segment data to `/ws/live/{id}` subscribers.
    pub user_txs: RwLock<HashMap<uuid::Uuid, broadcast::Sender<String>>>,
    pub db_pool: Arc<sqlx::PgPool>,
    pub dev_mode: bool,
}

pub type SharedLive = Arc<LiveContext>;

/// Push the current open segment JSON to any subscribers watching this user.
pub async fn push_user_segment(ctx: &LiveContext, user_id: uuid::Uuid) {
    let txs = ctx.user_txs.read().await;
    let Some(tx) = txs.get(&user_id) else {
        return;
    };
    if tx.receiver_count() == 0 {
        return;
    }
    match db::get_current_segment_json(&ctx.db_pool, user_id).await {
        Ok(json) => {
            let _ = tx.send(json);
        }
        Err(e) => error!(error = %e, "Failed to query segment for push"),
    }
}

pub fn routes() -> Router<SharedLive> {
    Router::new()
        .route("/api/simulate/register", post(register_simulated_user))
        .route("/ws/live", get(ws_live))
        .route("/ws/live/{id}", get(ws_live_user))
}

// -- POST /api/simulate/register --

#[derive(Deserialize)]
struct SimulateRegisterPayload {
    name: String,
    email: String,
}

#[derive(serde::Serialize)]
struct SimulateRegisterResponse {
    token: String,
}

async fn register_simulated_user(
    State(ctx): State<SharedLive>,
    Json(payload): Json<SimulateRegisterPayload>,
) -> impl IntoResponse {
    if !ctx.dev_mode {
        return (
            StatusCode::NOT_FOUND,
            Json(SimulateRegisterResponse {
                token: String::new(),
            }),
        );
    }
    let token = format!("sim-{}", payload.email);

    let pool = &ctx.db_pool;
    if let Err(e) = db::upsert_user(pool, &payload.email, &payload.name, None).await {
        error!(error = %e, "Failed to upsert simulated user");
    }
    let id = db::get_user_id(pool, &payload.email)
        .await
        .unwrap_or_else(|_| uuid::Uuid::new_v4());
    if let Err(e) = db::store_token(pool, &token, id).await {
        error!(error = %e, "Failed to store simulated user token");
    }

    (StatusCode::OK, Json(SimulateRegisterResponse { token }))
}

// -- WebSocket /ws/live (notification-only) --

async fn ws_live(State(ctx): State<SharedLive>, ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws_live(socket, ctx))
}

async fn handle_ws_live(mut socket: WebSocket, ctx: SharedLive) {
    let mut rx = ctx.broadcast_tx.subscribe();
    info!("Live viewer connected");

    // Send initial notification so dashboard fetches immediately.
    let _ = socket.send(Message::Text("update".into())).await;

    loop {
        match rx.recv().await {
            Ok(()) => {
                if socket.send(Message::Text("update".into())).await.is_err() {
                    break;
                }
            }
            Err(broadcast::error::RecvError::Lagged(n)) => {
                warn!(skipped = n, "Live viewer lagging, skipped messages");
            }
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }

    info!("Live viewer disconnected");
}

// -- WebSocket /ws/live/{id} (per-user segment push) --

async fn ws_live_user(
    State(ctx): State<SharedLive>,
    headers: axum::http::HeaderMap,
    Path(id_str): Path<String>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    // Require login: caller must have a valid walker_id cookie.
    let Some(caller) = super::cookie_user_id(&headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    if !db::user_exists(&ctx.db_pool, caller).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let Ok(user_id) = uuid::Uuid::parse_str(&id_str) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    ws.on_upgrade(move |socket| handle_ws_live_user(socket, ctx, user_id))
        .into_response()
}

async fn handle_ws_live_user(mut socket: WebSocket, ctx: SharedLive, user_id: uuid::Uuid) {
    info!(user_id = %user_id, "Activity viewer connected");

    // Get or create the per-user broadcast channel.
    let mut rx = {
        let mut txs = ctx.user_txs.write().await;
        let tx = txs
            .entry(user_id)
            .or_insert_with(|| broadcast::channel(16).0);
        tx.subscribe()
    };

    // Send current segment immediately.
    match db::get_current_segment_json(&ctx.db_pool, user_id).await {
        Ok(json) => {
            let _ = socket.send(Message::Text(json.into())).await;
        }
        Err(e) => error!(error = %e, "Failed to query initial segment"),
    }

    loop {
        match rx.recv().await {
            Ok(json) => {
                if socket.send(Message::Text(json.into())).await.is_err() {
                    break;
                }
            }
            Err(broadcast::error::RecvError::Lagged(n)) => {
                warn!(skipped = n, "Activity viewer lagging, skipped messages");
            }
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }

    info!(user_id = %user_id, "Activity viewer disconnected");
}

/// Lightweight timer: checks for disconnected users every 5 seconds
/// and closes their open segments.
pub fn spawn_disconnect_checker(ctx: SharedLive) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
        interval.tick().await; // Skip the first instant tick.
        loop {
            interval.tick().await;

            // Close segments for disconnected users (no heartbeat in 30s).
            match db::close_stale_segments(&ctx.db_pool, 30.0).await {
                Ok(user_ids) => {
                    // Push null segment to viewers watching disconnected users.
                    for user_id in &user_ids {
                        push_user_segment(&ctx, *user_id).await;
                    }
                }
                Err(e) => {
                    error!(error = %e, "Disconnect checker failed to close stale segments");
                }
            }

            // Notify all viewers that state may have changed.
            let _ = ctx.broadcast_tx.send(());
        }
    });
}
