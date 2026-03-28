use serde::Serialize;
use std::collections::HashMap;
use std::time::Instant;

/// MET value for a given walking speed. Linearly interpolated.
fn met_for_speed_kmh(speed_kmh: f64) -> f64 {
    // MET table: 2→2.0, 3→2.5, 4→3.0, 5→3.5, 6→4.0
    let clamped = speed_kmh.clamp(0.0, 8.0);
    if clamped <= 2.0 {
        2.0
    } else if clamped >= 6.0 {
        4.0
    } else {
        2.0 + (clamped - 2.0) * 0.5
    }
}

fn mph_to_kmh(mph: f64) -> f64 {
    mph * 1.60934
}

/// Delta from a single update — written to DB as an increment.
pub struct StatsDelta {
    pub calories_ucal: u64,
    pub active_secs: u64,
    pub idle_secs: u64,
    pub distance_m: f64,
}

impl StatsDelta {
    pub const ZERO: Self = Self {
        calories_ucal: 0,
        active_secs: 0,
        idle_secs: 0,
        distance_m: 0.0,
    };

    pub fn is_empty(&self) -> bool {
        self.calories_ucal == 0 && self.active_secs == 0 && self.idle_secs == 0
    }
}

/// Per-user live state tracked by the server.
pub struct UserState {
    pub id: String,
    #[allow(dead_code)] // Used internally for DB lookups.
    pub email: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub moving: bool,
    pub speed_mph: f64,
    pub last_seen: Instant,
    pub weight_kg: f64,
    pub calories_ucal: u64,
    pub active_secs: u64,
    pub idle_secs: u64,
    /// Last computed distance delta (for broadcast to games).
    pub last_distance_delta_m: f64,
    /// Track time for computing deltas between updates.
    last_compute: Instant,
}

impl UserState {
    pub fn new(
        id: String,
        email: String,
        display_name: String,
        avatar_url: Option<String>,
        initial_calories_ucal: u64,
        initial_active_secs: u64,
        initial_idle_secs: u64,
    ) -> Self {
        let now = Instant::now();
        Self {
            id,
            email,
            display_name,
            avatar_url,
            moving: false,
            speed_mph: 0.0,
            last_seen: now,
            weight_kg: 70.0,
            calories_ucal: initial_calories_ucal,
            active_secs: initial_active_secs,
            idle_secs: initial_idle_secs,
            last_distance_delta_m: 0.0,
            last_compute: now,
        }
    }

    pub fn calories_kcal(&self) -> f64 {
        self.calories_ucal as f64 / 1_000_000.0
    }

    /// Compute calories and time since last update. Called on each incoming event.
    /// Returns the delta (calories_ucal, active_secs, idle_secs) for DB accumulation.
    pub fn compute_since_last(&mut self) -> StatsDelta {
        let now = Instant::now();
        let elapsed_secs = now.duration_since(self.last_compute).as_secs_f64();
        self.last_compute = now;

        if elapsed_secs <= 0.0 || elapsed_secs > 30.0 {
            return StatsDelta::ZERO;
        }

        if self.moving {
            let speed_kmh = mph_to_kmh(self.speed_mph);
            let met = met_for_speed_kmh(speed_kmh);
            let ucal = (met * self.weight_kg * 1_000_000.0 / 3600.0 * elapsed_secs) as u64;
            self.calories_ucal += ucal;
            let active = elapsed_secs as u64;
            self.active_secs += active;
            // distance in meters: speed_km/h × 1000 / 3600 × seconds
            let distance_m = speed_kmh * 1000.0 / 3600.0 * elapsed_secs;
            StatsDelta {
                calories_ucal: ucal,
                active_secs: active,
                idle_secs: 0,
                distance_m,
            }
        } else {
            let idle = elapsed_secs as u64;
            self.idle_secs += idle;
            StatsDelta {
                calories_ucal: 0,
                active_secs: 0,
                idle_secs: idle,
                distance_m: 0.0,
            }
        }
    }

    pub fn is_disconnected(&self) -> bool {
        self.last_seen.elapsed().as_secs() > 5
    }

    pub fn status(&self) -> &'static str {
        if self.is_disconnected() {
            "disconnected"
        } else if self.moving {
            "walking"
        } else {
            "idle"
        }
    }
}

#[derive(Serialize, Clone)]
pub struct LiveBroadcast {
    pub users: Vec<LiveUser>,
}

#[derive(Serialize, Clone)]
pub struct LiveUser {
    pub id: String,
    pub name: String,
    pub avatar_url: Option<String>,
    pub status: String,
    pub speed_mph: f64,
    pub calories_kcal: f64,
    pub distance_delta_m: f64,
    pub active_secs: u64,
    pub idle_secs: u64,
}

pub struct LiveState {
    pub users: HashMap<String, UserState>,
}

impl LiveState {
    pub fn new() -> Self {
        Self {
            users: HashMap::new(),
        }
    }

    /// Process an incoming update: compute calories, update state, return delta for DB.
    /// `initial_stats` seeds the in-memory state for new users (loaded from DB).
    #[allow(clippy::too_many_arguments)]
    pub fn process_update(
        &mut self,
        id: &str,
        email: &str,
        display_name: &str,
        avatar_url: Option<String>,
        moving: bool,
        speed_mph: f64,
        initial_stats: (u64, u64, u64),
    ) -> StatsDelta {
        let user = self.users.entry(email.to_string()).or_insert_with(|| {
            UserState::new(
                id.to_string(),
                email.to_string(),
                display_name.to_string(),
                avatar_url.clone(),
                initial_stats.0,
                initial_stats.1,
                initial_stats.2,
            )
        });

        // Compute calories for the period since the last update (at the OLD speed/state).
        let delta = user.compute_since_last();
        user.last_distance_delta_m = delta.distance_m;

        // Now update to the new state.
        user.moving = moving;
        user.speed_mph = speed_mph;
        user.last_seen = Instant::now();
        if avatar_url.is_some() {
            user.avatar_url = avatar_url;
        }

        delta
    }

    pub fn snapshot(&self) -> LiveBroadcast {
        let users = self
            .users
            .values()
            .map(|u| LiveUser {
                id: u.id.clone(),
                name: u.display_name.clone(),
                avatar_url: u.avatar_url.clone(),
                status: u.status().to_string(),
                speed_mph: u.speed_mph,
                calories_kcal: u.calories_kcal(),
                distance_delta_m: u.last_distance_delta_m,
                active_secs: u.active_secs,
                idle_secs: u.idle_secs,
            })
            .collect();
        LiveBroadcast { users }
    }
}
