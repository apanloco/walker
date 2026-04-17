use colored::Colorize;
use uuid::Uuid;

use crate::activity::ActivityState;
use crate::device::{TreadmillData, TreadmillStatus};

pub fn hex_dump(data: &[u8]) -> String {
    data.iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn char_short_name(uuid: &Uuid) -> &'static str {
    match uuid.as_u128() {
        0x0000_2acd_0000_1000_8000_0080_5f9b_34fb => "FTMS Treadmill Data",
        0x0000_2ad3_0000_1000_8000_0080_5f9b_34fb => "FTMS Training Status",
        0x0000_2ada_0000_1000_8000_0080_5f9b_34fb => "FTMS Machine Status",
        0x0000_2acc_0000_1000_8000_0080_5f9b_34fb => "FTMS Machine Feature",
        0x0000_2ad9_0000_1000_8000_0080_5f9b_34fb => "FTMS Control Point",
        0x0000_fff1_0000_1000_8000_0080_5f9b_34fb => "UREVO Data",
        0x0000_fff2_0000_1000_8000_0080_5f9b_34fb => "UREVO Control",
        _ => "Unknown",
    }
}

pub fn print_walk_header() {
    // \r\n so rows align cleanly in raw mode (used when speed control is active).
    print!(
        "  {:<12} {:<10} {:<10} {:<10} {:<10} {:<8} {:<10} {:<10}\r\n",
        "ACTIVITY", "STATUS", "SPEED", "DURATION", "DISTANCE", "STEPS", "ACTIVE", "IDLE"
    );
    print!("  {}\r\n", "─".repeat(90));
}

/// Colorize a string after padding, so ANSI escape codes don't affect column width.
fn pad_color(text: &str, width: usize, color: colored::Color, bold: bool) -> String {
    let padded = format!("{text:<width$}");
    let colored = padded.color(color);
    if bold {
        colored.bold().to_string()
    } else {
        colored.to_string()
    }
}

fn pad_dimmed(text: &str, width: usize) -> String {
    format!("{text:<width$}").dimmed().to_string()
}

pub fn print_data_row(data: &TreadmillData, activity: &ActivityState) {
    let status_name = data.status.display_name();
    let duration = format!(
        "{:>3}:{:02}",
        data.duration_secs / 60,
        data.duration_secs % 60
    );
    let active = format!(
        "{:>3}:{:02}",
        activity.active_duration_secs / 60,
        activity.active_duration_secs % 60
    );
    let idle = format!(
        "{:>3}:{:02}",
        activity.idle_duration_secs / 60,
        activity.idle_duration_secs % 60
    );
    let speed = format!("{:.1} km/h", data.speed_kmh);
    let distance = format!("{:.2} km", data.distance_km);
    let steps = match data.steps {
        Some(s) => format!("{s}"),
        None => "—".to_string(),
    };

    use crate::activity::ActivityPhase;
    let activity_col = match activity.phase {
        ActivityPhase::Walking => pad_color("WALKING", 12, colored::Color::Green, true),
        ActivityPhase::Idle => pad_color("IDLE", 12, colored::Color::Red, true),
        ActivityPhase::Init => pad_dimmed("—", 12),
    };

    let status_col = if data.status == TreadmillStatus::Running {
        pad_color(status_name, 10, colored::Color::Green, false)
    } else {
        pad_color(status_name, 10, colored::Color::Yellow, false)
    };

    print!(
        "  {activity_col} {status_col} {:<10} {:<10} {:<10} {:<8} {:<10} {:<10}\r\n",
        speed, duration, distance, steps, active, idle,
    );
}

pub fn print_status_row(status: &TreadmillStatus, raw: &[u8]) {
    let status_col = pad_dimmed(status.display_name(), 10);
    print!(
        "  {:<12} {status_col} {:<10} {:<10} {:<10} {:<8} {:<10} {}\r\n",
        "—",
        "—",
        "—",
        "—",
        "—",
        "—",
        hex_dump(raw).dimmed(),
    );
}

pub fn print_unknown_row(label: &str, data: &[u8]) {
    print!(
        "  {:<25} ({:>2} bytes)  {}\r\n",
        label.yellow(),
        data.len(),
        hex_dump(data),
    );
}

pub fn print_other_notification(uuid: &Uuid, data: &[u8]) {
    let name = char_short_name(uuid);
    print!(
        "  {:<25} ({:>2} bytes)  {}\r\n",
        name.cyan(),
        data.len(),
        hex_dump(data),
    );
}

/// One-line feedback when the user changes the target speed.
/// Written as \r\n for raw-mode compatibility.
pub fn print_target_speed(target_kmh: f32) {
    print!(
        "  {}  Target speed set to: {:.1} km/h\r\n",
        "→".bold().yellow(),
        target_kmh
    );
}
