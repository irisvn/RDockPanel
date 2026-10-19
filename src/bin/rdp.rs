//! `rdp`, RDockPanel's command line: the public API (`/api/v1`) with a token
//! of Settings → API tokens.
//!
//!   rdp login https://panel.example.com   (the token is read from stdin)
//!   rdp projects
//!   rdp status shop/web
//!   rdp deploy shop/web [--wait]
//!   rdp restart shop/web [--wait]
//!   rdp logs <deployment>
//!   rdp env shop/web
//!   rdp domains shop/web
//!
//! The panel's address and token are kept in `~/.config/rdp/config.json`
//! (mode 0600), or given by `RDP_URL` and `RDP_TOKEN`.

use std::io::{BufRead, Write};
use std::process::ExitCode;
use std::time::Duration;

use serde_json::Value;

struct Config {
    url: String,
    token: String,
}

fn config_path() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::Path::new(&h).join(".config")))?;
    Some(home.join("rdp").join("config.json"))
}

fn load() -> Result<Config, String> {
    if let (Ok(url), Ok(token)) = (std::env::var("RDP_URL"), std::env::var("RDP_TOKEN")) {
        return Ok(Config {
            url: checked_url(&url)?,
            token,
        });
    }
    let path = config_path().ok_or("no home directory")?;
    let text =
        std::fs::read_to_string(&path).map_err(|_| "not signed in: rdp login <url>".to_owned())?;
    let value: Value =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(Config {
        // Checked again: a config edited by hand gets the token nowhere else.
        url: checked_url(value["url"].as_str().unwrap_or_default())?,
        token: value["token"].as_str().unwrap_or_default().to_owned(),
    })
}

/// `https://host[:port]` (or http to localhost), no path, credentials or query.
fn checked_url(url: &str) -> Result<String, String> {
    let parsed = reqwest::Url::parse(url.trim()).map_err(|e| format!("{url}: {e}"))?;
    let local = matches!(parsed.host_str(), Some("localhost" | "127.0.0.1" | "::1"));
    if parsed.scheme() != "https" && !(parsed.scheme() == "http" && local) {
        return Err("the panel's address must be https (http only for localhost)".into());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() || parsed.query().is_some() {
        return Err("the panel's address has no credentials or query".into());
    }
    let mut base = format!(
        "{}://{}",
        parsed.scheme(),
        parsed.host_str().unwrap_or_default()
    );
    if let Some(port) = parsed.port() {
        base.push_str(&format!(":{port}"));
    }
    Ok(base)
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        // The token goes to the panel only.
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("rdp/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| e.to_string())
}

async fn call(config: &Config, method: reqwest::Method, path: &str) -> Result<Value, String> {
    let response = client()?
        .request(method, format!("{}/api/v1{path}", config.url))
        .bearer_auth(&config.token)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| format!("the panel did not answer: {}", e.without_url()))?;
    let status = response.status();
    let body: Value = response.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        let message = body["message"].as_str().unwrap_or("refused");
        return Err(format!("{} {message}", status.as_u16()));
    }
    Ok(body)
}

fn segment(id: &str) -> String {
    percent_encoding::utf8_percent_encode(id, percent_encoding::NON_ALPHANUMERIC).to_string()
}

async fn login(url: &str) -> Result<(), String> {
    let url = checked_url(url)?;
    eprint!("API token (Settings → API tokens): ");
    let _ = std::io::stderr().flush();
    let token = read_token()?;
    let token = token.trim().to_owned();
    if !token.starts_with("rcms_") {
        return Err("that is no RDockPanel token (rcms_…)".into());
    }
    let config = Config {
        url: url.clone(),
        token,
    };
    call(&config, reqwest::Method::GET, "/me").await?;
    let path = config_path().ok_or("no home directory")?;
    let text = serde_json::json!({ "url": config.url, "token": config.token }).to_string();
    save(&path, &text).map_err(|e| format!("{}: {e}", path.display()))?;
    println!("signed in to {url} ({})", path.display());
    Ok(())
}

/// The token from stdin, not echoed when it is a terminal.
fn read_token() -> Result<String, String> {
    use std::io::IsTerminal;
    let tty = std::io::stdin().is_terminal();
    let stty = |arg: &str| {
        std::process::Command::new("stty")
            .arg(arg)
            .stdin(std::process::Stdio::inherit())
            .status()
            .is_ok_and(|s| s.success())
    };
    // Never echoed: without stty, give it through a pipe.
    if tty && !stty("-echo") {
        return Err("cannot hide the input (stty): echo $TOKEN | rdp login <url>".into());
    }
    let mut token = String::new();
    let read = std::io::stdin().lock().read_line(&mut token);
    if tty {
        let _ = stty("echo");
        eprintln!();
    }
    read.map_err(|e| e.to_string())?;
    Ok(token)
}

/// Writes the config whole or not at all: a file of mode 0600 in a
/// directory of 0700, renamed over the old one (never through a symlink).
fn save(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
    let dir = path.parent().ok_or(std::io::ErrorKind::InvalidInput)?;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    if std::fs::symlink_metadata(dir)?.file_type().is_symlink() {
        return Err(std::io::Error::other("the config directory is a symlink"));
    }
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    if let Ok(meta) = std::fs::symlink_metadata(path)
        && meta.file_type().is_symlink()
    {
        return Err(std::io::Error::other("the config is a symlink"));
    }
    let temp = dir.join(format!(".config.json.{}", std::process::id()));
    let written = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&temp, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    written
}

async fn wait(config: &Config, deployment: i64) -> Result<(), String> {
    for _ in 0..600 {
        let d = call(
            config,
            reqwest::Method::GET,
            &format!("/deployments/{deployment}"),
        )
        .await?;
        let d = &d["deployment"];
        if d["finished"].as_bool().unwrap_or(true) {
            println!(
                "deployment {deployment}: {}",
                d["status"].as_str().unwrap_or("?")
            );
            if let Some(log) = d["log"].as_str() {
                print!("{log}");
            }
            return if d["status"] == "succeeded" {
                Ok(())
            } else {
                Err(d["error"].as_str().unwrap_or("failed").to_owned())
            };
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    Err("still running after 20 minutes".into())
}

async fn run(args: Vec<String>) -> Result<(), String> {
    let arg = |i: usize| args.get(i).map(String::as_str);
    match arg(0) {
        Some("login") => login(arg(1).ok_or("rdp login <url>")?).await,
        Some("projects") => {
            let config = load()?;
            let out = call(&config, reqwest::Method::GET, "/projects").await?;
            for p in out["projects"].as_array().into_iter().flatten() {
                println!("{}", p["name"].as_str().unwrap_or_default());
                for s in p["services"].as_array().into_iter().flatten() {
                    println!(
                        "  {}/{}  {}  {}",
                        p["name"].as_str().unwrap_or_default(),
                        s["name"].as_str().unwrap_or_default(),
                        s["kind"].as_str().unwrap_or_default(),
                        s["status"].as_str().unwrap_or_default()
                    );
                }
            }
            Ok(())
        }
        Some(command @ ("status" | "env" | "domains")) => {
            let config = load()?;
            let id = segment(arg(1).ok_or("give a service: project/service")?);
            let path = match command {
                "status" => format!("/services/{id}"),
                other => format!("/services/{id}/{other}"),
            };
            let out = call(&config, reqwest::Method::GET, &path).await?;
            if command == "env" {
                for (k, v) in out["env"].as_object().into_iter().flatten() {
                    println!("{k}={}", v.as_str().unwrap_or_default());
                }
            } else {
                println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
            }
            Ok(())
        }
        Some(command @ ("deploy" | "restart")) => {
            let config = load()?;
            let id = segment(arg(1).ok_or("give a service: project/service")?);
            let out = call(&config, reqwest::Method::POST, &format!("/services/{id}/{command}")).await?;
            let deployment = out["deployment"]["id"].as_i64().unwrap_or_default();
            println!("deployment {deployment} queued");
            if args.iter().any(|a| a == "--wait") {
                wait(&config, deployment).await?;
            }
            Ok(())
        }
        Some("logs") => {
            let config = load()?;
            let id: i64 = arg(1)
                .and_then(|i| i.parse().ok())
                .ok_or("rdp logs <deployment>")?;
            wait(&config, id).await
        }
        _ => Err(
            "usage: rdp login <url> | projects | status <p/s> | deploy <p/s> [--wait] | restart <p/s> [--wait] | logs <deployment> | env <p/s> | domains <p/s>"
                .into(),
        ),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("rdp: {e}");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(run(args)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("rdp: {e}");
            ExitCode::FAILURE
        }
    }
}
