use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::broadcast;
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

#[derive(Serialize, Clone)]
pub struct LiveBroadcast {
    pub users: Vec<LiveUser>,
}

#[derive(Serialize, Clone)]
pub struct LiveUser {
    pub id: uuid::Uuid,
    pub name: String,
    pub avatar_url: Option<String>,
    pub status: String,
    pub speed_kmh: f64,
    pub calories_kcal: f64,
    pub distance_m: f64,
    pub active_secs: u64,
}

pub struct LiveContext {
    pub broadcast_tx: broadcast::Sender<LiveBroadcast>,
    pub db_pool: Arc<sqlx::PgPool>,
    pub dev_mode: bool,
}

pub type SharedLive = Arc<LiveContext>;

pub fn routes() -> Router<SharedLive> {
    Router::new()
        .route("/api/simulate/register", post(register_simulated_user))
        .route("/ws/live", get(ws_live))
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

// -- WebSocket /ws/live --

async fn ws_live(State(ctx): State<SharedLive>, ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws_live(socket, ctx))
}

async fn handle_ws_live(mut socket: WebSocket, ctx: SharedLive) {
    let mut rx = ctx.broadcast_tx.subscribe();
    info!("Live viewer connected");

    // Send current state immediately so the dashboard doesn't start empty.
    match db::live_snapshot(&ctx.db_pool).await {
        Ok(snapshot) => {
            if let Ok(json) = serde_json::to_string(&snapshot) {
                let _ = socket.send(Message::Text(json.into())).await;
            }
        }
        Err(e) => error!(error = %e, "Failed to build initial live snapshot"),
    }

    loop {
        match rx.recv().await {
            Ok(broadcast) => {
                let Ok(json) = serde_json::to_string(&broadcast) else {
                    continue;
                };
                if socket.send(Message::Text(json.into())).await.is_err() {
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

/// Lightweight timer: checks for disconnected users every 5 seconds
/// and closes their open segments.
pub fn spawn_disconnect_checker(ctx: SharedLive) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
        loop {
            interval.tick().await;

            // Close segments for disconnected users (no heartbeat in 5s).
            if let Err(e) = db::close_stale_segments(&ctx.db_pool, 5.0).await {
                error!(error = %e, "Disconnect checker failed to close stale segments");
            }

            // Broadcast current state to all viewers.
            match db::live_snapshot(&ctx.db_pool).await {
                Ok(broadcast) => {
                    let _ = ctx.broadcast_tx.send(broadcast);
                }
                Err(e) => error!(error = %e, "Disconnect checker failed to build live snapshot"),
            }
        }
    });
}
