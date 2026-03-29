pub mod auth;
pub mod dashboard;
pub mod db;
pub mod leaderboard;
pub mod live;
pub mod profile;
pub mod state;

use axum::Router;
use std::sync::Arc;
use tokio::sync::{RwLock, broadcast};
use tracing::info;

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
        tracing::warn!("  Database: not configured (set DATABASE_URL). Running in-memory only.");
    }
    if config.dev {
        info!("  Dev mode: enabled");
    }

    let token_map: live::TokenMap = Arc::new(RwLock::new(std::collections::HashMap::new()));
    let (broadcast_tx, _) = broadcast::channel(64);

    // Connect to database.
    let pool = if let Some(ref db_url) = config.database_url {
        let pool = db::connect(db_url).await?;
        let tokens = db::load_tokens(&pool).await?;
        let count = tokens.len();
        let mut map = token_map.write().await;
        for (token, user) in tokens {
            map.insert(token, user);
        }
        drop(map);
        info!("Loaded {count} token(s) from database");
        Some(Arc::new(pool))
    } else {
        info!("No DATABASE_URL — running without persistence");
        None
    };

    // Dev mode: create a test token.
    if config.dev {
        let dev_token = "dev-token-walker";
        let dev_email = "dev@walker.local";
        let dev_name = "Dev User";

        if let Some(ref pool) = pool {
            db::upsert_user(pool, dev_email, dev_name, None).await?;
            let _ = db::store_token(pool, dev_token, dev_email).await;
        }

        token_map.write().await.insert(
            dev_token.to_string(),
            live::TokenUser {
                id: "dev-user".to_string(),
                email: dev_email.to_string(),
                display_name: dev_name.to_string(),
                avatar_url: None,
            },
        );
        info!("Dev mode: test token = '{dev_token}'");
    }

    let live_ctx = Arc::new(live::LiveContext {
        state: Arc::new(RwLock::new(state::LiveState::new())),
        tokens: token_map.clone(),
        broadcast_tx,
        db_pool: pool.clone(),
        dev_mode: config.dev,
    });

    let auth_state = Arc::new(RwLock::new(auth::ServerState::new(
        config.base_url,
        config.github_client_id,
        config.github_client_secret,
        config.google_client_id,
        config.google_client_secret,
        token_map,
        pool.clone(),
    )));

    // Lightweight timer for disconnect detection.
    live::spawn_disconnect_checker(live_ctx.clone());

    let app = Router::new()
        .merge(auth::routes().with_state(auth_state))
        .merge(live::routes().with_state(live_ctx.clone()))
        .merge(leaderboard::routes().with_state(live_ctx.clone()))
        .merge(profile::routes().with_state(live_ctx))
        .merge(dashboard::routes(config.dev));

    let addr = format!("0.0.0.0:{}", config.port);
    info!("Walker server listening on http://{addr}");

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
