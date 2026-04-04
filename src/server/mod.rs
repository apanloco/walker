pub mod activity;
pub mod auth;
pub mod dashboard;
pub mod db;
pub mod leaderboard;
pub mod live;
pub mod profile;
pub mod update;

use axum::Router;
use std::sync::Arc;
use tokio::sync::{RwLock, broadcast};
use tracing::info;

/// Extract the walker_id cookie as a UUID, or None if missing/invalid.
pub fn cookie_user_id(headers: &axum::http::HeaderMap) -> Option<uuid::Uuid> {
    let cookie_header = headers.get(axum::http::header::COOKIE)?.to_str().ok()?;
    cookie_header
        .split(';')
        .map(|s| s.trim())
        .find(|s| s.starts_with("walker_id="))
        .and_then(|s| uuid::Uuid::parse_str(&s["walker_id=".len()..]).ok())
}

pub struct ServerConfig {
    pub port: u16,
    pub base_url: String,
    pub github_client_id: Option<String>,
    pub github_client_secret: Option<String>,
    pub google_client_id: Option<String>,
    pub google_client_secret: Option<String>,
    pub database_url: Option<String>,
    pub dev: bool,
}

pub async fn run(config: ServerConfig) -> anyhow::Result<()> {
    // -- Startup checks --
    info!("Starting Walker server...");
    info!("  Base URL: {}", config.base_url);

    let has_github = config.github_client_id.is_some() && config.github_client_secret.is_some();
    let has_google = config.google_client_id.is_some() && config.google_client_secret.is_some();

    if has_github {
        info!("  GitHub OAuth: configured");
    } else {
        tracing::warn!(
            "  GitHub OAuth: not configured (set WALKER_GITHUB_CLIENT_ID + WALKER_GITHUB_CLIENT_SECRET)"
        );
    }
    if has_google {
        info!("  Google OAuth: configured");
    } else {
        tracing::warn!(
            "  Google OAuth: not configured (set WALKER_GOOGLE_CLIENT_ID + WALKER_GOOGLE_CLIENT_SECRET)"
        );
    }
    if !has_github && !has_google && !config.dev {
        tracing::warn!(
            "  No login providers configured! Users won't be able to log in. Use --dev for testing."
        );
    }
    if config.database_url.is_some() {
        info!("  Database: configured");
    } else {
        anyhow::bail!("DATABASE_URL is required. Set it to a PostgreSQL connection string.");
    }
    if config.dev {
        info!("  Dev mode: enabled");
    }

    let (broadcast_tx, _) = broadcast::channel(64);

    // Connect to database.
    let pool = Arc::new(db::connect(config.database_url.as_ref().unwrap()).await?);

    // Close stale open segments from any previous crash (1 minute threshold).
    match db::close_stale_segments(&pool, 60.0).await {
        Ok(closed) if !closed.is_empty() => {
            info!(
                "Closed {} stale open segment(s) from previous run",
                closed.len()
            );
        }
        Err(e) => {
            tracing::warn!(error = %e, "Failed to close stale segments on startup");
        }
        _ => {}
    }

    // Dev mode: seed dev user + history (but no auto-login — user must log in).
    if config.dev {
        let id = db::upsert_user_returning_id(&pool, "dev@walker.local", "Dev User", None).await?;
        db::seed_dev_history(&pool, id).await?;

        info!("Dev mode: login at http://localhost:{}/login", config.port);
    }

    let live_ctx = Arc::new(live::LiveContext {
        broadcast_tx,
        user_txs: RwLock::new(std::collections::HashMap::new()),
        db_pool: pool.clone(),
        dev_mode: config.dev,
    });

    let auth_state: auth::SharedState = Arc::new(auth::ServerState {
        github_client_id: config.github_client_id,
        github_client_secret: config.github_client_secret,
        google_client_id: config.google_client_id,
        google_client_secret: config.google_client_secret,
        base_url: config.base_url,
        db_pool: pool.clone(),
        dev: config.dev,
    });

    // Lightweight timer for disconnect detection.
    live::spawn_disconnect_checker(live_ctx.clone());

    // Stale cookie middleware: if walker_id references a non-existent user, clear it.
    let stale_pool = pool.clone();
    let app = Router::new()
        .merge(auth::routes().with_state(auth_state))
        .merge(update::routes().with_state(live_ctx.clone()))
        .merge(live::routes().with_state(live_ctx.clone()))
        .merge(leaderboard::routes().with_state(live_ctx.clone()))
        .merge(profile::routes().with_state(live_ctx.clone()))
        .merge(activity::routes().with_state(live_ctx))
        .merge(dashboard::routes(config.dev))
        .layer(axum::middleware::map_response(
            move |request_headers: axum::http::HeaderMap,
                  mut response: axum::response::Response| {
                let pool = stale_pool.clone();
                async move {
                    if let Some(user_id) = cookie_user_id(&request_headers) {
                        if !db::user_exists(&pool, user_id).await.unwrap_or(true) {
                            // User doesn't exist — clear the cookie.
                            if let Ok(val) = "walker_id=; Path=/; Max-Age=0".parse() {
                                response
                                    .headers_mut()
                                    .insert(axum::http::header::SET_COOKIE, val);
                            }
                        }
                    }
                    response
                }
            },
        ));

    let addr = format!("0.0.0.0:{}", config.port);
    info!("Walker server listening on http://{addr}");

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
