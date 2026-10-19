//! Notifications (as EasyPanel's): what happened — a deployment, a backup,
//! a full disk — sent to each enabled channel that asked for it. Discord and
//! Slack through their webhooks, Telegram through its bot API, email over
//! SMTP, any other service through a webhook (JSON). Best effort: a channel
//! that fails is logged, never retried, never blocks what happened.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use rustcms::AppState;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::{Value, json};

use crate::models::{backup_run, deployment, notification_channel, project, service};

/// How long one channel may take.
const SEND_TIME: Duration = Duration::from_secs(15);
/// A full disk is said again after this long (per channel).
const DISK_AGAIN: Duration = Duration::from_secs(6 * 60 * 60);

/// The events a channel may hear (`notification_channels.events`), and
/// whether a channel set before they existed hears it.
pub const EVENTS: [(&str, bool); 9] = [
    ("deploy", true),
    ("backup", true),
    ("disk", true),
    ("certificate", true),
    // A running service whose tasks fail, or that came back.
    ("service", true),
    ("traefik", true),
    ("update", true),
    ("prune", false),
    ("login", false),
];

/// The events `channel` hears.
pub fn events_of(channel: &notification_channel::Model) -> Vec<String> {
    if let Some(list) = channel
        .events
        .as_deref()
        .and_then(|e| serde_json::from_str::<Vec<String>>(e).ok())
    {
        return list;
    }
    // Set before `events`: its choices, and the panel's alerts.
    EVENTS
        .iter()
        .filter(|(name, legacy)| match *name {
            "deploy" => channel.on_deploy,
            "backup" => channel.on_backup,
            "disk" => channel.on_disk,
            _ => *legacy,
        })
        .map(|(name, _)| (*name).to_owned())
        .collect()
}

/// What happened.
#[derive(Debug, Clone)]
pub enum Event {
    Deploy {
        project: String,
        service: String,
        trigger: String,
        /// `succeeded`, `failed` or `rolled_back`.
        status: String,
        error: Option<String>,
    },
    Backup {
        project: String,
        service: String,
        /// `backup` or `restore`.
        kind: String,
        database: String,
        status: String,
        file: Option<String>,
        error: Option<String>,
    },
    Disk {
        percent: u8,
        used: u64,
        total: u64,
    },
    /// An uploaded certificate expires soon (or has): every channel hears it
    /// (Traefik would serve it expired).
    Certificate {
        name: String,
        names: String,
        days_left: i64,
    },
    /// A running service's tasks fail (`down`: none runs, or they keep
    /// restarting), or it came back (`!down`).
    Service {
        project: String,
        service: String,
        down: bool,
        error: Option<String>,
    },
    /// Traefik stopped answering (no task runs), or runs again.
    Traefik {
        down: bool,
    },
    /// A newer image of the panel is out (the installer updates it).
    Update {
        image: String,
    },
    /// Docker objects pruned (Settings → Panel → Docker).
    Prune {
        kind: String,
        removed: usize,
        reclaimed: u64,
    },
    /// An administrator signed in from an address never seen for them.
    Login {
        user: String,
        ip: String,
    },
    /// The "Send test notification" button.
    Test,
}

impl Event {
    fn name(&self) -> &'static str {
        match self {
            Self::Deploy { .. } => "deploy",
            Self::Backup { .. } => "backup",
            Self::Disk { .. } => "disk",
            Self::Certificate { .. } => "certificate",
            Self::Service { .. } => "service",
            Self::Traefik { .. } => "traefik",
            Self::Update { .. } => "update",
            Self::Prune { .. } => "prune",
            Self::Login { .. } => "login",
            Self::Test => "test",
        }
    }

    fn wants(&self, channel: &notification_channel::Model) -> bool {
        // A test reaches it whatever it hears.
        if matches!(self, Self::Test) {
            return true;
        }
        if !events_of(channel).iter().any(|e| e == self.name()) {
            return false;
        }
        match self {
            Self::Disk { percent, .. } => i32::from(*percent) >= channel.disk_threshold,
            _ => true,
        }
    }

    /// One line: what and where.
    pub fn title(&self) -> String {
        match self {
            Self::Deploy {
                project,
                service,
                trigger,
                status,
                ..
            } => {
                let what = match status.as_str() {
                    "succeeded" => "succeeded",
                    "rolled_back" => "was rolled back",
                    _ => "failed",
                };
                format!("{trigger} of {project}/{service} {what}")
            }
            Self::Backup {
                project,
                service,
                kind,
                database,
                status,
                ..
            } => {
                let what = if status == "succeeded" {
                    "succeeded"
                } else {
                    "failed"
                };
                format!("{kind} of {project}/{service} ({database}) {what}")
            }
            Self::Disk { percent, .. } => format!("the server's disk is {percent}% full"),
            Self::Certificate {
                name, days_left, ..
            } if *days_left <= 0 => {
                format!("the certificate {name} has expired")
            }
            Self::Certificate {
                name, days_left, ..
            } => {
                format!("the certificate {name} expires in {days_left} days")
            }
            Self::Service {
                project,
                service,
                down: true,
                ..
            } => format!("{project}/{service} is failing"),
            Self::Service {
                project, service, ..
            } => format!("{project}/{service} runs again"),
            Self::Traefik { down: true } => "Traefik is not running: no domain answers".to_owned(),
            Self::Traefik { down: false } => "Traefik runs again".to_owned(),
            Self::Update { .. } => "a newer RDockPanel is out".to_owned(),
            Self::Prune { kind, removed, .. } => format!("Docker {kind} pruned: {removed} removed"),
            Self::Login { user, ip } => format!("{user} signed in from a new address ({ip})"),
            Self::Test => "a test notification from RDockPanel".to_owned(),
        }
    }

    /// The details under the title.
    pub fn details(&self) -> String {
        match self {
            Self::Deploy { error, .. } => error.clone().unwrap_or_default(),
            Self::Backup { file, error, .. } => error
                .clone()
                .or_else(|| file.as_ref().map(|f| format!("file {f}")))
                .unwrap_or_default(),
            Self::Disk { used, total, .. } => format!(
                "{} GB used of {} GB",
                used / 1_000_000_000,
                total / 1_000_000_000
            ),
            Self::Certificate { names, .. } => {
                format!("{names}: upload a renewed one (Settings → Certificates)")
            }
            Self::Service { error, .. } => error.clone().unwrap_or_default(),
            Self::Traefik { .. } => String::new(),
            Self::Update { image } => {
                format!("{image}: Settings → Panel → Check for updates gives the installer command")
            }
            Self::Prune { reclaimed, .. } => {
                format!("{} MB freed", reclaimed / 1_000_000)
            }
            Self::Login { .. } => {
                "Not you? Change the password and sign the sessions out.".to_owned()
            }
            Self::Test => "This channel works.".to_owned(),
        }
    }

    fn ok(&self) -> bool {
        match self {
            Self::Deploy { status, .. } | Self::Backup { status, .. } => status == "succeeded",
            Self::Service { down, .. } | Self::Traefik { down } => !down,
            Self::Prune { .. } | Self::Test => true,
            Self::Disk { .. }
            | Self::Certificate { .. }
            | Self::Update { .. }
            | Self::Login { .. } => false,
        }
    }

    fn data(&self) -> Value {
        match self {
            Self::Deploy {
                project,
                service,
                trigger,
                status,
                error,
            } => {
                json!({"project": project, "service": service, "trigger": trigger, "status": status, "error": error})
            }
            Self::Backup {
                project,
                service,
                kind,
                database,
                status,
                file,
                error,
            } => {
                json!({"project": project, "service": service, "kind": kind, "database": database, "status": status, "file": file, "error": error})
            }
            Self::Disk {
                percent,
                used,
                total,
            } => json!({"percent": percent, "used": used, "total": total}),
            Self::Certificate {
                name,
                names,
                days_left,
            } => json!({"name": name, "names": names, "days_left": days_left}),
            Self::Service {
                project,
                service,
                down,
                error,
            } => json!({"project": project, "service": service, "down": down, "error": error}),
            Self::Traefik { down } => json!({"down": down}),
            Self::Update { image } => json!({"image": image}),
            Self::Prune {
                kind,
                removed,
                reclaimed,
            } => json!({"kind": kind, "removed": removed, "reclaimed": reclaimed}),
            Self::Login { user, ip } => json!({"user": user, "ip": ip}),
            Self::Test => json!({}),
        }
    }

    fn text(&self) -> String {
        let mark = if self.ok() { "✅" } else { "⚠️" };
        let details = self.details();
        if details.is_empty() {
            format!("{mark} RDockPanel: {}", self.title())
        } else {
            format!("{mark} RDockPanel: {}\n{details}", self.title())
        }
    }
}

/// An http(s) URL the panel posts to (no credentials in it).
pub fn valid_url(url: &str) -> bool {
    reqwest::Url::parse(url).is_ok_and(|u| {
        matches!(u.scheme(), "https" | "http") && u.host_str().is_some() && u.username().is_empty()
    })
}

/// Sends `event` to `channel` now: why it failed.
pub async fn send(
    state: &AppState,
    channel: &notification_channel::Model,
    event: &Event,
) -> Result<(), String> {
    let secret = match &channel.secret {
        Some(sealed) => sealed
            .open(state.keyring(), &notification_channel::secret_context())
            .map_err(|_| "its secret cannot be read (sealed with another APP_KEY)".to_owned())?,
        None => String::new(),
    };
    tokio::time::timeout(SEND_TIME, deliver(channel, &secret, event))
        .await
        .map_err(|_| "no answer in 15 s".to_owned())?
}

async fn deliver(
    channel: &notification_channel::Model,
    secret: &str,
    event: &Event,
) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())?;
    let post = |url: String, body: Value| {
        let client = client.clone();
        async move {
            if !valid_url(&url) {
                return Err("the URL is not an http(s) URL".to_owned());
            }
            let response = client
                .post(url)
                .json(&body)
                .send()
                .await
                // Never the URL (it is the secret).
                .map_err(|e| format!("the request failed: {}", e.without_url()))?;
            if response.status().is_success() {
                Ok(())
            } else {
                Err(format!("answered {}", response.status()))
            }
        }
    };
    match channel.kind.as_str() {
        "discord" => post(secret.to_owned(), json!({ "content": event.text() })).await,
        "slack" => post(secret.to_owned(), json!({ "text": event.text() })).await,
        "webhook" => {
            post(
                secret.to_owned(),
                json!({
                    "event": event.name(),
                    "ok": event.ok(),
                    "title": event.title(),
                    "details": event.details(),
                    "data": event.data(),
                }),
            )
            .await
        }
        "telegram" => {
            if secret.is_empty()
                || !secret
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '_' | '-'))
            {
                return Err("the bot token is not valid".to_owned());
            }
            post(
                format!("https://api.telegram.org/bot{secret}/sendMessage"),
                json!({ "chat_id": channel.chat_id, "text": event.text() }),
            )
            .await
        }
        "email" => email(channel, secret, event).await,
        other => Err(format!("unknown channel kind {other}")),
    }
}

async fn email(
    channel: &notification_channel::Model,
    password: &str,
    event: &Event,
) -> Result<(), String> {
    use lettre::message::Mailbox;
    use lettre::transport::smtp::authentication::Credentials;
    use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
    let from: Mailbox = channel
        .smtp_from
        .as_deref()
        .unwrap_or_default()
        .parse()
        .map_err(|_| "the sender is not an email address".to_owned())?;
    let mut message = Message::builder()
        .from(from)
        .subject(format!("RDockPanel: {}", event.title()));
    for to in channel
        .email_to
        .as_deref()
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
    {
        message = message.to(to
            .parse()
            .map_err(|_| format!("{to} is not an email address"))?);
    }
    let message = message
        .body(format!("{}\n\n{}\n", event.title(), event.details()))
        .map_err(|e| e.to_string())?;
    let host = channel.smtp_host.as_deref().unwrap_or_default();
    let port = channel
        .smtp_port
        .and_then(|p| u16::try_from(p).ok())
        .unwrap_or(587);
    // 465: TLS from the start; else STARTTLS (required).
    let builder = if port == 465 {
        AsyncSmtpTransport::<Tokio1Executor>::relay(host)
    } else {
        AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host)
    }
    .map_err(|e| format!("the SMTP server: {e}"))?
    .port(port)
    .timeout(Some(SEND_TIME));
    let builder = match channel.smtp_username.as_deref().filter(|u| !u.is_empty()) {
        Some(user) => builder.credentials(Credentials::new(user.to_owned(), password.to_owned())),
        None => builder,
    };
    builder
        .build()
        .send(message)
        .await
        .map(|_| ())
        .map_err(|e| format!("the SMTP server: {e}"))
}

/// Sends `event` to every enabled channel that wants it, in the background.
pub fn notify(state: &AppState, event: Event) {
    let state = state.clone();
    tokio::spawn(async move {
        let channels = match notification_channel::Entity::find()
            .filter(notification_channel::Column::Enabled.eq(true))
            .all(state.db())
            .await
        {
            Ok(channels) => channels,
            Err(err) => {
                tracing::warn!(%err, "notification channels not read");
                return;
            }
        };
        // Each channel on its own: one that fails (and is retried) never
        // delays another.
        for channel in channels.into_iter().filter(|c| event.wants(c)) {
            if matches!(event, Event::Disk { .. }) && !disk_due(channel.id) {
                continue;
            }
            let state = state.clone();
            let event = event.clone();
            tokio::spawn(async move {
                // Twice more, a few seconds apart, then given up (logged).
                let mut tries = 0;
                while let Err(err) = send(&state, &channel, &event).await {
                    tries += 1;
                    if tries == 3 {
                        tracing::warn!(
                            channel = channel.name,
                            event = event.name(),
                            err,
                            "notification not sent"
                        );
                        break;
                    }
                    tokio::time::sleep(Duration::from_secs(5 * tries)).await;
                }
            });
        }
    });
}

/// Whether a full disk is said to `channel` now (at most every [`DISK_AGAIN`]).
fn disk_due(channel: i64) -> bool {
    static LAST: Mutex<Option<HashMap<i64, Instant>>> = Mutex::new(None);
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    let last = last.get_or_insert_with(HashMap::new);
    match last.get(&channel) {
        Some(at) if at.elapsed() < DISK_AGAIN => false,
        _ => {
            last.insert(channel, Instant::now());
            true
        }
    }
}

async fn names(state: &AppState, service_id: i64) -> Option<(String, String)> {
    let service = service::Entity::find_by_id(service_id)
        .one(state.db())
        .await
        .ok()??;
    let project = project::Entity::find_by_id(service.project_id)
        .one(state.db())
        .await
        .ok()??;
    Some((project.name, service.name))
}

/// A deployment finished: Deploy and Rollback, and every failure.
pub async fn deployment_finished(state: &AppState, id: i64) {
    let Ok(Some(row)) = deployment::Entity::find_by_id(id).one(state.db()).await else {
        return;
    };
    let failed = row.status != "succeeded";
    if !failed && !matches!(row.trigger.as_str(), "deploy" | "rollback") {
        return;
    }
    let Some((project, service)) = names(state, row.service_id).await else {
        return;
    };
    notify(
        state,
        Event::Deploy {
            project,
            service,
            trigger: row.trigger,
            status: row.status,
            error: row.error,
        },
    );
}

/// A backup or restore finished.
pub async fn backup_finished(state: &AppState, run: &backup_run::Model) {
    let Some((project, service)) = names(state, run.service_id).await else {
        return;
    };
    notify(
        state,
        Event::Backup {
            project,
            service,
            kind: run.kind.clone(),
            database: run.database_name.clone(),
            status: run.status.clone(),
            file: run.file.clone(),
            error: run.error.clone(),
        },
    );
}

/// The disk check (every few minutes): a full disk to the channels that
/// asked, each at its own threshold.
pub fn check_disk(state: &AppState) {
    let Some(stats) = state.service::<crate::services::system_stats_service::SystemStats>() else {
        return;
    };
    let sample = stats.sample_now();
    if sample.disk_total == 0 {
        return;
    }
    let percent = (sample.disk_used.saturating_mul(100) / sample.disk_total).min(100) as u8;
    notify(
        state,
        Event::Disk {
            percent,
            used: sample.disk_used,
            total: sample.disk_total,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_read_well() {
        let deploy = Event::Deploy {
            project: "shop".into(),
            service: "web".into(),
            trigger: "deploy".into(),
            status: "failed".into(),
            error: Some("the image cannot be pulled".into()),
        };
        assert_eq!(deploy.title(), "deploy of shop/web failed");
        assert_eq!(
            deploy.text(),
            "⚠️ RDockPanel: deploy of shop/web failed\nthe image cannot be pulled"
        );
        let disk = Event::Disk {
            percent: 92,
            used: 92_000_000_000,
            total: 100_000_000_000,
        };
        assert_eq!(disk.details(), "92 GB used of 100 GB");
        assert!(valid_url("https://discord.com/api/webhooks/1/x"));
        assert!(!valid_url("file:///etc/passwd") && !valid_url("https://u:p@x"));
        assert!(disk_due(-7) && !disk_due(-7));
    }
}
