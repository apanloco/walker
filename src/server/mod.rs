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
    let closed = db::close_stale_segments(&pool, 60.0)
        .await
        .unwrap_or_default();
    if !closed.is_empty() {
        info!(
            "Closed {} stale open segment(s) from previous run",
            closed.len()
        );
    }

    // Dev mode: create a test token.
    if config.dev {
        let dev_token = "dev-token-walker";
        let dev_email = "dev@walker.local";
        let dev_name = "Dev User";

        db::upsert_user(&pool, dev_email, dev_name, None).await?;
        let id = db::get_user_id(&pool, dev_email).await?;
        let _ = db::store_token(&pool, dev_token, id).await;
        db::seed_dev_history(&pool, id).await?;

        info!(
            "Dev mode: test token = '{dev_token}', dashboard login: http://localhost:{}/dev/login",
            config.port
        );
    }

    let live_ctx = Arc::new(live::LiveContext {
        broadcast_tx,
        user_txs: RwLock::new(std::collections::HashMap::new()),
        db_pool: pool.clone(),
        dev_mode: config.dev,
    });

    let auth_state = Arc::new(RwLock::new(auth::ServerState::new(
        config.base_url,
        config.github_client_id,
        config.github_client_secret,
        config.google_client_id,
        config.google_client_secret,
        pool.clone(),
    )));

    // Lightweight timer for disconnect detection.
    live::spawn_disconnect_checker(live_ctx.clone());

    let mut app = Router::new()
        .merge(auth::routes().with_state(auth_state))
        .merge(update::routes().with_state(live_ctx.clone()))
        .merge(live::routes().with_state(live_ctx.clone()))
        .merge(leaderboard::routes().with_state(live_ctx.clone()))
        .merge(profile::routes().with_state(live_ctx.clone()))
        .merge(activity::routes().with_state(live_ctx))
        .merge(dashboard::routes(config.dev));

    // In dev mode, inject walker_id cookie on every response so the dashboard
    // works without OAuth. No manual /dev/login step needed.
    if config.dev {
        let dev_user_id = db::get_user_id(&pool, "dev@walker.local")
            .await
            .ok()
            .map(|id| id.to_string())
            .unwrap_or_default();
        let cookie_value = format!("walker_id={dev_user_id}; Path=/; SameSite=Lax");
        app = app.layer(axum::middleware::map_response(
            move |mut response: axum::response::Response| {
                let cookie = cookie_value.clone();
                async move {
                    response
                        .headers_mut()
                        .insert(axum::http::header::SET_COOKIE, cookie.parse().unwrap());
                    response
                }
            },
        ));
    }

    let addr = format!("0.0.0.0:{}", config.port);
    info!("Walker server listening on http://{addr}");

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
