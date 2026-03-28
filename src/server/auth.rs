use axum::{
    Json, Router,
    extract::{Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect},
    routing::{get, post},
};
use rand::RngExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::info;

pub type SharedState = Arc<RwLock<ServerState>>;

pub struct ServerState {
    pub github_client_id: Option<String>,
    pub github_client_secret: Option<String>,
    pub google_client_id: Option<String>,
    pub google_client_secret: Option<String>,
    /// Base URL for constructing callback URLs (e.g., "http://localhost:3000").
    pub base_url: String,
    /// Pending device code authorizations.
    pub device_codes: HashMap<String, DeviceAuth>,
    /// Shared token map: token → user info. Used by live endpoints to authenticate.
    pub token_map: super::live::TokenMap,
    /// Database pool (optional).
    pub db_pool: Option<std::sync::Arc<sqlx::PgPool>>,
}

pub struct DeviceAuth {
    pub user_code: String,
    pub created_at: Instant,
    /// Set once the user completes OAuth.
    pub token: Option<String>,
    pub user: Option<AuthUser>,
}

/// User identity — email is the primary key (shared across providers).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthUser {
    pub email: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub provider: String,
}

impl ServerState {
    pub fn new(
        base_url: String,
        github_client_id: Option<String>,
        github_client_secret: Option<String>,
        google_client_id: Option<String>,
        google_client_secret: Option<String>,
        token_map: super::live::TokenMap,
        db_pool: Option<std::sync::Arc<sqlx::PgPool>>,
    ) -> Self {
        Self {
            github_client_id,
            github_client_secret,
            google_client_id,
            google_client_secret,
            base_url,
            device_codes: HashMap::new(),
            token_map,
            db_pool,
        }
    }

    pub fn has_github(&self) -> bool {
        self.github_client_id.is_some() && self.github_client_secret.is_some()
    }

    pub fn has_google(&self) -> bool {
        self.google_client_id.is_some() && self.google_client_secret.is_some()
    }
}

pub fn routes() -> Router<SharedState> {
    Router::new()
        .route("/auth/device", post(create_device_code))
        .route("/auth/device/token", post(poll_device_token))
        .route("/auth/device/verify", get(verify_page))
        .route("/auth/web/github", get(web_github_redirect))
        .route("/auth/web/google", get(web_google_redirect))
        .route("/auth/github", get(github_redirect))
        .route("/auth/github/callback", get(github_callback))
        .route("/auth/google", get(google_redirect))
        .route("/auth/google/callback", get(google_callback))
}

// -- Device code flow --

#[derive(Serialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_url: String,
    expires_in: u64,
    interval: u64,
}

async fn create_device_code(State(state): State<SharedState>) -> Json<DeviceCodeResponse> {
    let device_code = generate_token(32);
    let user_code = generate_user_code();

    let mut state = state.write().await;
    state.device_codes.insert(
        device_code.clone(),
        DeviceAuth {
            user_code: user_code.clone(),
            created_at: Instant::now(),
            token: None,
            user: None,
        },
    );

    info!(user_code = %user_code, "Device code created");

    Json(DeviceCodeResponse {
        device_code,
        user_code,
        verification_url: String::new(),
        expires_in: 900,
        interval: 2,
    })
}

#[derive(Deserialize)]
struct PollRequest {
    device_code: String,
}

async fn poll_device_token(
    State(state): State<SharedState>,
    Json(req): Json<PollRequest>,
) -> impl IntoResponse {
    let state = state.read().await;

    let Some(auth) = state.device_codes.get(&req.device_code) else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "invalid_device_code"})),
        );
    };

    if auth.created_at.elapsed() > Duration::from_secs(900) {
        return (
            StatusCode::GONE,
            Json(serde_json::json!({"error": "expired"})),
        );
    }

    if let (Some(token), Some(user)) = (&auth.token, &auth.user) {
        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "token": token,
                "user": user,
            })),
        );
    }

    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({"error": "authorization_pending"})),
    )
}

// -- Verification page --

#[derive(Deserialize)]
struct VerifyParams {
    code: Option<String>,
}

async fn verify_page(
    State(state): State<SharedState>,
    Query(params): Query<VerifyParams>,
) -> Html<String> {
    let code_value = params.code.unwrap_or_default();
    let state = state.read().await;

    let mut buttons = String::new();
    if state.has_github() {
        buttons.push_str(&format!(
            r#"<a class="btn github" href="/auth/github?user_code={code_value}">Login with GitHub</a>"#
        ));
    }
    if state.has_google() {
        buttons.push_str(&format!(
            r#"<a class="btn google" href="/auth/google?user_code={code_value}">Login with Google</a>"#
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
  body {{ font-family: system-ui; max-width: 400px; margin: 80px auto; text-align: center; }}
  input {{ font-size: 24px; text-align: center; padding: 8px; letter-spacing: 4px; width: 200px; }}
  .buttons {{ margin-top: 24px; display: flex; flex-direction: column; gap: 12px; align-items: center; }}
  a.btn {{ display: inline-block; padding: 12px 24px; width: 220px;
           color: white; text-decoration: none; border-radius: 6px; font-size: 16px; }}
  a.btn.github {{ background: #24292e; }}
  a.btn.github:hover {{ background: #444; }}
  a.btn.google {{ background: #4285f4; }}
  a.btn.google:hover {{ background: #3367d6; }}
</style>
</head>
<body>
  <h1>Walker</h1>
  <p>Enter the code shown in your terminal:</p>
  <input value="{code_value}" maxlength="9" readonly />
  <div class="buttons">
    {buttons}
  </div>
</body>
</html>"#
    ))
}

// -- GitHub OAuth --

#[derive(Deserialize)]
struct OAuthRedirectParams {
    user_code: String,
}

async fn github_redirect(
    State(state): State<SharedState>,
    Query(params): Query<OAuthRedirectParams>,
) -> impl IntoResponse {
    let state = state.read().await;
    let Some(client_id) = &state.github_client_id else {
        return Html("GitHub login not configured".to_string()).into_response();
    };
    let callback = format!("{}/auth/github/callback", state.base_url);
    let redirect_url = format!(
        "https://github.com/login/oauth/authorize?client_id={client_id}&redirect_uri={}&state={}&scope=read:user%20user:email",
        urlencoded(&callback),
        params.user_code,
    );
    Redirect::temporary(&redirect_url).into_response()
}

#[derive(Deserialize)]
struct GitHubCallbackParams {
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
    Query(params): Query<GitHubCallbackParams>,
) -> impl IntoResponse {
    let flow = params.state.clone();
    let client = reqwest::Client::new();

    let (client_id, client_secret) = {
        let s = state.read().await;
        match (&s.github_client_id, &s.github_client_secret) {
            (Some(id), Some(secret)) => (id.clone(), secret.clone()),
            _ => return Html("<h1>GitHub not configured</h1>".to_string()).into_response(),
        }
    };

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

    let auth_user = AuthUser {
        email: email.clone(),
        display_name: github_user.login,
        avatar_url: github_user.avatar_url,
        provider: "github".to_string(),
    };

    // Web dashboard login: set cookie, redirect to /.
    if flow == "web" {
        let s = state.read().await;
        let user_id = if let Some(ref pool) = s.db_pool {
            let _ = super::db::upsert_user(
                pool,
                &auth_user.email,
                &auth_user.display_name,
                auth_user.avatar_url.as_deref(),
            )
            .await;
            super::db::get_user_id(pool, &email)
                .await
                .unwrap_or_else(|_| email.clone())
        } else {
            email.clone()
        };
        return set_cookie_and_redirect(&user_id);
    }

    // CLI device code flow: complete the device auth.
    complete_auth(&state, &flow, auth_user)
        .await
        .into_response()
}

// -- Google OAuth --

async fn google_redirect(
    State(state): State<SharedState>,
    Query(params): Query<OAuthRedirectParams>,
) -> impl IntoResponse {
    let state = state.read().await;
    let Some(client_id) = &state.google_client_id else {
        return Html("Google login not configured".to_string()).into_response();
    };
    let callback = format!("{}/auth/google/callback", state.base_url);
    let redirect_url = format!(
        "https://accounts.google.com/o/oauth2/v2/auth?client_id={client_id}&redirect_uri={}&state={}&response_type=code&scope=openid%20profile%20email",
        urlencoded(&callback),
        params.user_code,
    );
    Redirect::temporary(&redirect_url).into_response()
}

#[derive(Deserialize)]
struct GoogleCallbackParams {
    code: String,
    state: String,
}

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
    Query(params): Query<GoogleCallbackParams>,
) -> impl IntoResponse {
    let flow = params.state.clone();
    let client = reqwest::Client::new();

    let (client_id, client_secret, base_url) = {
        let s = state.read().await;
        match (&s.google_client_id, &s.google_client_secret) {
            (Some(id), Some(secret)) => (id.clone(), secret.clone(), s.base_url.clone()),
            _ => return Html("<h1>Google not configured</h1>".to_string()).into_response(),
        }
    };

    let callback = format!("{base_url}/auth/google/callback");

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

    let auth_user = AuthUser {
        display_name: google_user.name.unwrap_or_else(|| email.clone()),
        email: email.clone(),
        avatar_url: google_user.picture,
        provider: "google".to_string(),
    };

    if flow == "web" {
        let s = state.read().await;
        let user_id = if let Some(ref pool) = s.db_pool {
            let _ = super::db::upsert_user(
                pool,
                &auth_user.email,
                &auth_user.display_name,
                auth_user.avatar_url.as_deref(),
            )
            .await;
            super::db::get_user_id(pool, &email)
                .await
                .unwrap_or_else(|_| email.clone())
        } else {
            email.clone()
        };
        return set_cookie_and_redirect(&user_id);
    }

    complete_auth(&state, &flow, auth_user)
        .await
        .into_response()
}

// -- Shared auth completion --

async fn complete_auth(state: &SharedState, user_code: &str, auth_user: AuthUser) -> Html<String> {
    let walker_token = generate_token(48);
    let display_name = auth_user.display_name.clone();
    let email = auth_user.email.clone();

    let mut state = state.write().await;
    let mut found = false;
    for auth in state.device_codes.values_mut() {
        if auth.user_code == user_code && auth.token.is_none() {
            auth.token = Some(walker_token.clone());
            auth.user = Some(auth_user.clone());
            found = true;
            break;
        }
    }

    if found {
        // Persist to DB if available, and get user's public ID.
        let user_id = if let Some(ref pool) = state.db_pool {
            let _ = super::db::upsert_user(
                pool,
                &email,
                &display_name,
                auth_user.avatar_url.as_deref(),
            )
            .await;
            let _ = super::db::store_token(pool, &walker_token, &email).await;
            super::db::get_user_id(pool, &email)
                .await
                .unwrap_or_else(|_| email.clone())
        } else {
            uuid::Uuid::new_v4().to_string()
        };

        // Register token in the shared map so /api/update can authenticate.
        let mut tokens = state.token_map.write().await;
        tokens.insert(
            walker_token,
            super::live::TokenUser {
                id: user_id,
                email,
                display_name: display_name.clone(),
                avatar_url: auth_user.avatar_url.clone(),
            },
        );
        drop(tokens);
        info!(user = %display_name, provider = %auth_user.provider, "User authenticated");
        Html(format!(
            r#"<!DOCTYPE html>
<html>
<head><title>Walker - Success</title>
<style>body {{ font-family: system-ui; max-width: 400px; margin: 80px auto; text-align: center; }}</style>
</head>
<body>
  <h1>Welcome, {display_name}!</h1>
  <p>You can close this tab and return to your terminal.</p>
</body>
</html>"#
        ))
    } else {
        Html("<h1>Invalid or expired code</h1>".to_string())
    }
}

// -- Web (dashboard) login --
// These redirect through OAuth and set a cookie, then redirect to /.

async fn web_github_redirect(State(state): State<SharedState>) -> impl IntoResponse {
    let state = state.read().await;
    let Some(client_id) = &state.github_client_id else {
        return Html("GitHub login not configured".to_string()).into_response();
    };
    // Same callback URL as CLI flow — state=web distinguishes the flows.
    let callback = format!("{}/auth/github/callback", state.base_url);
    let url = format!(
        "https://github.com/login/oauth/authorize?client_id={client_id}&redirect_uri={}&state=web&scope=read:user%20user:email",
        urlencoded(&callback),
    );
    Redirect::temporary(&url).into_response()
}

async fn web_google_redirect(State(state): State<SharedState>) -> impl IntoResponse {
    let state = state.read().await;
    let Some(client_id) = &state.google_client_id else {
        return Html("Google login not configured".to_string()).into_response();
    };
    let callback = format!("{}/auth/google/callback", state.base_url);
    let url = format!(
        "https://accounts.google.com/o/oauth2/v2/auth?client_id={client_id}&redirect_uri={}&state=web&response_type=code&scope=openid%20profile%20email",
        urlencoded(&callback),
    );
    Redirect::temporary(&url).into_response()
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

fn generate_user_code() -> String {
    let mut rng = rand::rng();
    let a: u32 = rng.random_range(0..10000);
    let b: u32 = rng.random_range(0..10000);
    format!("{a:04}-{b:04}")
}

fn urlencoded(s: &str) -> String {
    s.replace(':', "%3A").replace('/', "%2F")
}
