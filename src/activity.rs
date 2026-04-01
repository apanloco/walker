use std::time::Instant;

/// Three-state activity detection:
///
/// INIT → WALKING → IDLE → WALKING → ...
///
/// - INIT: no confirmed state yet. Don't report to server.
/// - INIT → WALKING: first step increase detected.
/// - INIT → IDLE: impossible. Can't claim idle without first confirming walking.
/// - WALKING → IDLE: no step increase for IDLE_TIMEOUT_SECS.
/// - IDLE → WALKING: step increase detected.
/// - Any reset (pause/standby/reconnect) → INIT.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ActivityPhase {
    Init,
    Walking,
    Idle,
}

pub struct ActivityTracker {
    phase: ActivityPhase,
    last_step_total: Option<u64>,
    last_step_time: Option<Instant>,
    /// Total seconds the user was actually walking.
    active_secs: f64,
    /// Total seconds the treadmill was running but user was idle.
    idle_secs: f64,
    /// Timestamp of the last update.
    last_update: Option<Instant>,
}

/// Idle timeout based on current speed. Slower speeds need longer timeouts
/// because the treadmill's step sensor is less reliable at low speeds.
fn idle_timeout_secs(speed_kmh: f32) -> f64 {
    // Add entries here as needed. Must be sorted by speed ascending.
    const TABLE: &[(f32, f64)] = &[
        (1.5, 10.0),     // < 1.5 km/h: 10 seconds
        (2.0, 6.0),      // 1.5–2.0 km/h: 6 seconds
        (f32::MAX, 3.0), // >= 2.0 km/h: 3 seconds
    ];
    for &(threshold, timeout) in TABLE {
        if speed_kmh < threshold {
            return timeout;
        }
    }
    5.0
}

/// Snapshot of the current activity state for display/reporting.
#[derive(Debug, Clone)]
pub struct ActivityState {
    pub phase: ActivityPhase,
    pub active_duration_secs: u64,
    pub idle_duration_secs: u64,
}

impl ActivityState {
    /// Whether the user is confirmed walking. Use this for server reporting.
    pub fn is_walking(&self) -> bool {
        self.phase == ActivityPhase::Walking
    }

    /// Whether the state is confirmed (not INIT). Only report to server when true.
    pub fn is_confirmed(&self) -> bool {
        self.phase != ActivityPhase::Init
    }
}

impl ActivityTracker {
    pub fn new() -> Self {
        Self {
            phase: ActivityPhase::Init,
            last_step_total: None,
            last_step_time: None,
            active_secs: 0.0,
            idle_secs: 0.0,
            last_update: None,
        }
    }

    /// Reset to INIT state. Called on pause/standby/off/reconnect.
    pub fn reset(&mut self) {
        self.phase = ActivityPhase::Init;
        self.last_step_total = None;
        self.last_step_time = None;
    }

    /// Update with new total step count (from StepTracker).
    /// Pass None when no step data is available (before baseline is established).
    pub fn update(
        &mut self,
        total_steps: Option<u64>,
        treadmill_running: bool,
        speed_kmh: f32,
    ) -> ActivityState {
        let now = Instant::now();

        if let Some(steps) = total_steps {
            match self.last_step_total {
                Some(last) if steps > last => {
                    // Real step increase → WALKING (from any state, including INIT).
                    self.phase = ActivityPhase::Walking;
                    self.last_step_time = Some(now);
                }
                None => {
                    // First reading after reset — establish baseline, stay in current phase.
                }
                _ => {
                    // No increase — check idle timeout (only if already confirmed).
                    if self.phase == ActivityPhase::Walking {
                        if let Some(last_time) = self.last_step_time {
                            if now.duration_since(last_time).as_secs_f64()
                                >= idle_timeout_secs(speed_kmh)
                            {
                                self.phase = ActivityPhase::Idle;
                            }
                        }
                    }
                    // In INIT: stay in INIT. Cannot go INIT → IDLE.
                }
            }
            self.last_step_total = Some(steps);
        }

        // Track active vs idle time (only when confirmed).
        if let Some(last_update) = self.last_update {
            let elapsed = now.duration_since(last_update).as_secs_f64();
            if treadmill_running {
                match self.phase {
                    ActivityPhase::Walking => self.active_secs += elapsed,
                    ActivityPhase::Idle => self.idle_secs += elapsed,
                    ActivityPhase::Init => {} // Don't count time in INIT.
                }
            }
        }

        self.last_update = Some(now);
        self.state()
    }

    pub fn state(&self) -> ActivityState {
        ActivityState {
            phase: self.phase,
            active_duration_secs: self.active_secs as u64,
            idle_duration_secs: self.idle_secs as u64,
        }
    }
}
