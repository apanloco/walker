use axum::{
    Json, Router,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{info, warn};

use super::db;
use super::live::SharedLive;

pub struct StravaState {
    pub live: SharedLive,
}

pub type SharedStrava = Arc<StravaState>;

pub fn routes() -> Router<SharedStrava> {
    Router::new()
        .route("/auth/strava/connect", post(connect))
        .route("/auth/strava/disconnect", post(disconnect))
        .route("/strava-auth", get(strava_auth_helper))
        .route("/api/strava/sync", post(sync))
}

// -- Strava API types --

#[derive(Deserialize)]
struct StravaTokenResponse {
    access_token: String,
    refresh_token: String,
    expires_at: i64,
    athlete: Option<StravaAthlete>,
}

#[derive(Deserialize)]
struct StravaAthlete {
    id: i64,
    firstname: Option<String>,
}

// -- GET /strava-auth --
// Redirect target for Strava OAuth. After the user authorizes, Strava redirects
// here with ?code=XXX. This page posts the code to the opener (Walker profile
// page) via postMessage so the form can auto-submit. Falls back to displaying
// the code manually if there is no opener (e.g. direct navigation).

async fn strava_auth_helper(
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let code = params.get("code").map(|s| s.as_str()).unwrap_or("");
    let error = params.get("error").map(|s| s.as_str()).unwrap_or("");

    fn html_escape(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    }
    let code_attr = html_escape(code);
    let error_attr = html_escape(error);

    let body = format!(
        r#"<!DOCTYPE html><html><head><title>Strava Auth</title></head>
<body data-code="{code_attr}" data-error="{error_attr}" style="font-family:monospace;padding:2rem">
<p id="msg"></p>
<pre id="code-pre" style="background:#eee;padding:1rem;border-radius:4px;word-break:break-all;margin-top:0.5rem;display:none">{code_attr}</pre>
<script>
(function() {{
  var code = document.body.dataset.code;
  var error = document.body.dataset.error;
  var msg = document.getElementById('msg');
  var codePre = document.getElementById('code-pre');
  if (error) {{
    var span = document.createElement('span');
    span.style.color = 'red';
    var strong = document.createElement('strong');
    strong.textContent = error;
    span.textContent = 'Strava error: ';
    span.appendChild(strong);
    msg.appendChild(span);
    msg.appendChild(document.createElement('br'));
    msg.appendChild(document.createTextNode('Close this tab and try again.'));
    return;
  }}
  // Always show the code so the user can copy it manually if needed.
  codePre.style.display = '';
  msg.textContent = 'Copy this code and paste it into the Walker Connect form:';
  // Broadcast to any open Walker tab (same origin) then close this tab.
  // BroadcastChannel is more reliable than postMessage(opener) because
  // Strava's COOP headers sever the opener reference during OAuth.
  try {{
    var bc = new BroadcastChannel('strava-auth');
    bc.postMessage({{ stravaCode: code }});
    bc.close();
    msg.textContent = 'Connected! You can close this tab.';
    setTimeout(function() {{ window.close(); }}, 500);
  }} catch(e) {{
    // Fallback: try postMessage to opener for older browsers.
    if (window.opener && !window.opener.closed) {{
      try {{
        window.opener.postMessage({{ stravaCode: code }}, location.origin);
        setTimeout(function() {{ window.close(); }}, 500);
      }} catch(e2) {{}}
    }}
  }}
}})();
</script>
</body></html>"#
    );

    axum::response::Html(body)
}

// -- POST /auth/strava/connect --
// Users supply their own Strava API app credentials (from strava.com/settings/api)
// plus an authorization code obtained by completing the Strava OAuth flow with their app.
// Walker exchanges the code for tokens, verifies they work, and stores everything.

#[derive(Deserialize)]
struct ConnectBody {
    client_id: String,
    client_secret: String,
    code: String,
}

#[derive(Serialize)]
struct ConnectResponse {
    athlete_name: String,
}

async fn connect(
    State(state): State<SharedStrava>,
    headers: HeaderMap,
    Json(body): Json<ConnectBody>,
) -> impl IntoResponse {
    let Some(user_id) = super::cookie_user_id(&headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };

    let client = reqwest::Client::new();

    // Exchange the authorization code for tokens. The response includes the athlete
    // object with id and name — no separate /athlete API call needed.
    let token_res = match client
        .post("https://www.strava.com/api/v3/oauth/token")
        .form(&[
            ("client_id", body.client_id.as_str()),
            ("client_secret", body.client_secret.as_str()),
            ("code", body.code.as_str()),
            ("grant_type", "authorization_code"),
        ])
        .send()
        .await
    {
        Ok(r) => r,
        Err(_) => return (StatusCode::BAD_GATEWAY, "Failed to contact Strava").into_response(),
    };

    let token_data = match token_res.json::<StravaTokenResponse>().await {
        Ok(d) => d,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                "Invalid credentials or authorization code",
            )
                .into_response();
        }
    };

    let Some(athlete) = token_data.athlete else {
        return (
            StatusCode::BAD_GATEWAY,
            "Strava did not return athlete info",
        )
            .into_response();
    };

    if let Err(e) = db::upsert_strava_connection(
        &state.live.db_pool,
        user_id,
        athlete.id,
        &body.client_id,
        &body.client_secret,
        &token_data.access_token,
        &token_data.refresh_token,
        token_data.expires_at,
    )
    .await
    {
        tracing::error!(error = %e, "Failed to store Strava connection");
        return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
    }

    let name = athlete.firstname.clone().unwrap_or_default();
    info!(user = %user_id, athlete = %name, "Strava account connected");

    Json(ConnectResponse { athlete_name: name }).into_response()
}

// -- POST /auth/strava/disconnect --

async fn disconnect(State(state): State<SharedStrava>, headers: HeaderMap) -> impl IntoResponse {
    let Some(user_id) = super::cookie_user_id(&headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };

    if let Err(e) = db::delete_strava_connection(&state.live.db_pool, user_id).await {
        tracing::error!(error = %e, "Failed to delete Strava connection");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    info!(user = %user_id, "Strava account disconnected");
    StatusCode::OK.into_response()
}

// -- POST /api/strava/sync --

async fn sync(State(state): State<SharedStrava>, headers: HeaderMap) -> impl IntoResponse {
    let Some(user_id) = super::cookie_user_id(&headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };

    if db::get_strava_connection(&state.live.db_pool, user_id)
        .await
        .unwrap_or(None)
        .is_none()
    {
        return (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({"error": "Strava not connected"})),
        )
            .into_response();
    }

    let last_synced = db::get_strava_last_synced(&state.live.db_pool, user_id)
        .await
        .unwrap_or(None);
    let since = sync_after_for_user(&state, user_id, last_synced).await;
    match import_activities_since(&state, user_id, since).await {
        Ok(imported) => {
            if imported > 0 {
                let _ = state.live.broadcast_tx.send(());
            }
            axum::Json(serde_json::json!({"imported": imported})).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, user = %user_id, "Strava sync failed");
            (
                StatusCode::BAD_GATEWAY,
                axum::Json(serde_json::json!({"error": "Sync failed"})),
            )
                .into_response()
        }
    }
}

// -- Core import logic --

/// Refresh the Strava access token if it's about to expire, then return it.
/// Uses the per-user client_id and client_secret stored in strava_connections.
async fn fresh_token(state: &StravaState, user_id: uuid::Uuid) -> anyhow::Result<String> {
    let conn = db::get_strava_connection(&state.live.db_pool, user_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("No Strava connection for user"))?;

    if !conn.needs_refresh {
        return Ok(conn.access_token);
    }

    let client = reqwest::Client::new();
    let resp = client
        .post("https://www.strava.com/api/v3/oauth/token")
        .form(&[
            ("client_id", conn.client_id.as_str()),
            ("client_secret", conn.client_secret.as_str()),
            ("grant_type", "refresh_token"),
            ("refresh_token", conn.refresh_token.as_str()),
        ])
        .send()
        .await?
        .json::<StravaTokenResponse>()
        .await?;

    db::update_strava_tokens(
        &state.live.db_pool,
        user_id,
        &resp.access_token,
        &resp.refresh_token,
        resp.expires_at,
    )
    .await?;

    Ok(resp.access_token)
}

/// Fetch Walk, Hike, and treadmill-run activities since `after` (Unix timestamp) and import them.
/// Excluded: outdoor Run, TrailRun, rides, swims, and everything else.
/// Treadmill runs are identified by sport_type="Run"/"VirtualRun" with trainer=true,
/// or sport_type="VirtualRun" (always indoor).
/// Returns the count of newly imported segments.
async fn import_activities_since(
    state: &StravaState,
    user_id: uuid::Uuid,
    after: i64,
) -> anyhow::Result<usize> {
    let token = fresh_token(state, user_id).await?;
    let weight_kg = db::get_user_weight(&state.live.db_pool, user_id).await?;
    let client = reqwest::Client::new();
    let mut imported = 0;
    let mut page = 1u32;

    tracing::debug!(user = %user_id, after, "Strava sync: fetching activities");

    loop {
        let url = format!(
            "https://www.strava.com/api/v3/athlete/activities?after={after}&per_page=200&page={page}"
        );
        let resp = client.get(&url).bearer_auth(&token).send().await?;

        let status = resp.status();
        let body = resp.text().await?;
        tracing::debug!(user = %user_id, page, %status, "Strava API response");

        if !status.is_success() {
            anyhow::bail!("Strava API error {status}: {body}");
        }

        let activities: Vec<serde_json::Value> = serde_json::from_str(&body)?;
        let page_len = activities.len();
        tracing::debug!(user = %user_id, page, count = page_len, "Strava activities on page");

        for summary in &activities {
            let id = summary["id"].as_i64().unwrap_or(0);
            // sport_type is the modern field (more granular); fall back to type for older records.
            let sport_type = summary["sport_type"]
                .as_str()
                .or_else(|| summary["type"].as_str())
                .unwrap_or("");
            tracing::debug!(
                id,
                sport_type,
                start_date = summary["start_date"].as_str().unwrap_or(""),
                "Strava activity"
            );

            if !matches!(sport_type, "Walk" | "Hike" | "Run" | "VirtualRun") {
                tracing::debug!(id, sport_type, "Skipping activity");
                continue;
            }

            let moving_time = summary["moving_time"].as_i64().unwrap_or(0);
            let average_speed = summary["average_speed"].as_f64().unwrap_or(0.0);
            let distance = summary["distance"].as_f64().unwrap_or(0.0);

            if moving_time <= 0 || average_speed <= 0.0 || distance <= 0.0 {
                tracing::debug!(id, "Skipping activity with zero moving_time/speed/distance");
                continue;
            }

            // Fetch the detailed activity to get average_grade (not in the list response).
            let detail_url = format!("https://www.strava.com/api/v3/activities/{id}");
            let detail_resp = client.get(&detail_url).bearer_auth(&token).send().await?;
            let detail_status = detail_resp.status();
            let detail_body = detail_resp.text().await?;
            tracing::debug!(user = %user_id, id, %detail_status, "Strava detail API response");
            if !detail_status.is_success() {
                anyhow::bail!(
                    "Strava detail API error {detail_status} for activity {id}: {detail_body}"
                );
            }
            let detail: serde_json::Value = serde_json::from_str(&detail_body)?;
            tracing::debug!(id, average_grade = ?detail["average_grade"], "Strava activity detail");

            let start_date = summary["start_date"].as_str().unwrap_or("");
            let name = summary["name"].as_str();
            let source_url = format!("https://www.strava.com/activities/{id}");
            let speed_kmh = average_speed as f32 * 3.6;
            let incline_percent = detail["average_grade"].as_f64().map(|g| g as f32);

            let activity_id = db::upsert_imported_activity(
                &state.live.db_pool,
                "strava",
                &id.to_string(),
                name,
                Some(&source_url),
                &detail,
            )
            .await?;

            let inserted = db::insert_imported_segment(
                &state.live.db_pool,
                user_id,
                start_date,
                speed_kmh,
                incline_percent,
                moving_time as f32,
                distance as f32,
                weight_kg as f32,
                activity_id,
            )
            .await?;

            if inserted {
                imported += 1;
            }
        }

        if page_len < 200 {
            break;
        }
        page += 1;
    }

    if imported > 0 {
        let _ = db::update_strava_last_synced(&state.live.db_pool, user_id).await;
    }
    Ok(imported)
}

/// On startup, resync every connected user from their last known sync point.
/// Catches activities missed while the server was offline.
/// Safe to run unconditionally — import is idempotent (ON CONFLICT DO NOTHING).
pub async fn startup_sync(state: &StravaState) {
    let users = match db::get_strava_users_for_sync(&state.live.db_pool).await {
        Ok(u) => u,
        Err(e) => {
            warn!(error = %e, "Strava startup sync: failed to fetch connected users");
            return;
        }
    };

    if users.is_empty() {
        return;
    }

    info!(
        "Strava startup sync: checking {} connected user(s) for missed activities",
        users.len()
    );

    for (user_id, last_synced_unix) in users {
        let since = sync_after_for_user(state, user_id, last_synced_unix).await;

        match import_activities_since(state, user_id, since).await {
            Ok(n) if n > 0 => {
                info!(user = %user_id, imported = n, "Strava startup sync: caught up missed activities");
                let _ = state.live.broadcast_tx.send(());
            }
            Ok(_) => {}
            Err(e) => warn!(error = %e, user = %user_id, "Strava startup sync: failed for user"),
        }
    }
}

// -- Helpers --

fn unix_now() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

/// Determine the `after` timestamp for a Strava sync.
/// Uses the most recent Strava segment already in the DB (minus 1h buffer).
/// Falls back to `last_synced_unix` from the connection row if no segments exist.
/// Falls back to now (no backfill) if neither is available.
async fn sync_after_for_user(
    state: &StravaState,
    user_id: uuid::Uuid,
    last_synced_unix: Option<i64>,
) -> i64 {
    match db::get_latest_strava_segment_unix(&state.live.db_pool, user_id).await {
        Ok(Some(ts)) => ts.saturating_sub(3600),
        _ => last_synced_unix.unwrap_or_else(unix_now),
    }
}
