use async_trait::async_trait;
use btleplug::api::{Peripheral, WriteType};
use tracing::debug;
use uuid::Uuid;

use super::{TreadmillCapabilities, TreadmillData, TreadmillEvent, TreadmillProfile, TreadmillStatus};

const UREVO_NOTIFY_UUID: Uuid = Uuid::from_u128(0x0000_fff1_0000_1000_8000_0080_5f9b_34fb);
const UREVO_WRITE_UUID: Uuid = Uuid::from_u128(0x0000_fff2_0000_1000_8000_0080_5f9b_34fb);
const UREVO_ACTIVATE_CMD: &[u8] = &[0x02, 0x51, 0x0B, 0x03];

const NAME_PREFIXES: &[&str] = &["URTM"];

/// XOR key used in the UREVO proprietary frame checksum.
const UREVO_CHECKSUM_XOR: u8 = 0x5A;
/// Sub-command byte for "start session" (best guess from packet capture).
const UREVO_SUBCMD_START: u8 = 0x01;
/// Sub-command byte for "set target speed".
const UREVO_SUBCMD_SET_SPEED: u8 = 0x02;

/// Build the proprietary "start session" frame:
/// `02 53 01 00 00 00 00 00 00 00 00 0e 03`
/// The 8 zero data bytes are presumed workout targets (time/distance/calories),
/// all zero = no target. Captured verbatim from the iOS app.
fn build_start_cmd() -> [u8; 13] {
    // sum(53 + 01 + 0*8) = 0x54 → XOR 0x5A = 0x0e.
    [
        0x02,
        0x53,
        UREVO_SUBCMD_START,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0x0e,
        0x03,
    ]
}

/// Build a proprietary speed-set frame for FFF2:
/// `02 53 02 <u16 LE speed in 0.1 km/h> <checksum> 03`
/// where checksum = sum(all middle bytes) XOR 0x5A.
/// Protocol reverse-engineered from a PacketLogger capture of the UREVO iOS
/// app — documented under "Supported Devices" in CLAUDE.md.
fn build_set_speed_cmd(speed_kmh: f32) -> [u8; 7] {
    let raw = (speed_kmh * 10.0).round().clamp(0.0, u16::MAX as f32) as u16;
    let lo = (raw & 0xFF) as u8;
    let hi = (raw >> 8) as u8;
    let sum: u8 = 0x53u8
        .wrapping_add(UREVO_SUBCMD_SET_SPEED)
        .wrapping_add(lo)
        .wrapping_add(hi);
    let checksum = sum ^ UREVO_CHECKSUM_XOR;
    [0x02, 0x53, UREVO_SUBCMD_SET_SPEED, lo, hi, checksum, 0x03]
}

fn parse_status(byte: u8) -> TreadmillStatus {
    match byte {
        0x00 => TreadmillStatus::Standby,
        0x02 => TreadmillStatus::Starting,
        0x03 => TreadmillStatus::Running,
        0x04 => TreadmillStatus::Pausing,
        0x06 => TreadmillStatus::Off,
        0x0A => TreadmillStatus::Paused,
        other => TreadmillStatus::Unknown(other),
    }
}

/// Identify a UREVO model from its BLE local_name.
/// New models go here — add capabilities to `capabilities_for` below.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Model {
    /// URTM041 — UREVO SpaceWalk E1L. Speed control only, no incline.
    SpaceWalkE1L,
    /// URTM051 — UREVO CyberPad. Speed and incline control.
    CyberPad,
    /// Unrecognized URTM device. Parsing works (shared protocol) but no control.
    Unknown,
}

fn model_from_name(device_name: Option<&str>) -> Model {
    let Some(name) = device_name else {
        return Model::Unknown;
    };
    let upper = name.to_uppercase();
    if upper.starts_with("URTM041") {
        Model::SpaceWalkE1L
    } else if upper.starts_with("URTM051") {
        Model::CyberPad
    } else {
        Model::Unknown
    }
}

fn model_full_name(model: Model) -> &'static str {
    match model {
        Model::SpaceWalkE1L => "UREVO SpaceWalk E1L",
        Model::CyberPad => "UREVO CyberPad",
        Model::Unknown => "UREVO (unknown model)",
    }
}

fn capabilities_for(model: Model) -> TreadmillCapabilities {
    match model {
        // Range verified via `walker probe` against a real E1L:
        // FTMS Supported Speed Range reports 1.00 – 6.00 km/h.
        Model::SpaceWalkE1L => TreadmillCapabilities {
            speed_control: true,
            incline_control: false,
            speed_range_kmh: (1.0, 6.0),
        },
        // CyberPad range not yet probed; 1.0-6.0 is a conservative guess matching
        // the E1L. Verify with `walker probe` when a CyberPad is available.
        Model::CyberPad => TreadmillCapabilities {
            speed_control: true,
            incline_control: true,
            speed_range_kmh: (1.0, 6.0),
        },
        Model::Unknown => TreadmillCapabilities::default(),
    }
}

pub struct UrevoProfile;

#[async_trait]
impl TreadmillProfile for UrevoProfile {
    fn name(&self) -> &str {
        "UREVO"
    }

    fn full_name(&self, device_name: Option<&str>) -> String {
        match device_name {
            Some(n) => format!("{} ({})", model_full_name(model_from_name(device_name)), n),
            None => model_full_name(Model::Unknown).to_string(),
        }
    }

    fn capabilities(&self, device_name: Option<&str>) -> TreadmillCapabilities {
        capabilities_for(model_from_name(device_name))
    }

    fn notify_uuids(&self) -> &[Uuid] {
        &[UREVO_NOTIFY_UUID]
    }

    fn matches(&self, device_name: Option<&str>, _service_uuids: &[Uuid]) -> bool {
        // Only match by name prefix — FTMS UUID alone is too broad (matches bikes, rowers, etc.)
        if let Some(name) = device_name {
            let upper = name.to_uppercase();
            return NAME_PREFIXES.iter().any(|prefix| upper.starts_with(prefix));
        }
        false
    }

    async fn activate(&self, device: &btleplug::platform::Peripheral) -> anyhow::Result<()> {
        let services = device.services();

        // Start the UREVO proprietary data stream.
        if let Some(ch) = services
            .iter()
            .flat_map(|s| &s.characteristics)
            .find(|c| c.uuid == UREVO_WRITE_UUID)
        {
            debug!("Activating UREVO proprietary data stream...");
            device
                .write(ch, UREVO_ACTIVATE_CMD, WriteType::WithoutResponse)
                .await?;
            debug!("UREVO data stream activated");
        }

        // IMPORTANT: we intentionally do NOT write FTMS Request Control here.
        // On the E1L, doing so makes the treadmill stop emitting on the proprietary
        // 0xFFF1 stream (it switches to FTMS mode), which breaks read-only tracking.
        // Request Control (and set_speed) must be opt-in — only when the user
        // explicitly asks for speed control — and must be preceded by an observed
        // Running state with a real belt speed.

        Ok(())
    }

    async fn set_speed(
        &self,
        device: &btleplug::platform::Peripheral,
        speed_kmh: f32,
    ) -> anyhow::Result<()> {
        let services = device.services();
        let ch = services
            .iter()
            .flat_map(|s| &s.characteristics)
            .find(|c| c.uuid == UREVO_WRITE_UUID)
            .ok_or_else(|| anyhow::anyhow!("UREVO write characteristic not found"))?;
        let cmd = build_set_speed_cmd(speed_kmh);
        device.write(ch, &cmd, WriteType::WithoutResponse).await?;
        Ok(())
    }

    async fn start(&self, device: &btleplug::platform::Peripheral) -> anyhow::Result<()> {
        let services = device.services();
        let ch = services
            .iter()
            .flat_map(|s| &s.characteristics)
            .find(|c| c.uuid == UREVO_WRITE_UUID)
            .ok_or_else(|| anyhow::anyhow!("UREVO write characteristic not found"))?;

        // Replay the iOS app's handshake observed via PacketLogger:
        //   1. Send `02 50 03 09 03` (a short query — the device replies but the
        //      payload doesn't matter to us; the treadmill's state machine seems
        //      to require this exchange before it will accept the start command).
        //   2. Pause so the device can process.
        //   3. Send the actual start: `02 53 01 00...0e 03`.
        // Without the query step, the treadmill accepts the write but ignores it.
        let query: [u8; 5] = [0x02, 0x50, 0x03, 0x09, 0x03];
        device.write(ch, &query, WriteType::WithoutResponse).await?;
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let cmd = build_start_cmd();
        device.write(ch, &cmd, WriteType::WithoutResponse).await?;
        Ok(())
    }

    fn parse_notification(&self, uuid: &Uuid, data: &[u8]) -> Option<TreadmillEvent> {
        if *uuid != UREVO_NOTIFY_UUID {
            return None;
        }

        // Short packets (6 bytes): status-only when treadmill is off/standby/starting.
        if data.len() == 6 && data[0] == 0x02 && data[1] == 0x51 {
            return Some(TreadmillEvent::StatusOnly(parse_status(data[2])));
        }

        // Full data packets (19 bytes): active session data.
        if data.len() == 19 && data[0] == 0x02 && data[1] == 0x51 {
            return Some(TreadmillEvent::Data(TreadmillData {
                status: parse_status(data[2]),
                speed_kmh: data[3] as f32 * 0.1,
                duration_secs: u16::from_le_bytes([data[5], data[6]]),
                distance_km: u16::from_le_bytes([data[7], data[8]]) as f32 * 0.01,
                calories_kcal: u16::from_le_bytes([data[9], data[10]]) as f32 * 0.1,
                steps: Some(u16::from_le_bytes([data[11], data[12]])),
            }));
        }

        // Command acknowledgement: treadmill echoes `02 53 …` frames back on the
        // notify channel when it's accepted one of our control writes. No data
        // for us beyond "I got it" — suppress instead of printing as unknown.
        if data.len() >= 4 && data[0] == 0x02 && data[1] == 0x53 && data[data.len() - 1] == 0x03 {
            return Some(TreadmillEvent::CommandAck);
        }

        // Unrecognized UREVO packet.
        Some(TreadmillEvent::Unknown {
            uuid: *uuid,
            data: data.to_vec(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ground truth from the PacketLogger capture of the UREVO iOS app:
    /// each (kmh, bytes) pair was a write the real app made to 0xFFF2.
    #[test]
    fn set_speed_cmd_matches_app_capture() {
        assert_eq!(build_set_speed_cmd(1.0), [0x02, 0x53, 0x02, 0x0a, 0x00, 0x05, 0x03]);
        assert_eq!(build_set_speed_cmd(1.1), [0x02, 0x53, 0x02, 0x0b, 0x00, 0x3a, 0x03]);
        assert_eq!(build_set_speed_cmd(1.2), [0x02, 0x53, 0x02, 0x0c, 0x00, 0x3b, 0x03]);
        assert_eq!(build_set_speed_cmd(1.3), [0x02, 0x53, 0x02, 0x0d, 0x00, 0x38, 0x03]);
    }
}
