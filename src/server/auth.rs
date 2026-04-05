use axum::{
    Router,
    extract::{Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect},
    routing::get,
};
use rand::RngExt;
use serde::Deserialize;
use std::sync::Arc;
use tracing::info;

/// Read-only server config. No mutex needed — nothing is mutated after startup.
pub struct ServerState {
    pub github_client_id: Option<String>,
    pub github_client_secret: Option<String>,
    pub google_client_id: Option<String>,
    pub google_client_secret: Option<String>,
    pub base_url: String,
    pub db_pool: Arc<sqlx::PgPool>,
    pub dev: bool,
}

pub type SharedState = Arc<ServerState>;

impl ServerState {
    pub fn has_github(&self) -> bool {
        self.github_client_id.is_some() && self.github_client_secret.is_some()
    }

    pub fn has_google(&self) -> bool {
        self.google_client_id.is_some() && self.google_client_secret.is_some()
    }
}

pub fn routes() -> Router<SharedState> {
    Router::new()
        .route("/login", get(login_page))
        .route("/auth/github/callback", get(github_callback))
        .route("/auth/google/callback", get(google_callback))
        .route("/auth/dev/callback", get(dev_callback))
}

// -- Login page --

#[derive(Deserialize)]
struct LoginParams {
    cli_port: Option<u16>,
}

/// The `state` parameter encodes the flow: "web" for dashboard, "cli:<port>" for CLI.
fn flow_state(cli_port: Option<u16>) -> String {
    match cli_port {
        Some(port) => format!("cli:{port}"),
        None => "web".to_string(),
    }
}

async fn login_page(
    State(state): State<SharedState>,
    Query(params): Query<LoginParams>,
) -> Html<String> {
    let flow = flow_state(params.cli_port);

    let mut buttons = String::new();

    if state.has_github() {
        let client_id = state.github_client_id.as_ref().unwrap();
        let callback = format!("{}/auth/github/callback", state.base_url);
        let url = format!(
            "https://github.com/login/oauth/authorize?client_id={client_id}&redirect_uri={}&state={flow}&scope=read:user%20user:email",
            urlencoding::encode(&callback),
        );
        buttons.push_str(&format!(
            r#"<a class="btn github" href="{url}">Login with GitHub</a>"#
        ));
    }

    if state.has_google() {
        let client_id = state.google_client_id.as_ref().unwrap();
        let callback = format!("{}/auth/google/callback", state.base_url);
        let url = format!(
            "https://accounts.google.com/o/oauth2/v2/auth?client_id={client_id}&redirect_uri={}&state={flow}&response_type=code&scope=openid%20profile%20email",
            urlencoding::encode(&callback),
        );
        buttons.push_str(&format!(
            r#"<a class="btn google" href="{url}">Login with Google</a>"#
        ));
    }

    if state.dev {
        buttons.push_str(&format!(
            r#"<a class="btn dev" href="/auth/dev/callback?state={flow}">Dev Login</a>"#
        ));
    }

    if buttons.is_empty() {
        buttons = "No login providers configured.".to_string();
    }

    Html(format!(
        r#"<!DOCTYPE html>
<html>
<head><title>Walker - Login</title>
<style>
  body {{ font-family: system-ui; max-width: 440px; margin: 60px auto; text-align: center; color: #e0e0e0; background: #1a1a1a; }}
  h1 {{ font-size: 2.5em; margin-bottom: 0.2em; }}
  .tagline {{ color: #999; font-size: 0.95em; margin-bottom: 2em; }}
  .buttons {{ margin-top: 24px; display: flex; flex-direction: column; gap: 12px; align-items: center; }}
  a.btn {{ display: inline-block; padding: 12px 24px; width: 220px;
           color: white; text-decoration: none; border-radius: 6px; font-size: 16px; }}
  a.btn.github {{ background: #24292e; }}
  a.btn.github:hover {{ background: #444; }}
  a.btn.google {{ background: #4285f4; }}
  a.btn.google:hover {{ background: #3367d6; }}
  a.btn.dev {{ background: #059669; }}
  a.btn.dev:hover {{ background: #047857; }}
  .footer {{ margin-top: 3em; font-size: 0.85em; color: #666; }}
  .footer a {{ color: #888; }}
</style>
</head>
<body>
  <h1>Walker</h1>
  <p class="tagline">Real-time treadmill tracking with honest calories.<br>Connect, walk, compete.</p>
  <div class="buttons">
    {buttons}
  </div>
  <div class="footer">
    <a href="https://github.com/apanloco/walker">GitHub</a> &middot; How to get started, supported devices, and more.
  </div>
</body>
</html>"#
    ))
}

// -- GitHub OAuth --

#[derive(Deserialize)]
struct OAuthCallbackParams {
    code: String,
    state: String,
}

#[derive(Deserialize)]
struct GitHubTokenResponse {
    access_token: String,
}

#[derive(Deserialize)]
struct GitHubUser {
    login: String,
    avatar_url: Option<String>,
}

#[derive(Deserialize)]
struct GitHubEmail {
    email: String,
    primary: bool,
    verified: bool,
}

async fn github_callback(
    State(state): State<SharedState>,
    Query(params): Query<OAuthCallbackParams>,
) -> impl IntoResponse {
    let (client_id, client_secret) = match (&state.github_client_id, &state.github_client_secret) {
        (Some(id), Some(secret)) => (id.clone(), secret.clone()),
        _ => return Html("<h1>GitHub not configured</h1>".to_string()).into_response(),
    };

    let client = reqwest::Client::new();

    let Ok(token_res) = client
        .post("https://github.com/login/oauth/access_token")
        .header("Accept", "application/json")
        .json(&serde_json::json!({
            "client_id": client_id,
            "client_secret": client_secret,
            "code": params.code,
        }))
        .send()
        .await
    else {
        return Html("<h1>Error contacting GitHub</h1>".to_string()).into_response();
    };

    let Ok(token_data) = token_res.json::<GitHubTokenResponse>().await else {
        return Html("<h1>Error exchanging code for token</h1>".to_string()).into_response();
    };

    let auth_header = format!("Bearer {}", token_data.access_token);

    let Ok(user_res) = client
        .get("https://api.github.com/user")
        .header("Authorization", &auth_header)
        .header("User-Agent", "walker")
        .send()
        .await
    else {
        return Html("<h1>Error fetching user info</h1>".to_string()).into_response();
    };

    let Ok(github_user) = user_res.json::<GitHubUser>().await else {
        return Html("<h1>Error parsing user info</h1>".to_string()).into_response();
    };

    let Ok(email_res) = client
        .get("https://api.github.com/user/emails")
        .header("Authorization", &auth_header)
        .header("User-Agent", "walker")
        .send()
        .await
    else {
        return Html("<h1>Error fetching email</h1>".to_string()).into_response();
    };

    let Ok(emails) = email_res.json::<Vec<GitHubEmail>>().await else {
        return Html("<h1>Error parsing emails</h1>".to_string()).into_response();
    };

    let Some(email) = emails
        .into_iter()
        .find(|e| e.primary && e.verified)
        .map(|e| e.email)
    else {
        return Html("<h1>No verified primary email found on GitHub</h1>".to_string())
            .into_response();
    };

    complete_auth(
        &state,
        &params.state,
        &email,
        &github_user.login,
        github_user.avatar_url.as_deref(),
        "github",
    )
    .await
    .into_response()
}

// -- Google OAuth --

#[derive(Deserialize)]
struct GoogleTokenResponse {
    access_token: String,
}

#[derive(Deserialize)]
struct GoogleUser {
    email: Option<String>,
    name: Option<String>,
    picture: Option<String>,
}

async fn google_callback(
    State(state): State<SharedState>,
    Query(params): Query<OAuthCallbackParams>,
) -> impl IntoResponse {
    let (client_id, client_secret) = match (&state.google_client_id, &state.google_client_secret) {
        (Some(id), Some(secret)) => (id.clone(), secret.clone()),
        _ => return Html("<h1>Google not configured</h1>".to_string()).into_response(),
    };

    let client = reqwest::Client::new();
    let callback = format!("{}/auth/google/callback", state.base_url);

    let Ok(token_res) = client
        .post("https://oauth2.googleapis.com/token")
        .form(&[
            ("client_id", client_id.as_str()),
            ("client_secret", client_secret.as_str()),
            ("code", params.code.as_str()),
            ("redirect_uri", callback.as_str()),
            ("grant_type", "authorization_code"),
        ])
        .send()
        .await
    else {
        return Html("<h1>Error contacting Google</h1>".to_string()).into_response();
    };

    let Ok(token_data) = token_res.json::<GoogleTokenResponse>().await else {
        return Html("<h1>Error exchanging code for token</h1>".to_string()).into_response();
    };

    let Ok(user_res) = client
        .get("https://openidconnect.googleapis.com/v1/userinfo")
        .header(
            "Authorization",
            format!("Bearer {}", token_data.access_token),
        )
        .send()
        .await
    else {
        return Html("<h1>Error fetching user info</h1>".to_string()).into_response();
    };

    let Ok(google_user) = user_res.json::<GoogleUser>().await else {
        return Html("<h1>Error parsing user info</h1>".to_string()).into_response();
    };

    let Some(email) = google_user.email else {
        return Html("<h1>No email returned from Google</h1>".to_string()).into_response();
    };

    let display_name = google_user.name.unwrap_or_else(|| email.clone());

    complete_auth(
        &state,
        &params.state,
        &email,
        &display_name,
        google_user.picture.as_deref(),
        "google",
    )
    .await
    .into_response()
}

// -- Dev provider --

#[derive(Deserialize)]
struct DevCallbackParams {
    state: String,
}

async fn dev_callback(
    State(state): State<SharedState>,
    Query(params): Query<DevCallbackParams>,
) -> impl IntoResponse {
    if !state.dev {
        return Html("<h1>Dev login is only available in dev mode</h1>".to_string())
            .into_response();
    }

    complete_auth(
        &state,
        &params.state,
        "dev@walker.local",
        "Dev User",
        None,
        "dev",
    )
    .await
    .into_response()
}

// -- Shared auth completion --

/// Handles both flows after OAuth identity is established:
/// - "web" → upsert user, set cookie, redirect to /
/// - "cli:<port>" → upsert user, create token, redirect to localhost callback
async fn complete_auth(
    state: &ServerState,
    flow: &str,
    email: &str,
    display_name: &str,
    avatar_url: Option<&str>,
    provider: &str,
) -> axum::response::Response {
    let pool = &state.db_pool;

    let user_id =
        match super::db::upsert_user_returning_id(pool, email, display_name, avatar_url).await {
            Ok(id) => id,
            Err(e) => {
                tracing::error!(error = %e, "Failed to upsert user");
                return Html("<h1>Database error</h1>".to_string()).into_response();
            }
        };

    info!(user = %display_name, provider = %provider, "User authenticated");

    if flow == "web" {
        return set_cookie_and_redirect(&user_id.to_string());
    }

    // CLI flow: state is "cli:<port>"
    if let Some(port_str) = flow.strip_prefix("cli:") {
        let Ok(port) = port_str.parse::<u16>() else {
            return Html("<h1>Invalid CLI port</h1>".to_string()).into_response();
        };

        let walker_token = generate_token(48);
        if let Err(e) = super::db::store_token(pool, &walker_token, user_id).await {
            tracing::error!(error = %e, "Failed to store token");
            return Html("<h1>Database error</h1>".to_string()).into_response();
        }

        let callback_url = format!(
            "http://localhost:{port}/callback?token={}&email={}&name={}",
            urlencoding::encode(&walker_token),
            urlencoding::encode(email),
            urlencoding::encode(display_name),
        );

        return Redirect::temporary(&callback_url).into_response();
    }

    Html("<h1>Invalid login flow</h1>".to_string()).into_response()
}

fn set_cookie_and_redirect(user_id: &str) -> axum::response::Response {
    use axum::http::header;
    let cookie = format!(
        "walker_id={}; Path=/; SameSite=Lax; Max-Age=2592000",
        user_id
    );
    (
        [
            (header::SET_COOKIE, cookie),
            (header::LOCATION, "/".to_string()),
        ],
        StatusCode::FOUND,
    )
        .into_response()
}

// -- Helpers --

fn generate_token(len: usize) -> String {
    use rand::distr::Alphanumeric;
    rand::rng()
        .sample_iter(Alphanumeric)
        .take(len)
        .map(char::from)
        .collect()
}
