pub mod urevo;

use async_trait::async_trait;
use uuid::Uuid;

/// FTMS (Fitness Machine Service) UUID — standard BLE service for treadmills.
/// Not currently used; UREVO devices are controlled via their proprietary
/// protocol on 0xFFF2, not via FTMS.
#[allow(dead_code)]
pub const FTMS_SERVICE_UUID: Uuid = Uuid::from_u128(0x0000_1826_0000_1000_8000_0080_5f9b_34fb);

/// FTMS Machine Feature — bitmask of supported capabilities. Read-only.
pub const FTMS_MACHINE_FEATURE_UUID: Uuid =
    Uuid::from_u128(0x0000_2acc_0000_1000_8000_0080_5f9b_34fb);
/// FTMS Supported Speed Range — 6 bytes: min, max, min_increment (each u16 LE, 0.01 km/h units).
pub const FTMS_SUPPORTED_SPEED_RANGE_UUID: Uuid =
    Uuid::from_u128(0x0000_2ad4_0000_1000_8000_0080_5f9b_34fb);
/// FTMS Supported Inclination Range — 6 bytes: min, max (i16 LE, 0.1%), min_increment (u16 LE, 0.1%).
pub const FTMS_SUPPORTED_INCLINATION_RANGE_UUID: Uuid =
    Uuid::from_u128(0x0000_2ad5_0000_1000_8000_0080_5f9b_34fb);

/// What a connected treadmill supports in terms of runtime control.
/// Returned by `TreadmillProfile::capabilities()` after the device name is known,
/// so the same profile can describe different models (e.g. URTM041 vs URTM051).
#[derive(Debug, Clone, Copy)]
pub struct TreadmillCapabilities {
    pub speed_control: bool,
    /// Declared in the capability table for future incline controls (e.g. URTM051).
    /// Not yet consumed by the CLI.
    #[allow(dead_code)]
    pub incline_control: bool,
    /// Min/max target speed in km/h. Used to clamp arrow-key adjustments.
    pub speed_range_kmh: (f32, f32),
}

impl Default for TreadmillCapabilities {
    fn default() -> Self {
        Self {
            speed_control: false,
            incline_control: false,
            speed_range_kmh: (0.0, 0.0),
        }
    }
}

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
    pub speed_kmh: f32,
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
    /// Treadmill echoing back a command we sent — pure acknowledgement, no
    /// information beyond "I got your write". The walk loop ignores these.
    CommandAck,
    /// Unrecognized packet from a profile-owned characteristic.
    Unknown {
        #[allow(dead_code)]
        uuid: Uuid,
        data: Vec<u8>,
    },
}

// -- Step tracking --

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StepChange {
    Baseline,  // first reading after reset, no comparison yet
    Changed,   // raw value differs from previous
    Unchanged, // same as previous
}

pub struct StepTracker {
    prev_raw: Option<u16>,
}

impl StepTracker {
    pub fn new() -> Self {
        Self { prev_raw: None }
    }

    /// Call on reconnect — clears the previous raw value so the next reading
    /// becomes a new baseline.
    pub fn reset(&mut self) {
        self.prev_raw = None;
    }

    /// Feed a new raw step value. Returns whether it changed from the last reading.
    pub fn update(&mut self, raw_steps: u16) -> StepChange {
        match self.prev_raw {
            Some(prev) => {
                self.prev_raw = Some(raw_steps);
                if raw_steps != prev {
                    StepChange::Changed
                } else {
                    StepChange::Unchanged
                }
            }
            None => {
                self.prev_raw = Some(raw_steps);
                StepChange::Baseline
            }
        }
    }
}

// -- Profile trait --

#[async_trait]
pub trait TreadmillProfile: Send + Sync {
    /// Short profile name (e.g. "UREVO").
    fn name(&self) -> &str;

    /// Human-friendly model name for display (e.g. "UREVO SpaceWalk E1L").
    /// Derived from the BLE local_name when known. Falls back to `name()`.
    fn full_name(&self, _device_name: Option<&str>) -> String {
        self.name().to_string()
    }

    /// What runtime control this specific device supports.
    /// Takes the BLE local_name so one profile can describe multiple models.
    fn capabilities(&self, _device_name: Option<&str>) -> TreadmillCapabilities {
        TreadmillCapabilities::default()
    }

    /// Check if this profile matches a discovered device.
    fn matches(&self, device_name: Option<&str>, service_uuids: &[Uuid]) -> bool;

    /// Which characteristic UUIDs this profile needs notifications from.
    fn notify_uuids(&self) -> &[Uuid];

    /// After connection + service discovery, perform profile-specific activation
    /// (e.g., write commands to start a proprietary data stream, request FTMS control).
    async fn activate(&self, device: &btleplug::platform::Peripheral) -> anyhow::Result<()>;

    /// Send a target-speed command to the treadmill. Default implementation errors.
    /// Callers must check `capabilities().speed_control` first.
    async fn set_speed(
        &self,
        _device: &btleplug::platform::Peripheral,
        _speed_kmh: f32,
    ) -> anyhow::Result<()> {
        anyhow::bail!("set_speed is not supported by this profile")
    }

    /// Start a session — roughly "press Play on the remote". Opt-in; callers
    /// must only invoke when they're sure the belt is idle and the user is
    /// ready (moving belts are a safety concern). Default errors.
    async fn start(&self, _device: &btleplug::platform::Peripheral) -> anyhow::Result<()> {
        anyhow::bail!("start is not supported by this profile")
    }

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
