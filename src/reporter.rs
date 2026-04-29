use crate::activity::ActivityState;
use std::time::Instant;
use tokio::sync::mpsc;
use tracing::{debug, warn};

/// Sends updates to the walker server via HTTP POST. All sends are funneled
/// through a single sender task: at most one HTTP request is in flight at a
/// time. Reqwest's keep-alive pool reuses one TCP connection for sequential
/// requests, so the server receives our updates strictly in send order.
/// Concurrent in-flight POSTs (the previous fire-and-forget approach) could
/// arrive out of order on the server and race on the segments unique partial
/// index, producing duplicate-key warns and lost state changes.
pub struct ServerReporter {
    tx: mpsc::UnboundedSender<UpdateMsg>,
    last_sent: Option<SentState>,
    last_send_time: Option<Instant>,
    heartbeat_secs: f64,
    send_count: u64,
    count_start: Instant,
}

struct UpdateMsg {
    state: &'static str,
    speed_kmh: f64,
}

#[derive(Clone, PartialEq)]
struct SentState {
    state: &'static str,
    speed_kmh_x10: i32, // Compare at 0.1 resolution to avoid float issues.
}

impl ServerReporter {
    pub fn new(server_url: String, token: String) -> Self {
        let (tx, mut rx) = mpsc::unbounded_channel::<UpdateMsg>();
        let client = reqwest::Client::new();
        let url = format!("{server_url}/api/update");
        let auth = format!("Bearer {token}");

        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                let res = client
                    .post(&url)
                    .header("Authorization", &auth)
                    .json(&serde_json::json!({
                        "state": msg.state,
                        "speed_kmh": msg.speed_kmh,
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
            }
        });

        Self {
            tx,
            last_sent: None,
            last_send_time: None,
            heartbeat_secs: 1.0,
            send_count: 0,
            count_start: Instant::now(),
        }
    }

    /// Call on every data update. Only actually sends when needed.
    pub fn maybe_send(&mut self, activity: &ActivityState, speed_kmh: f32) {
        // Don't report during INIT phase — state is unconfirmed.
        if !activity.is_confirmed() {
            return;
        }
        let state_str = if activity.is_walking() {
            "walking"
        } else {
            "idle"
        };
        let current = SentState {
            state: state_str,
            speed_kmh_x10: (speed_kmh * 10.0) as i32,
        };

        let reason = match (&self.last_sent, &self.last_send_time) {
            // Never sent → send.
            (None, _) => Some("first"),
            // State changed → send immediately.
            (Some(prev), _) if *prev != current => Some("change"),
            // Heartbeat: enough time elapsed.
            (_, Some(last)) if last.elapsed().as_secs_f64() >= self.heartbeat_secs => {
                Some("heartbeat")
            }
            _ => None,
        };

        let Some(reason) = reason else {
            return;
        };

        // Log reason + elapsed since last send.
        let elapsed = self
            .last_send_time
            .map(|t| t.elapsed().as_secs_f64())
            .unwrap_or(0.0);
        if reason == "change" {
            let prev = self.last_sent.as_ref().unwrap();
            debug!(
                reason,
                elapsed_ms = format!("{:.0}", elapsed * 1000.0),
                prev_state = prev.state,
                prev_speed = prev.speed_kmh_x10,
                new_state = current.state,
                new_speed = current.speed_kmh_x10,
                "Reporter send"
            );
        } else {
            debug!(
                reason,
                elapsed_ms = format!("{:.0}", elapsed * 1000.0),
                "Reporter send"
            );
        }

        // Rate summary every 10 seconds.
        self.send_count += 1;
        let window = self.count_start.elapsed().as_secs_f64();
        if window >= 10.0 {
            debug!(
                sends = self.send_count,
                window_secs = format!("{:.1}", window),
                rate = format!("{:.1}", self.send_count as f64 / window),
                "Reporter rate"
            );
            self.send_count = 0;
            self.count_start = Instant::now();
        }

        self.last_sent = Some(current);
        self.last_send_time = Some(Instant::now());

        self.fire_send(state_str, speed_kmh as f64);
    }

    /// Send a "stopped" signal — treadmill went to standby/off.
    /// Skips if already sent "stopped" (dedup like maybe_send).
    pub fn send_stopped(&mut self) {
        let stopped = SentState {
            state: "stopped",
            speed_kmh_x10: 0,
        };
        if self.last_sent.as_ref() == Some(&stopped) {
            return;
        }
        self.last_sent = Some(stopped);
        self.last_send_time = Some(Instant::now());
        self.fire_send("stopped", 0.0);
    }

    fn fire_send(&self, state: &'static str, speed_kmh: f64) {
        // Non-blocking enqueue. The sender task drains and POSTs in order.
        let _ = self.tx.send(UpdateMsg { state, speed_kmh });
    }
}
