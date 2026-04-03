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

/// Login via localhost callback: start a local HTTP server, open the browser to the
/// server's /login page, and wait for the OAuth redirect back to localhost.
pub async fn login(server: &str, dev: bool) -> anyhow::Result<AuthConfig> {
    // Bind to a random available port.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();

    let login_url = format!("{server}/login?cli_port={port}");

    println!();
    println!("  Opening browser to log in...");
    println!();
    println!("    {login_url}");
    println!();

    if open::that(&login_url).is_err() {
        println!("  Could not open browser. Please open the URL above manually.");
    }

    println!("  Waiting for login...");

    // Accept connections in a loop — the browser may make preconnects or
    // favicon requests before/alongside the real callback request.
    loop {
        let (stream, _) = listener.accept().await?;
        match handle_callback(stream, server, dev).await {
            Ok(config) => {
                println!();
                println!("  Logged in as {}", config.display_name);
                println!();
                return Ok(config);
            }
            Err(_) => continue,
        }
    }
}

struct CallbackQuery {
    token: String,
    email: String,
    name: String,
}

fn parse_callback_query(query: &str) -> anyhow::Result<CallbackQuery> {
    let params: std::collections::HashMap<String, String> = query
        .split('&')
        .filter_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            Some((
                urlencoding::decode(k).ok()?.into_owned(),
                urlencoding::decode(v).ok()?.into_owned(),
            ))
        })
        .collect();

    Ok(CallbackQuery {
        token: params.get("token").context("missing token")?.clone(),
        email: params.get("email").context("missing email")?.clone(),
        name: params.get("name").context("missing name")?.clone(),
    })
}

/// Handle the browser redirect: parse the query params, save credentials,
/// respond with a success page, then return the config.
async fn handle_callback(
    mut stream: tokio::net::TcpStream,
    server: &str,
    dev: bool,
) -> anyhow::Result<AuthConfig> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut buf = vec![0u8; 4096];
    let n = stream.read(&mut buf).await?;
    anyhow::ensure!(n > 0, "Empty connection");
    let request = String::from_utf8_lossy(&buf[..n]);

    // Parse the GET request line to extract query string.
    let path = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .context("Invalid HTTP request")?;

    let query_str = path.split('?').nth(1).context("No query parameters")?;

    let params = parse_callback_query(query_str).context("Failed to parse callback parameters")?;

    let config = AuthConfig {
        server: server.to_string(),
        token: params.token,
        email: params.email,
        display_name: params.name.clone(),
    };
    save(&config, dev)?;

    // Send success page to the browser.
    let body = format!(
        r#"<!DOCTYPE html>
<html>
<head><title>Walker - Success</title>
<style>body {{ font-family: system-ui; max-width: 400px; margin: 80px auto; text-align: center; }}</style>
</head>
<body>
  <h1>Welcome, {}!</h1>
  <p>You can close this tab and return to your terminal.</p>
</body>
</html>"#,
        params.name
    );

    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream.write_all(response.as_bytes()).await?;
    stream.shutdown().await?;

    Ok(config)
}
