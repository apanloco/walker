use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tracing::info;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    pub server: String,
    pub token: String,
    pub email: String,
    pub display_name: String,
}

fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("walker")
}

fn config_file(dev: bool) -> PathBuf {
    config_path().join(if dev { "auth_dev.json" } else { "auth.json" })
}

pub fn load(dev: bool) -> anyhow::Result<Option<AuthConfig>> {
    let path = config_file(dev);
    if !path.exists() {
        return Ok(None);
    }
    let data = std::fs::read_to_string(&path).context("reading auth config")?;
    let config: AuthConfig = serde_json::from_str(&data).context("parsing auth config")?;
    Ok(Some(config))
}

pub fn logout(dev: bool) -> anyhow::Result<()> {
    let path = config_file(dev);
    if path.exists() {
        std::fs::remove_file(&path)?;
        println!("  Logged out");
    } else {
        println!("  Not logged in");
    }
    Ok(())
}

pub fn save(config: &AuthConfig, dev: bool) -> anyhow::Result<()> {
    let dir = config_path();
    std::fs::create_dir_all(&dir)?;
    let path = config_file(dev);
    let data = serde_json::to_string_pretty(config)?;
    std::fs::write(&path, data)?;
    info!(path = %path.display(), "Auth config saved");
    Ok(())
}

#[derive(Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
}

#[derive(Deserialize)]
struct TokenPollResponse {
    token: Option<String>,
    user: Option<UserInfo>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct UserInfo {
    email: String,
    display_name: String,
}

pub async fn login(server: &str) -> anyhow::Result<AuthConfig> {
    let client = reqwest::Client::new();

    let res: DeviceCodeResponse = client
        .post(format!("{server}/auth/device"))
        .send()
        .await?
        .json()
        .await?;

    let verify_url = format!("{server}/auth/device/verify?code={}", res.user_code);

    println!();
    println!("  Open this URL to log in:");
    println!();
    println!("    {verify_url}");
    println!();
    println!("  Your code: {}", res.user_code);
    println!();

    if open::that(&verify_url).is_ok() {
        info!("Opened browser");
    }

    println!("  Waiting for authorization...");
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        let poll_res: TokenPollResponse = client
            .post(format!("{server}/auth/device/token"))
            .json(&serde_json::json!({"device_code": res.device_code}))
            .send()
            .await?
            .json()
            .await?;

        if let (Some(token), Some(user)) = (poll_res.token, poll_res.user) {
            let config = AuthConfig {
                server: server.to_string(),
                token,
                email: user.email,
                display_name: user.display_name.clone(),
            };
            save(&config, false)?;

            println!();
            println!("  Logged in as {}", user.display_name);
            println!();
            return Ok(config);
        }

        if poll_res.error.as_deref() == Some("expired") {
            anyhow::bail!("Device code expired. Please try again.");
        }
    }
}
