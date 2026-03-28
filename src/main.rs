mod activity;
mod auth;
mod ble;
mod device;
mod display;
mod reporter;
mod server;

use btleplug::api::Peripheral;
use clap::{Parser, Subcommand};
use colored::Colorize;
use futures::stream::StreamExt;
use tracing::{error, info};

use activity::ActivityTracker;
use device::{StepTracker, TreadmillEvent, TreadmillStatus, default_registry};

const DEFAULT_SERVER: &str = "http://localhost:3000";

#[derive(Parser)]
#[command(name = "walker", about = "Bluetooth walking machine tracker")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Scan for Bluetooth walking machine devices
    Enumerate {
        /// Scan duration in seconds
        #[arg(short, long, default_value = "10")]
        timeout: u64,
    },
    /// Connect to a walking machine and dump all data
    Walk {
        /// Scan duration in seconds when searching for the device
        #[arg(short, long, default_value = "10")]
        timeout: u64,
    },
    /// Simulate a treadmill (no BLE needed)
    Simulate {
        /// Walking speed in mph
        #[arg(short, long, default_value = "2.5")]
        speed: f32,
        /// Number of fake users to simulate
        #[arg(short, long, default_value = "1")]
        count: u32,
    },
    /// Authenticate with the walker server
    Login {
        /// Walker server URL
        #[arg(short, long, default_value = DEFAULT_SERVER)]
        server: String,
        /// Use dev mode token (no OAuth, requires server started with --dev)
        #[arg(long)]
        dev: bool,
    },
    /// Run the walker server
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
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Command::Enumerate { timeout } => enumerate(timeout).await?,
        Command::Walk { timeout } => walk(timeout).await?,
        Command::Simulate { speed, count } => simulate(speed, count).await?,
        Command::Login { server, dev } => {
            if dev {
                let config = auth::AuthConfig {
                    server,
                    token: "dev-token-walker".to_string(),
                    email: "dev@walker.local".to_string(),
                    display_name: "Dev User".to_string(),
                };
                auth::save(&config)?;
                println!("  Logged in as Dev User (dev mode)");
            } else {
                auth::login(&server).await?;
            }
        }
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
                database_url,
                dev,
            })
            .await?;
        }
    }

    Ok(())
}

async fn enumerate(timeout: u64) -> anyhow::Result<()> {
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

async fn walk(timeout: u64) -> anyhow::Result<()> {
    let registry = default_registry();
    let adapter = ble::get_adapter().await?;

    // Set up server reporter if logged in.
    let mut server_reporter = match auth::load()? {
        Some(config) => {
            info!(
                server = %config.server,
                user = %config.display_name,
                "Reporting to server"
            );
            Some(reporter::ServerReporter::new(config.server, config.token))
        }
        None => {
            info!("Not logged in — running offline (use 'walker login' to connect to a server)");
            None
        }
    };

    // Step tracker and activity tracker survive reconnections.
    let mut step_tracker = StepTracker::new();
    let mut activity_tracker = ActivityTracker::new();
    let mut lines_since_header: u32 = 0;

    loop {
        let (device, profile) = loop {
            if let Some(found) = ble::find_treadmill(&adapter, timeout, &registry).await? {
                break found;
            }
            info!("{}", "No walking machine found yet. Retrying...".yellow());
        };

        let props = device.properties().await?.unwrap_or_default();
        let name = props.local_name.as_deref().unwrap_or("Unknown");
        let address = props.address;
        info!(
            name = %name,
            address = %address,
            profile = profile.name(),
            "Found walking machine, connecting..."
        );

        if let Err(e) = device.connect().await {
            error!(error = %e, "Failed to connect, retrying in 3 seconds...");
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            continue;
        }
        info!("Connected!");

        device.discover_services().await?;
        let services = device.services();
        info!("Discovered {} service(s)", services.len());

        for service in &services {
            let chars: Vec<String> = service
                .characteristics
                .iter()
                .map(|c| {
                    let short = display::char_short_name(&c.uuid);
                    format!("    {} [{:?}] ({short})", c.uuid, c.properties)
                })
                .collect();
            info!(uuid = %service.uuid, "\n{}", chars.join("\n"));
        }

        ble::subscribe_all_notify(&device).await?;
        profile.activate(&device).await?;

        info!("{}", "Listening for data — press Ctrl+C to stop".green());
        if lines_since_header == 0 {
            println!();
        }

        let mut stream = device.notifications().await?;

        loop {
            let notification =
                match tokio::time::timeout(std::time::Duration::from_secs(10), stream.next()).await
                {
                    Ok(Some(n)) => n,
                    Ok(None) => break, // Stream ended cleanly.
                    Err(_) => {
                        info!(
                            "{}",
                            "No data for 10 seconds — assuming disconnected".yellow()
                        );
                        break;
                    }
                };
            if lines_since_header.is_multiple_of(20) {
                display::print_walk_header();
            }

            match profile.parse_notification(&notification.uuid, &notification.value) {
                Some(TreadmillEvent::Data(data)) => {
                    if let Some(raw_steps) = data.steps {
                        step_tracker.update(raw_steps);
                    }
                    let treadmill_running = data.status == TreadmillStatus::Running;
                    let activity = activity_tracker.update(step_tracker.total, treadmill_running);
                    display::print_data_row(&data, step_tracker.total, &activity);
                    if let Some(ref mut rpt) = server_reporter {
                        rpt.maybe_send(&activity, data.speed_mph);
                    }
                }
                Some(TreadmillEvent::StatusOnly(status)) => {
                    display::print_status_row(&status, &notification.value);
                }
                Some(TreadmillEvent::Unknown { data, .. }) => {
                    display::print_unknown_row("UREVO ???", &data);
                }
                None => {
                    display::print_other_notification(&notification.uuid, &notification.value);
                }
            }
            lines_since_header += 1;
        }

        // Stream ended — device disconnected.
        info!(
            "{}",
            "Device disconnected. Reconnecting in 3 seconds...".yellow()
        );
        step_tracker.on_reconnect();
        activity_tracker.on_reconnect();
        let _ = device.disconnect().await;
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    }
}

const SIM_NAMES: &[&str] = &[
    "alice", "bob", "charlie", "diana", "eve", "frank", "grace", "henry", "iris", "jack",
];

async fn simulate(speed: f32, count: u32) -> anyhow::Result<()> {
    let config =
        auth::load()?.ok_or_else(|| anyhow::anyhow!("Not logged in. Run 'walker login' first."))?;
    let client = reqwest::Client::new();
    let server = config.server.clone();

    // If count == 1, use the logged-in user (no registration needed).
    if count <= 1 {
        let url = format!("{server}/api/update");
        let auth_header = format!("Bearer {}", config.token);
        info!(server = %server, user = %config.display_name, speed = %speed, "Simulating treadmill");
        println!("  Simulating: {speed} mph — press Ctrl+C to stop");

        loop {
            let _ = client
                .post(&url)
                .header("Authorization", &auth_header)
                .json(&serde_json::json!({"moving": true, "speed_mph": speed}))
                .send()
                .await;
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    }

    // Multiple users: register fake users, then run them all.
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
        println!("  Registered: {name} ({email})");
        tokens.push((name.to_string(), token));
    }

    println!("  Simulating {count} users — press Ctrl+C to stop");

    let url = format!("{server}/api/update");
    let mut rng = rand::rng();

    loop {
        for (name, token) in &tokens {
            // Randomize speed a bit per user for variety.
            let user_speed = speed + (rand::RngExt::random_range(&mut rng, -10..=10) as f32 * 0.1);
            let user_speed = user_speed.max(0.5);

            let _ = client
                .post(&url)
                .header("Authorization", format!("Bearer {token}"))
                .json(&serde_json::json!({"moving": true, "speed_mph": user_speed}))
                .send()
                .await;

            tracing::debug!(name = %name, speed = %user_speed, "Sent update");
        }
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }
}
