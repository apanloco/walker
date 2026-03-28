use std::time::Instant;

/// Inferred activity state — the truth about whether the user is actually walking.
/// Rules:
/// - Any step increase → immediately WALKING
/// - No step increase for 5 seconds → IDLE
pub struct ActivityTracker {
    last_step_total: u64,
    last_step_time: Option<Instant>,
    /// Total seconds the user was actually walking.
    active_secs: f64,
    /// Total seconds the treadmill was running but user was idle.
    idle_secs: f64,
    /// Timestamp of the last update.
    last_update: Option<Instant>,
    /// Current movement state.
    moving: bool,
}

const IDLE_TIMEOUT_SECS: f64 = 2.5;

/// Snapshot of the current activity state for display/reporting.
#[derive(Debug, Clone)]
pub struct ActivityState {
    pub moving: bool,
    pub active_duration_secs: u64,
    pub idle_duration_secs: u64,
}

impl ActivityTracker {
    pub fn new() -> Self {
        Self {
            last_step_total: 0,
            last_step_time: None,
            active_secs: 0.0,
            idle_secs: 0.0,
            last_update: None,
            moving: false,
        }
    }

    /// Call on reconnect so the next step reading is treated as fresh.
    pub fn on_reconnect(&mut self) {
        self.last_step_total = 0;
        self.last_step_time = None;
    }

    /// Update with new total step count (from StepTracker).
    pub fn update(&mut self, total_steps: u64, treadmill_running: bool) -> ActivityState {
        let now = Instant::now();

        // Did steps increase?
        if total_steps > self.last_step_total {
            self.moving = true;
            self.last_step_time = Some(now);
            self.last_step_total = total_steps;
        } else if let Some(last) = self.last_step_time {
            // No new steps — check timeout.
            if now.duration_since(last).as_secs_f64() >= IDLE_TIMEOUT_SECS {
                self.moving = false;
            }
        }

        // Track active vs idle time.
        if let Some(last_update) = self.last_update {
            let elapsed = now.duration_since(last_update).as_secs_f64();
            if treadmill_running {
                if self.moving {
                    self.active_secs += elapsed;
                } else {
                    self.idle_secs += elapsed;
                }
            }
        }

        self.last_update = Some(now);
        self.state()
    }

    pub fn state(&self) -> ActivityState {
        ActivityState {
            moving: self.moving,
            active_duration_secs: self.active_secs as u64,
            idle_duration_secs: self.idle_secs as u64,
        }
    }
}
