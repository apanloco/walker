pub mod urevo;

use async_trait::async_trait;
use uuid::Uuid;

/// FTMS (Fitness Machine Service) UUID — standard BLE service for treadmills.
pub const FTMS_SERVICE_UUID: Uuid = Uuid::from_u128(0x0000_1826_0000_1000_8000_0080_5f9b_34fb);

// -- Common types --

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreadmillStatus {
    Off,
    Standby,
    Starting,
    Running,
    Pausing,
    Paused,
    Unknown(u8),
}

impl TreadmillStatus {
    pub fn display_name(&self) -> &str {
        match self {
            Self::Off => "Off",
            Self::Standby => "Standby",
            Self::Starting => "Starting",
            Self::Running => "Running",
            Self::Pausing => "Pausing",
            Self::Paused => "Paused",
            Self::Unknown(_) => "Unknown",
        }
    }

    #[allow(dead_code)] // Will be used for session detection.
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Running | Self::Starting | Self::Pausing)
    }
}

#[derive(Debug, Clone)]
pub struct TreadmillData {
    pub status: TreadmillStatus,
    pub speed_mph: f32,
    pub duration_secs: u16,
    pub distance_km: f32,
    #[allow(dead_code)] // Will be used for server reporting.
    pub calories_kcal: f32,
    pub steps: Option<u16>,
}

#[derive(Debug, Clone)]
pub enum TreadmillEvent {
    /// Status-only update (e.g., short packets when off/standby).
    StatusOnly(TreadmillStatus),
    /// Full data update.
    Data(TreadmillData),
    /// Unrecognized packet from a profile-owned characteristic.
    Unknown {
        #[allow(dead_code)]
        uuid: Uuid,
        data: Vec<u8>,
    },
}

// -- Step tracking --

pub struct StepTracker {
    prev_raw: Option<u16>,
    pub total: u64,
    pub wrap_count: u32,
}

impl StepTracker {
    pub fn new() -> Self {
        Self {
            prev_raw: None,
            total: 0,
            wrap_count: 0,
        }
    }

    /// Call on reconnect — clears the previous raw value so the next reading
    /// becomes a new baseline without triggering a false wrap.
    pub fn on_reconnect(&mut self) {
        self.prev_raw = None;
    }

    /// Feed a new raw step value. Returns (total_steps, wrap_count).
    pub fn update(&mut self, raw_steps: u16) -> (u64, u32) {
        match self.prev_raw {
            Some(prev) if raw_steps < prev => {
                // Wrap detected.
                let wrap_at = 10000u64;
                self.total += wrap_at - prev as u64 + raw_steps as u64;
                self.wrap_count += 1;
                tracing::warn!(
                    prev_raw = prev,
                    new_raw = raw_steps,
                    wrap_count = self.wrap_count,
                    total_steps = self.total,
                    "Step counter wrapped!"
                );
            }
            Some(prev) => {
                self.total += (raw_steps - prev) as u64;
            }
            None => {
                self.total = raw_steps as u64;
            }
        }
        self.prev_raw = Some(raw_steps);
        (self.total, self.wrap_count)
    }
}

// -- Profile trait --

#[async_trait]
pub trait TreadmillProfile: Send + Sync {
    /// Human-readable name for this profile.
    fn name(&self) -> &str;

    /// Check if this profile matches a discovered device.
    fn matches(&self, device_name: Option<&str>, service_uuids: &[Uuid]) -> bool;

    /// After connection + service discovery, perform profile-specific activation
    /// (e.g., write commands to start a proprietary data stream).
    /// Generic subscribe-to-all-notify is handled by the caller.
    async fn activate(&self, device: &btleplug::platform::Peripheral) -> anyhow::Result<()>;

    /// Parse a BLE notification into a TreadmillEvent.
    /// Returns None if this profile does not handle the given characteristic.
    fn parse_notification(&self, uuid: &Uuid, data: &[u8]) -> Option<TreadmillEvent>;
}

// -- Profile registry --

pub struct ProfileRegistry {
    profiles: Vec<Box<dyn TreadmillProfile>>,
}

impl ProfileRegistry {
    pub fn new() -> Self {
        Self {
            profiles: Vec::new(),
        }
    }

    pub fn register(&mut self, profile: Box<dyn TreadmillProfile>) {
        self.profiles.push(profile);
    }

    /// Find the first profile that matches the given device.
    pub fn match_device(
        &self,
        device_name: Option<&str>,
        service_uuids: &[Uuid],
    ) -> Option<&dyn TreadmillProfile> {
        self.profiles
            .iter()
            .find(|p| p.matches(device_name, service_uuids))
            .map(|p| p.as_ref())
    }
}

/// Create a registry pre-loaded with all known treadmill profiles.
pub fn default_registry() -> ProfileRegistry {
    let mut registry = ProfileRegistry::new();
    registry.register(Box::new(urevo::UrevoProfile));
    registry
}
