use async_trait::async_trait;
use btleplug::api::{Peripheral, WriteType};
use tracing::info;
use uuid::Uuid;

use super::{TreadmillData, TreadmillEvent, TreadmillProfile, TreadmillStatus};

const UREVO_NOTIFY_UUID: Uuid = Uuid::from_u128(0x0000_fff1_0000_1000_8000_0080_5f9b_34fb);
const UREVO_WRITE_UUID: Uuid = Uuid::from_u128(0x0000_fff2_0000_1000_8000_0080_5f9b_34fb);
const UREVO_ACTIVATE_CMD: &[u8] = &[0x02, 0x51, 0x0B, 0x03];

const NAME_PREFIXES: &[&str] = &["URTM"];

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

pub struct UrevoProfile;

#[async_trait]
impl TreadmillProfile for UrevoProfile {
    fn name(&self) -> &str {
        "UREVO"
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
        let write_char = services
            .iter()
            .flat_map(|s| &s.characteristics)
            .find(|c| c.uuid == UREVO_WRITE_UUID);

        if let Some(ch) = write_char {
            info!("Activating UREVO proprietary data stream...");
            device
                .write(ch, UREVO_ACTIVATE_CMD, WriteType::WithoutResponse)
                .await?;
            info!("UREVO data stream activated");
        }

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
                speed_mph: data[3] as f32 * 0.1,
                duration_secs: u16::from_le_bytes([data[5], data[6]]),
                distance_km: u16::from_le_bytes([data[7], data[8]]) as f32 * 0.01,
                calories_kcal: u16::from_le_bytes([data[9], data[10]]) as f32 * 0.1,
                steps: Some(u16::from_le_bytes([data[11], data[12]])),
            }));
        }

        // Unrecognized UREVO packet.
        Some(TreadmillEvent::Unknown {
            uuid: *uuid,
            data: data.to_vec(),
        })
    }
}
