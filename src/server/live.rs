use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::{RwLock, broadcast};
use tracing::{info, warn};

use super::state::{LiveBroadcast, LiveState};

#[derive(Clone)]
pub struct TokenUser {
    pub id: String,
    pub email: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
}

/// Token → user info lookup.
pub type TokenMap = Arc<RwLock<std::collections::HashMap<String, TokenUser>>>;

pub struct LiveContext {
    pub state: Arc<RwLock<LiveState>>,
    pub tokens: TokenMap,
    pub broadcast_tx: broadcast::Sender<LiveBroadcast>,
    pub db_pool: Option<Arc<sqlx::PgPool>>,
    pub dev_mode: bool,
}

pub type SharedLive = Arc<LiveContext>;

pub fn routes() -> Router<SharedLive> {
    Router::new()
        .route("/api/update", post(handle_update))
        .route("/api/simulate/register", post(register_simulated_user))
        .route("/ws/live", get(ws_live))
}

// -- POST /api/update --

#[derive(Deserialize)]
struct UpdatePayload {
    moving: bool,
    speed_mph: f64,
}

async fn handle_update(
    State(ctx): State<SharedLive>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<UpdatePayload>,
) -> impl IntoResponse {
    let Some(token) = extract_bearer_token(&headers) else {
        return StatusCode::UNAUTHORIZED;
    };

    let tokens = ctx.tokens.read().await;
    let Some(user) = tokens.get(token) else {
        return StatusCode::UNAUTHORIZED;
    };
    let user = user.clone();
    drop(tokens);

    // Load today's totals from DB if this is a new user in memory.
    let initial_stats = {
        let state = ctx.state.read().await;
        if state.users.contains_key(&user.email) {
            (0, 0, 0)
        } else {
            drop(state);
            if let Some(ref pool) = ctx.db_pool {
                super::db::load_daily_stats(pool, &user.email)
                    .await
                    .ok()
                    .flatten()
                    .map(|(c, a, i)| (c as u64, a as u64, i as u64))
                    .unwrap_or((0, 0, 0))
            } else {
                (0, 0, 0)
            }
        }
    };

    // Compute calories, update state.
    let (delta, broadcast) = {
        let mut state = ctx.state.write().await;
        let delta = state.process_update(
            &user.id,
            &user.email,
            &user.display_name,
            user.avatar_url.clone(),
            payload.moving,
            payload.speed_mph,
            initial_stats,
        );
        let broadcast = state.snapshot();
        (delta, broadcast)
    };

    // Write delta to DB (accumulates into daily total).
    if let Some(ref pool) = ctx.db_pool
        && !delta.is_empty()
    {
        let _ = super::db::accumulate_daily_stats(
            pool,
            &user.email,
            delta.calories_ucal,
            delta.active_secs,
            delta.idle_secs,
            delta.distance_m,
        )
        .await;
    }

    // Broadcast to all viewers.
    let _ = ctx.broadcast_tx.send(broadcast);

    StatusCode::OK
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
    let id = format!("sim-{}", payload.name);

    // Register in DB if available.
    if let Some(ref pool) = ctx.db_pool {
        let _ = super::db::upsert_user(pool, &payload.email, &payload.name, None).await;
        let _ = super::db::store_token(pool, &token, &payload.email).await;
    }

    // Register in token map.
    ctx.tokens.write().await.insert(
        token.clone(),
        TokenUser {
            id,
            email: payload.email,
            display_name: payload.name,
            avatar_url: None,
        },
    );

    (StatusCode::OK, Json(SimulateRegisterResponse { token }))
}

fn extract_bearer_token(headers: &axum::http::HeaderMap) -> Option<&str> {
    headers
        .get("authorization")?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

// -- WebSocket /ws/live --

async fn ws_live(State(ctx): State<SharedLive>, ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws_live(socket, ctx))
}

async fn handle_ws_live(mut socket: WebSocket, ctx: SharedLive) {
    let mut rx = ctx.broadcast_tx.subscribe();
    info!("Live viewer connected");

    // Send current state immediately so the dashboard doesn't start empty.
    {
        let state = ctx.state.read().await;
        let snapshot = state.snapshot();
        if let Ok(json) = serde_json::to_string(&snapshot) {
            let _ = socket.send(Message::Text(json.into())).await;
        }
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
/// and broadcasts updated state if any status changed.
pub fn spawn_disconnect_checker(ctx: SharedLive) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
        loop {
            interval.tick().await;
            let broadcast = {
                let state = ctx.state.read().await;
                state.snapshot()
            };
            let _ = ctx.broadcast_tx.send(broadcast);
        }
    });
}
