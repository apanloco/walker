use crate::activity::ActivityState;
use std::time::Instant;
use tracing::{debug, warn};

/// Sends updates to the walker server via HTTP POST.
/// Only sends on state changes or every heartbeat_interval seconds.
pub struct ServerReporter {
    client: reqwest::Client,
    server_url: String,
    token: String,
    last_sent: Option<SentState>,
    last_send_time: Option<Instant>,
    heartbeat_secs: f64,
}

#[derive(Clone, PartialEq)]
struct SentState {
    moving: bool,
    speed_mph_x10: i32, // Compare at 0.1 resolution to avoid float issues.
}

impl ServerReporter {
    pub fn new(server_url: String, token: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            server_url,
            token,
            last_sent: None,
            last_send_time: None,
            heartbeat_secs: 1.0,
        }
    }

    /// Call on every data update. Only actually sends when needed.
    pub fn maybe_send(&mut self, activity: &ActivityState, speed_mph: f32) {
        let current = SentState {
            moving: activity.moving,
            speed_mph_x10: (speed_mph * 10.0) as i32,
        };

        let should_send = match (&self.last_sent, &self.last_send_time) {
            // Never sent → send.
            (None, _) => true,
            // State changed → send immediately.
            (Some(prev), _) if *prev != current => true,
            // Heartbeat: walking and enough time elapsed.
            (_, Some(last)) if activity.moving => {
                last.elapsed().as_secs_f64() >= self.heartbeat_secs
            }
            _ => false,
        };

        if !should_send {
            return;
        }

        self.last_sent = Some(current);
        self.last_send_time = Some(Instant::now());

        let url = format!("{}/api/update", self.server_url);
        let token = self.token.clone();
        let moving = activity.moving;
        let speed = speed_mph as f64;
        let client = self.client.clone();

        // Fire-and-forget — don't block the BLE loop.
        tokio::spawn(async move {
            let res = client
                .post(&url)
                .header("Authorization", format!("Bearer {token}"))
                .json(&serde_json::json!({
                    "moving": moving,
                    "speed_mph": speed,
                }))
                .send()
                .await;

            match res {
                Ok(r) if r.status().is_success() => {
                    debug!("Update sent to server");
                }
                Ok(r) => {
                    warn!(status = %r.status(), "Server rejected update");
                }
                Err(e) => {
                    warn!(error = %e, "Failed to send update to server");
                }
            }
        });
    }
}
