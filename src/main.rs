#![allow(clippy::collapsible_if)]

#[cfg(feature = "client")]
mod activity;
#[cfg(feature = "client")]
mod auth;
#[cfg(feature = "client")]
mod ble;
#[cfg(feature = "client")]
mod device;
#[cfg(feature = "client")]
mod display;
#[cfg(feature = "client")]
mod reporter;
#[cfg(feature = "server")]
mod server;

use clap::{Parser, Subcommand};
use std::io::{self, Write};

/// Writer that rewrites every `\n` to `\r\n` before passing it to stdout. We
/// install this on `tracing_subscriber` so log lines render correctly while
/// `walker walk` has the terminal in raw mode (where bare `\n` moves down a
/// row without returning to column 0). In cooked mode the extra `\r` is a
/// harmless no-op — the cursor is already at column 0.
struct CrlfStdout;

impl Write for CrlfStdout {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let stdout = io::stdout();
        let mut out = stdout.lock();
        let mut start = 0;
        for (i, &b) in buf.iter().enumerate() {
            if b == b'\n' {
                out.write_all(&buf[start..i])?;
                out.write_all(b"\r\n")?;
                start = i + 1;
            }
        }
        if start < buf.len() {
            out.write_all(&buf[start..])?;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        io::stdout().flush()
    }
}

#[cfg(feature = "client")]
use tracing::info;

#[cfg(feature = "client")]
const DEFAULT_SERVER: &str = "https://walker.akerud.se";
#[cfg(feature = "client")]
const DEV_SERVER: &str = "http://localhost:3000";

#[derive(Parser)]
#[command(name = "walker", about = "Bluetooth walking machine tracker")]
struct Cli {
    /// Log verbosity level (trace, debug, info, warn, error)
    #[arg(short, long, global = true)]
    verbose: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Scan for Bluetooth walking machine devices
    #[cfg(feature = "client")]
    Enumerate {
        /// Scan duration in seconds
        #[arg(short, long, default_value = "10")]
        timeout: u64,
    },
    /// Connect to a treadmill and dump its FTMS capabilities (read-only)
    #[cfg(feature = "client")]
    Probe {
        /// Scan duration in seconds when searching for the device
        #[arg(short, long, default_value = "10")]
        timeout: u64,
    },
    /// Connect to a walking machine and dump all data
    #[cfg(feature = "client")]
    Walk {
        /// Scan duration in seconds when searching for the device
        #[arg(short, long, default_value = "10")]
        timeout: u64,
        /// Use dev credentials
        #[arg(long)]
        dev: bool,
        /// Run without server connection (data not reported)
        #[arg(long)]
        offline: bool,
        /// After connecting, send a start command so the belt begins moving
        /// without pressing Play on the remote. Only safe when you are ready
        /// to walk — the belt will start moving at the treadmill's default speed.
        #[arg(long)]
        start: bool,
    },
    /// Simulate a treadmill (no BLE needed)
    #[cfg(feature = "client")]
    Simulate {
        /// Walking speed in km/h
        #[arg(short, long, default_value = "2.5")]
        speed: f32,
        /// Number of fake users to simulate
        #[arg(short, long, default_value = "1")]
        count: u32,
        /// Use dev credentials
        #[arg(long)]
        dev: bool,
    },
    /// Remove saved login credentials
    #[cfg(feature = "client")]
    Logout {
        /// Remove dev credentials instead of production
        #[arg(long)]
        dev: bool,
    },
    /// Authenticate with the walker server
    #[cfg(feature = "client")]
    Login {
        /// Walker server URL
        #[arg(short, long, default_value = DEFAULT_SERVER)]
        server: String,
        /// Use dev server (localhost:3000)
        #[arg(long)]
        dev: bool,
    },
    /// Set your weight (used for calorie calculations)
    #[cfg(feature = "client")]
    SetWeight {
        /// Weight in kg
        weight_kg: f32,
        /// Use dev credentials
        #[arg(long)]
        dev: bool,
    },
    /// Run the walker server
    #[cfg(feature = "server")]
    Listen {
        /// Port to listen on
        #[arg(short, long, default_value = "3000")]
        port: u16,
        /// Base URL for OAuth callbacks (default: http://localhost:<port>)
        #[arg(long, env = "WALKER_BASE_URL")]
        base_url: Option<String>,
        /// GitHub OAuth App client ID
        #[arg(long, env = "WALKER_GITHUB_CLIENT_ID")]
        github_client_id: Option<String>,
        /// GitHub OAuth App client secret
        #[arg(long, env = "WALKER_GITHUB_CLIENT_SECRET")]
        github_client_secret: Option<String>,
        /// Google OAuth client ID
        #[arg(long, env = "WALKER_GOOGLE_CLIENT_ID")]
        google_client_id: Option<String>,
        /// Google OAuth client secret
        #[arg(long, env = "WALKER_GOOGLE_CLIENT_SECRET")]
        google_client_secret: Option<String>,
        /// PostgreSQL connection string
        #[arg(long, env = "DATABASE_URL")]
        database_url: Option<String>,
        /// Dev mode: auto-create a test user (no OAuth needed)
        #[arg(long)]
        dev: bool,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let log_filter = cli
        .verbose
        .as_deref()
        .map(|v| format!("walker={v}"))
        .or_else(|| std::env::var("RUST_LOG").ok())
        .unwrap_or_else(|| "info".to_string());

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(&log_filter))
        .with_writer(|| CrlfStdout)
        .init();

    match cli.command {
        #[cfg(feature = "client")]
        Command::Enumerate { timeout } => enumerate(timeout).await?,
        #[cfg(feature = "client")]
        Command::Probe { timeout } => probe(timeout).await?,
        #[cfg(feature = "client")]
        Command::Walk {
            timeout,
            dev,
            offline,
            start,
        } => walk(timeout, dev, offline, start).await?,
        #[cfg(feature = "client")]
        Command::Simulate { speed, count, dev } => simulate(speed, count, dev).await?,
        #[cfg(feature = "client")]
        Command::SetWeight { weight_kg, dev } => set_weight(weight_kg, dev).await?,
        #[cfg(feature = "client")]
        Command::Logout { dev } => {
            auth::logout(dev)?;
        }
        #[cfg(feature = "client")]
        Command::Login { server, dev } => {
            let server = if dev { DEV_SERVER.to_string() } else { server };
            auth::login(&server, dev).await?;
        }
        #[cfg(feature = "server")]
        Command::Listen {
            port,
            base_url,
            github_client_id,
            github_client_secret,
            google_client_id,
            google_client_secret,
            database_url,
            dev,
        } => {
            server::run(server::ServerConfig {
                port,
                base_url: base_url.unwrap_or_else(|| format!("http://localhost:{port}")),
                github_client_id,
                github_client_secret,
                google_client_id,
                google_client_secret,
                database_url: database_url.or_else(|| {
                    dev.then(|| "postgres://postgres:walker@localhost/walker".to_string())
                }),
                dev,
            })
            .await?;
        }
    }

    Ok(())
}

#[cfg(feature = "client")]
async fn enumerate(timeout: u64) -> anyhow::Result<()> {
    use btleplug::api::Peripheral;
    use colored::Colorize;
    use device::default_registry;

    let registry = default_registry();
    let adapter = ble::get_adapter().await?;

    info!("Scanning for Bluetooth devices ({timeout} seconds)...");
    let peripherals = ble::scan(&adapter, timeout).await?;

    if peripherals.is_empty() {
        info!("No devices found");
        return Ok(());
    }

    println!();
    println!("  {:<25} {:<20} {:<10} SERVICES", "NAME", "ADDRESS", "RSSI");
    println!("  {}", "─".repeat(75));

    let mut treadmill_count = 0;

    for peripheral in &peripherals {
        let Some(props) = peripheral.properties().await? else {
            continue;
        };

        let name = props
            .local_name
            .clone()
            .unwrap_or_else(|| "Unknown".to_string());
        let address = props.address.to_string();
        let rssi = props.rssi.map_or("N/A".to_string(), |r| format!("{r} dBm"));
        let services: Vec<String> = props.services.iter().map(|u| format!("{u}")).collect();
        let services_str = if services.is_empty() {
            "—".to_string()
        } else {
            services.join(", ")
        };

        let matched_profile = registry.match_device(props.local_name.as_deref(), &props.services);

        let line = format!("  {name:<25} {address:<20} {rssi:<10} {services_str}");

        if let Some(profile) = matched_profile {
            treadmill_count += 1;
            println!(
                "{} {}",
                line.green().bold(),
                format!("[{}]", profile.name()).green()
            );
        } else {
            println!("{}", line.dimmed());
        }
    }

    println!();
    if treadmill_count > 0 {
        println!(
            "  {}",
            format!("Found {treadmill_count} walking machine(s)").green()
        );
    } else {
        println!(
            "  {}",
            "No walking machines found. Is your treadmill powered on?".yellow()
        );
    }
    println!();

    Ok(())
}

#[cfg(feature = "client")]
async fn probe(timeout: u64) -> anyhow::Result<()> {
    use btleplug::api::Peripheral;
    use colored::Colorize;
    use device::{
        FTMS_MACHINE_FEATURE_UUID, FTMS_SUPPORTED_INCLINATION_RANGE_UUID,
        FTMS_SUPPORTED_SPEED_RANGE_UUID, default_registry,
    };

    let registry = default_registry();
    let adapter = ble::get_adapter().await?;

    let (device, profile) = match ble::find_treadmill(&adapter, timeout, &registry).await? {
        Some(found) => found,
        None => {
            info!("No treadmill found");
            return Ok(());
        }
    };

    let props = device.properties().await.ok().flatten().unwrap_or_default();
    let device_name = props.local_name.clone();
    info!(name = %device_name.as_deref().unwrap_or("Unknown"), "Found, connecting...");
    device.connect().await?;
    device.discover_services().await?;

    println!();
    println!(
        "  {} {}",
        "Device:".bold(),
        profile.full_name(device_name.as_deref()).green().bold()
    );
    println!();

    // Speed range: min, max, min_increment — each u16 LE in 0.01 km/h units.
    if let Some(bytes) = read_char(&device, FTMS_SUPPORTED_SPEED_RANGE_UUID).await {
        if bytes.len() >= 6 {
            let min = u16::from_le_bytes([bytes[0], bytes[1]]) as f32 * 0.01;
            let max = u16::from_le_bytes([bytes[2], bytes[3]]) as f32 * 0.01;
            let inc = u16::from_le_bytes([bytes[4], bytes[5]]) as f32 * 0.01;
            println!("  Speed range:     {min:.2} – {max:.2} km/h  (min step: {inc:.2})");
        } else {
            println!("  Speed range:     <{} bytes> {}", bytes.len(), hex(&bytes));
        }
    } else {
        println!("  Speed range:     (not available)");
    }

    // Inclination range: min, max (i16 LE in 0.1%), min_increment (u16 LE in 0.1%).
    if let Some(bytes) = read_char(&device, FTMS_SUPPORTED_INCLINATION_RANGE_UUID).await {
        if bytes.len() >= 6 {
            let min = i16::from_le_bytes([bytes[0], bytes[1]]) as f32 * 0.1;
            let max = i16::from_le_bytes([bytes[2], bytes[3]]) as f32 * 0.1;
            let inc = u16::from_le_bytes([bytes[4], bytes[5]]) as f32 * 0.1;
            println!("  Incline range:   {min:.1} – {max:.1} %     (min step: {inc:.1})");
        } else {
            println!("  Incline range:   <{} bytes> {}", bytes.len(), hex(&bytes));
        }
    } else {
        println!("  Incline range:   (not available)");
    }

    if let Some(bytes) = read_char(&device, FTMS_MACHINE_FEATURE_UUID).await {
        println!("  Machine feature: {}", hex(&bytes));
    }

    println!();
    let _ = device.disconnect().await;
    Ok(())
}

#[cfg(feature = "client")]
async fn read_char(device: &btleplug::platform::Peripheral, uuid: uuid::Uuid) -> Option<Vec<u8>> {
    use btleplug::api::Peripheral;
    let services = device.services();
    let ch = services
        .iter()
        .flat_map(|s| &s.characteristics)
        .find(|c| c.uuid == uuid)?;
    device.read(ch).await.ok()
}

#[cfg(feature = "client")]
fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// RAII guard that puts the terminal in raw mode so we can capture arrow keys
/// without line buffering, and restores cooked mode on drop (including panics).
#[cfg(feature = "client")]
struct RawModeGuard;

#[cfg(feature = "client")]
impl RawModeGuard {
    fn enable() -> anyhow::Result<Self> {
        crossterm::terminal::enable_raw_mode()?;
        Ok(Self)
    }
}

#[cfg(feature = "client")]
impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

/// Why the inner walk loop exited.
#[cfg(feature = "client")]
enum WalkExit {
    /// BLE stream ended (device closed the connection).
    StreamEnded,
    /// No data received for the disconnect timeout.
    Timeout,
    /// User pressed Ctrl+C or 'q' — return all the way out.
    UserQuit,
}

#[cfg(feature = "client")]
async fn walk(timeout: u64, dev: bool, offline: bool, mut start: bool) -> anyhow::Result<()> {
    use btleplug::api::Peripheral;
    use colored::Colorize;
    use crossterm::event::{Event, EventStream, KeyCode, KeyModifiers};
    use futures::stream::StreamExt;
    use std::time::Duration;
    use tracing::{error, warn};

    use activity::ActivityTracker;
    use device::{StepTracker, TreadmillEvent, TreadmillStatus, default_registry};

    let registry = default_registry();
    let adapter = ble::get_adapter().await?;

    // Set up server reporter.
    let mut server_reporter = if offline {
        info!("Offline mode — data will not be reported to server");
        None
    } else {
        match auth::load(dev)? {
            Some(config) => {
                info!(
                    server = %config.server,
                    user = %config.display_name,
                    "Reporting to server"
                );
                Some(reporter::ServerReporter::new(config.server, config.token))
            }
            None => {
                anyhow::bail!(
                    "Not logged in. Run 'walker login' first, or use --offline to run without reporting."
                );
            }
        }
    };

    let mut step_tracker = StepTracker::new();
    let mut activity_tracker = ActivityTracker::new();
    let mut lines_since_header: u32 = 0;
    let mut last_display: Option<(
        TreadmillStatus,
        i32,
        u16,
        Option<u16>,
        activity::ActivityPhase,
    )> = None;
    let mut last_status_only: Option<TreadmillStatus> = None;

    loop {
        let (device, profile) = loop {
            match ble::find_treadmill(&adapter, timeout, &registry).await {
                Ok(Some(found)) => break found,
                Ok(None) => {
                    info!("{}", "No walking machine found yet. Retrying...".yellow());
                }
                Err(e) => {
                    error!(error = %e, "BLE scan failed, retrying in 3 seconds...");
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                }
            }
        };

        let props = device.properties().await.ok().flatten().unwrap_or_default();
        let device_name = props.local_name.clone();
        let address = props.address;
        info!(
            name = %device_name.as_deref().unwrap_or("Unknown"),
            address = %address,
            profile = profile.name(),
            "Found walking machine, connecting..."
        );

        if let Err(e) = device.connect().await {
            error!(error = %e, "Failed to connect, retrying in 3 seconds...");
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            continue;
        }

        // Set up the connection: discover services, subscribe, activate.
        // If any step fails, disconnect and retry from scanning.
        let stream: anyhow::Result<_> = async {
            device.discover_services().await?;
            let services = device.services();
            tracing::debug!("Discovered {} service(s)", services.len());

            for service in &services {
                let chars: Vec<String> = service
                    .characteristics
                    .iter()
                    .map(|c| {
                        let short = display::char_short_name(&c.uuid);
                        format!("    {} [{:?}] ({short})", c.uuid, c.properties)
                    })
                    .collect();
                tracing::debug!(uuid = %service.uuid, "\n{}", chars.join("\n"));
            }

            ble::subscribe_notify(&device, profile.notify_uuids()).await?;
            profile.activate(&device).await?;
            if start {
                // One-shot: consume regardless of outcome so later reconnects
                // (e.g. overnight power-cycle) don't silently restart the belt.
                start = false;
                match profile.start(&device).await {
                    Ok(()) => info!("Sent start command"),
                    Err(e) => warn!(error = %e, "Failed to send start command"),
                }
            }
            Ok(device.notifications().await?)
        }
        .await;

        let mut stream = match stream {
            Ok(s) => s,
            Err(e) => {
                error!(error = %e, "Connection setup failed, retrying in 3 seconds...");
                let _ = device.disconnect().await;
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                continue;
            }
        };

        // --- Connection banner ---
        let caps = profile.capabilities(device_name.as_deref());
        println!();
        println!(
            "  {} {}",
            "Connected to device:".bold(),
            profile.full_name(device_name.as_deref()).green().bold()
        );
        if caps.speed_control {
            let (min, max) = caps.speed_range_kmh;
            println!(
                "  Press {} / {} to change speed by 0.1 km/h ({min:.1}–{max:.1} km/h) — Ctrl+C or 'q' to stop",
                "↑".bold(),
                "↓".bold(),
            );
        } else {
            println!("  {}", "Listening for data — press Ctrl+C to stop".green());
        }
        println!();

        // Enter raw mode only if we need to capture keys. This swallows Ctrl+C
        // (it comes through as a KeyEvent we handle explicitly).
        let _raw_guard = if caps.speed_control {
            Some(RawModeGuard::enable()?)
        } else {
            None
        };
        let mut key_stream = EventStream::new();

        // Target speed the CLI tracks. Starts at 1.0 km/h as a safe fallback;
        // the data-stream sync below snaps it to the real belt-target on the
        // first Running packet and keeps it aligned whenever the user uses
        // the remote. The treadmill reports its *target* speed (not the
        // physical belt speed mid-ramp), so mirroring is immediate — except
        // for a short grace window after we issue a write, during which a
        // stale (pre-write) data packet can still arrive and would otherwise
        // undo our freshly-set target.
        let initial_target = 1.0_f32.clamp(caps.speed_range_kmh.0, caps.speed_range_kmh.1);
        let mut target_speed_kmh: f32 = initial_target;
        let mut last_set_speed_at: Option<std::time::Instant> = None;
        // Last seen device status. Arrow keys only send writes when this is
        // Running — the treadmill ignores speed commands in other states, and
        // sending them anyway just produces phantom "Target speed set to"
        // lines without effect.
        let mut last_status: Option<TreadmillStatus> = None;
        const SET_SPEED_GRACE: Duration = Duration::from_millis(750);

        let mut timeout_fut = Box::pin(tokio::time::sleep(Duration::from_secs(10)));

        let exit = loop {
            tokio::select! {
                notif = stream.next() => {
                    timeout_fut.as_mut().reset(
                        tokio::time::Instant::now() + Duration::from_secs(10),
                    );
                    let Some(notification) = notif else { break WalkExit::StreamEnded; };
                    tracing::trace!(
                        bytes = notification.value.len(),
                        uuid = %notification.uuid,
                        "BLE notification received",
                    );

                    match profile.parse_notification(&notification.uuid, &notification.value) {
                        Some(TreadmillEvent::Data(data)) => {
                            last_status_only = None;
                            last_status = Some(data.status);
                            // Mirror the device's reported target speed, except
                            // during a short grace window right after a write —
                            // the treadmill takes ~300ms to reflect the new
                            // target, so a stale packet could otherwise revert
                            // our target before the up-to-date one arrives.
                            if caps.speed_control
                                && data.status == TreadmillStatus::Running
                                && data.speed_kmh > 0.0
                            {
                                let in_grace = last_set_speed_at
                                    .map(|t| t.elapsed() < SET_SPEED_GRACE)
                                    .unwrap_or(false);
                                if !in_grace {
                                    target_speed_kmh = data
                                        .speed_kmh
                                        .clamp(caps.speed_range_kmh.0, caps.speed_range_kmh.1);
                                }
                            }
                            // Pausing/Paused = belt winding down. Reset activity
                            // tracking but keep the target — it still reflects
                            // the last commanded speed, which is informationally
                            // correct while paused.
                            if matches!(
                                data.status,
                                TreadmillStatus::Pausing | TreadmillStatus::Paused
                            ) {
                                step_tracker.reset();
                                activity_tracker.reset();
                                let activity = activity_tracker.state();
                                let key = (data.status, (data.speed_kmh * 10.0) as i32, data.duration_secs, data.steps, activity.phase);
                                if last_display.as_ref() != Some(&key) {
                                    if lines_since_header.is_multiple_of(20) {
                                        display::print_walk_header();
                                    }
                                    display::print_data_row(&data, &activity);
                                    lines_since_header += 1;
                                    last_display = Some(key);
                                }
                                if let Some(ref mut rpt) = server_reporter {
                                    rpt.send_stopped();
                                }
                                continue;
                            }
                            let step_change = data.steps.map(|raw| step_tracker.update(raw))
                                .unwrap_or(device::StepChange::Baseline);
                            let treadmill_running = data.status == TreadmillStatus::Running;
                            let activity =
                                activity_tracker.update(step_change, treadmill_running, data.speed_kmh);
                            let key = (data.status, (data.speed_kmh * 10.0) as i32, data.duration_secs, data.steps, activity.phase);
                            if last_display.as_ref() != Some(&key) {
                                if lines_since_header.is_multiple_of(20) {
                                    display::print_walk_header();
                                }
                                display::print_data_row(&data, &activity);
                                lines_since_header += 1;
                                last_display = Some(key);
                            }
                            if let Some(ref mut rpt) = server_reporter {
                                rpt.maybe_send(&activity, data.speed_kmh);
                            }
                        }
                        Some(TreadmillEvent::StatusOnly(status)) => {
                            last_display = None;
                            last_status = Some(status);
                            if let Some(ref mut rpt) = server_reporter {
                                rpt.send_stopped();
                            }
                            if matches!(status, TreadmillStatus::Standby | TreadmillStatus::Off) {
                                step_tracker.reset();
                                activity_tracker.reset();
                                target_speed_kmh = initial_target;
                            }
                            if last_status_only.as_ref() != Some(&status) {
                                display::print_status_row(&status, &notification.value);
                                last_status_only = Some(status);
                            }
                        }
                        Some(TreadmillEvent::CommandAck) => {
                            // Pure ack echo from a prior control write. Swallow.
                        }
                        Some(TreadmillEvent::Unknown { data, .. }) => {
                            display::print_unknown_row("UREVO ???", &data);
                        }
                        None => {
                            display::print_other_notification(&notification.uuid, &notification.value);
                        }
                    }
                }

                _ = &mut timeout_fut => {
                    break WalkExit::Timeout;
                }

                Some(Ok(Event::Key(ke))) = key_stream.next(), if caps.speed_control => {
                    match (ke.code, ke.modifiers) {
                        (KeyCode::Char('c'), m) if m.contains(KeyModifiers::CONTROL) => {
                            break WalkExit::UserQuit;
                        }
                        (KeyCode::Char('q'), _) => {
                            break WalkExit::UserQuit;
                        }
                        (KeyCode::Up, _) | (KeyCode::Down, _) => {
                            // The treadmill only accepts speed changes while
                            // Running. Writing in other states is silently
                            // ignored by the device — suppress the arrow so
                            // we don't print phantom "Target speed set" lines.
                            if last_status != Some(TreadmillStatus::Running) {
                                continue;
                            }
                            let step = if matches!(ke.code, KeyCode::Up) { 0.1 } else { -0.1 };
                            let (min, max) = caps.speed_range_kmh;
                            let new_target = (target_speed_kmh + step).clamp(min, max);
                            // Already at the boundary — skip write (no point
                            // beeping the treadmill) and the redundant print.
                            if new_target != target_speed_kmh {
                                target_speed_kmh = new_target;
                                display::print_target_speed(target_speed_kmh);
                                last_set_speed_at = Some(std::time::Instant::now());
                                if let Err(e) = profile.set_speed(&device, target_speed_kmh).await {
                                    warn!(error = %e, "Failed to set target speed");
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        };

        // Restore cooked mode before any further println/info output.
        drop(_raw_guard);

        // Immediate feedback on user-initiated quit — disconnect can take a
        // second, and without this the terminal looks frozen (tempting a second
        // Ctrl+C that would SIGINT the process mid-cleanup).
        if matches!(exit, WalkExit::UserQuit) {
            println!("  Disconnecting…");
        }

        step_tracker.reset();
        activity_tracker.reset();
        let _ = device.disconnect().await;

        match exit {
            WalkExit::UserQuit => return Ok(()),
            WalkExit::Timeout => {
                info!(
                    "{}",
                    "No data for 10 seconds — assuming disconnected".yellow()
                );
            }
            WalkExit::StreamEnded => {}
        }

        info!(
            "{}",
            "Device disconnected. Reconnecting in 3 seconds...".yellow()
        );
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}

#[cfg(feature = "client")]
async fn set_weight(weight_kg: f32, dev: bool) -> anyhow::Result<()> {
    let config = auth::load(dev)?
        .ok_or_else(|| anyhow::anyhow!("Not logged in. Run 'walker login' first."))?;

    let client = reqwest::Client::new();
    let res = client
        .put(format!("{}/api/weight", config.server))
        .header("Authorization", format!("Bearer {}", config.token))
        .json(&serde_json::json!({"weight_kg": weight_kg}))
        .send()
        .await?;

    if res.status().is_success() {
        println!("  Weight set to {weight_kg} kg");
    } else {
        anyhow::bail!("Server rejected weight update (status {})", res.status());
    }

    Ok(())
}

#[cfg(feature = "client")]
const SIM_NAMES: &[&str] = &[
    "alice", "bob", "charlie", "diana", "eve", "frank", "grace", "henry", "iris", "jack", "kate",
    "leo", "mia", "noah", "olivia", "paul", "quinn", "ruby", "sam", "tara",
];

#[cfg(feature = "client")]
async fn simulate(speed: f32, count: u32, dev: bool) -> anyhow::Result<()> {
    let config = auth::load(dev)?
        .ok_or_else(|| anyhow::anyhow!("Not logged in. Run 'walker login' first."))?;
    let client = reqwest::Client::new();
    let server = config.server.clone();

    if count <= 1 {
        let url = format!("{server}/api/update");
        let auth_header = format!("Bearer {}", config.token);
        info!(server = %server, user = %config.display_name, speed = %speed, "Simulating treadmill");
        println!("  Simulating: {speed} km/h — press Ctrl+C to stop");

        loop {
            let _ = client
                .post(&url)
                .header("Authorization", &auth_header)
                .json(&serde_json::json!({"state": "walking", "speed_kmh": speed}))
                .send()
                .await;
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    }

    let mut rng = rand::rng();
    let mut tokens = Vec::new();

    for i in 0..count {
        let name = SIM_NAMES[i as usize % SIM_NAMES.len()];
        let email = format!("{name}@walker.sim");

        let res = client
            .post(format!("{server}/api/simulate/register"))
            .json(&serde_json::json!({"name": name, "email": email}))
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        let token = res["token"].as_str().unwrap_or("").to_string();
        // Each user gets a distinct base speed: 1.5 to 8.0 km/h spread across users.
        let base_speed = 1.5 + (i as f32 / count.max(2) as f32) * 6.5;
        println!("  Registered: {name} ({email}) @ {base_speed:.1} km/h base");
        tokens.push((name.to_string(), token, base_speed));
    }

    println!("  Simulating {count} users — press Ctrl+C to stop");

    let url = format!("{server}/api/update");

    loop {
        for (name, token, base_speed) in &tokens {
            // Vary ±0.8 km/h around each user's base speed.
            let variation = rand::RngExt::random_range(&mut rng, -8..=8) as f32 * 0.1;
            let user_speed = (base_speed + variation).max(0.5);

            let _ = client
                .post(&url)
                .header("Authorization", format!("Bearer {token}"))
                .json(&serde_json::json!({"state": "walking", "speed_kmh": user_speed}))
                .send()
                .await;

            tracing::debug!(name = %name, speed = %user_speed, "Sent update");
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}
