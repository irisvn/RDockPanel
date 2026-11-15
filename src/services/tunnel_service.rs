//! Settings → Cloudflare Tunnel: the panel runs one `cloudflared` per tunnel
//! (the Swarm service [`service_name`]: `rdp-cloudflared` for `default`,
//! `rdp-cloudflared-<name>` else) with that tunnel's token, so domains are
//! reached through Cloudflare without opening ports 80 and 443. Every
//! cloudflared sends every hostname to Traefik's `tunnel` entrypoint
//! (`:8081`, never published, reached on the network `rdp-tunnel` only, which
//! they all share), where each domain is served over plain HTTP (Cloudflare
//! holds TLS): no redirect to HTTPS, so no loop.
//!
//! Each token is sealed (`tunnels`, APP_KEY; `default`'s in `panel_settings`
//! too, for a release rolled back), one tunnel per token, and given to its
//! cloudflared as a Swarm secret read with `--token-file` (never an
//! argument, an environment variable or a log). The panel's administrators' only
//! (`system.manage`). Public hostnames are set in Cloudflare (Zero Trust →
//! Tunnels → the tunnel → Public Hostname), each to [`status`]'s `origin`.

use std::collections::HashMap;

use rustcms::crypto::{Context, Encrypted};
use rustcms::{AppError, AppResult, AppState, ValidationErrors};
use serde_json::{Value, json};

use rustcms::db::begin_write;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, Set};

use crate::models::{panel_setting, tunnel};
use crate::services::docker_service::{Docker, DockerError, NetworkRequest, SYSTEM_LABEL};
use crate::services::traefik_service::Traefik;

/// cloudflared, pinned (it reads `--token-file`).
pub const IMAGE: &str = "cloudflare/cloudflared:2026.10.2";
/// The service of the tunnel `default` (the panel's first, of one token).
pub const SERVICE: &str = "rdp-cloudflared";
/// The first tunnel's name (the one Settings had before many).
pub const DEFAULT: &str = "default";
pub const NETWORK: &str = "rdp-tunnel";
/// Traefik's entrypoint for the tunnel, and its port.
pub const ENTRYPOINT: &str = "tunnel";
pub const PORT: u16 = 8081;
const TOKEN_KEY: &str = "tunnel.token";
const ENABLED_KEY: &str = "tunnel.enabled";
/// Traefik's argument for the entrypoint. Its forwarded headers are
/// trusted from the tunnel's network only ([`trust_arg`]): Traefik is on
/// every project's network, any container reaches it, and theirs are
/// dropped. cloudflared names the visitor (`X-Forwarded-For`,
/// `Cf-Connecting-Ip`) and their scheme (`Cf-Visitor`).
const TRAEFIK_ARG: &str = "--entrypoints.tunnel.address=:8081";
/// The argument's prefix trusting the tunnel's network.
const TRUST_PREFIX: &str = "--entrypoints.tunnel.forwardedHeaders.trustedIPs=";

/// Traefik's argument trusting the forwarded headers of `subnets` (the
/// tunnel's network: cloudflared and Traefik only).
fn trust_arg(subnets: &[String]) -> String {
    format!("{TRUST_PREFIX}{}", subnets.join(","))
}
/// Its aliasing header names dropped (the installer's, kept).
const TRAEFIK_ALIAS_ARG: &str = "--entrypoints.tunnel.http.aliasHeadersStrategy=delete";
/// Cloudflare's header for a visitor who came over plain HTTP (routed to
/// a redirect to HTTPS: no loop, the ones over HTTPS do not carry it).
pub const PLAIN_VISITOR: &str = r#"Header(`Cf-Visitor`, `{"scheme":"http"}`)"#;
/// Cloudflare's header for a visitor who came over HTTPS (Cloudflare's).
pub const SECURE_VISITOR: &str = r#"Header(`Cf-Visitor`, `{"scheme":"https"}`)"#;
/// Why a tunnel does not connect, as Settings says it.
const UNHEALTHY: &str =
    "Cloudflare does not accept the connection: is the tunnel's token still valid?";
/// The user cloudflared's image runs as (distroless `nonroot`).
const CLOUDFLARED_UID: &str = "65532";

static CHANGING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Held while a tunnel is turned on or off (one change at a time).
pub async fn exclusive() -> tokio::sync::MutexGuard<'static, ()> {
    CHANGING.lock().await
}

/// Where `default`'s token is sealed in `panel_settings`.
fn settings_context() -> Context {
    Context::column("panel_settings", "tunnel.token")
}

/// The Swarm service of the tunnel `name`.
pub fn service_name(name: &str) -> String {
    if name == DEFAULT {
        SERVICE.to_owned()
    } else {
        format!("{SERVICE}-{name}")
    }
}

/// 1-40 lowercase letters, digits and dashes, starting with a letter.
pub fn valid_name(name: &str) -> bool {
    (1..=40).contains(&name.len())
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Whether routes serve the tunnel's entrypoint too (a tunnel is on).
pub fn enabled(app: &AppState) -> bool {
    app.service::<Traefik>().is_some_and(|t| t.tunnel())
}

async fn any_enabled(app: &AppState) -> AppResult<bool> {
    Ok(tunnel::Entity::find()
        .filter(tunnel::Column::Enabled.eq(true))
        .one(app.db())
        .await?
        .is_some())
}

/// At boot: the tunnel of an older panel's settings becomes `default`, and
/// routes serve the tunnel's entrypoint when a tunnel is on.
pub async fn load(app: &AppState) -> AppResult<()> {
    adopt_settings(app).await?;
    let on = any_enabled(app).await?;
    if let Some(traefik) = app.service::<Traefik>() {
        traefik.set_tunnel(on);
    }
    Ok(())
}

/// `panel_settings`' tunnel (`tunnel.*`, before many) as the row `default`,
/// once. Here, not in the migration: sealing for the new column needs the
/// APP_KEY. A token it cannot open is kept as it is (Settings asks again).
async fn adopt_settings(app: &AppState) -> AppResult<()> {
    let db = app.db();
    let Some(kept) = panel_setting::get(db, TOKEN_KEY).await? else {
        return Ok(());
    };
    if tunnel::named(db, DEFAULT).await?.is_some() {
        return Ok(());
    }
    let old: Encrypted<String> = Encrypted::from_envelope(kept);
    let token = match old.open(app.keyring(), &settings_context()) {
        Ok(token) => Encrypted::seal(app.keyring(), &tunnel::token_context(), &token)?,
        Err(_) => old,
    };
    let enabled = panel_setting::get(db, ENABLED_KEY).await?.as_deref() == Some("true");
    let now = chrono::Utc::now();
    // One statement: two boots at once make one row.
    tunnel::Entity::insert(tunnel::ActiveModel {
        name: Set(DEFAULT.to_owned()),
        token: Set(token),
        enabled: Set(enabled),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    })
    .on_conflict(
        // A row made meanwhile kept (MySQL has no `DO NOTHING`).
        sea_orm::sea_query::OnConflict::column(tunnel::Column::Name)
            .update_column(tunnel::Column::Name)
            .to_owned(),
    )
    .exec_without_returning(db)
    .await?;
    Ok(())
}

/// The name and token of every tunnel that is on (a token the APP_KEY
/// cannot open left out).
pub async fn running_tokens(app: &AppState) -> AppResult<Vec<(String, String)>> {
    Ok(tunnel::all(app.db())
        .await?
        .into_iter()
        .filter(|row| row.enabled)
        .filter_map(|row| {
            let token = row
                .token
                .open(app.keyring(), &tunnel::token_context())
                .ok()?;
            Some((row.name, token))
        })
        .collect())
}

/// A tunnel token as Cloudflare gives it: base64 of a JSON object with its
/// account (`a`), tunnel (`t`) and secret (`s`).
pub fn valid_token(token: &str) -> bool {
    use base64::Engine;
    let token = token.trim();
    if !(60..=4096).contains(&token.len()) {
        return false;
    }
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(token)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(token));
    let Ok(Ok(Value::Object(fields))) =
        decoded.map(|bytes| serde_json::from_slice::<Value>(&bytes))
    else {
        return false;
    };
    let text = |key: &str| fields.get(key).and_then(Value::as_str).unwrap_or_default();
    text("a").len() >= 16
        && text("a").chars().all(|c| c.is_ascii_hexdigit())
        && text("t").len() == 36
        && text("t").chars().all(|c| c.is_ascii_hexdigit() || c == '-')
        && text("s").len() >= 16
}

fn docker_err(err: DockerError) -> AppError {
    AppError::from(err)
}

/// What Settings shows of the tunnel `name`: on or off, a token kept (never
/// shown), its cloudflared's tasks.
pub async fn status(app: &AppState, docker: &Docker, name: &str) -> AppResult<Value> {
    let row = tunnel::named(app.db(), name).await?;
    state_of(docker, name, row.as_ref()).await
}

/// Every tunnel and its [`status`] (never its token).
pub async fn list(app: &AppState, docker: &Docker) -> AppResult<Value> {
    let mut all = vec![];
    for row in tunnel::all(app.db()).await? {
        all.push(state_of(docker, &row.name, Some(&row)).await?);
    }
    Ok(Value::Array(all))
}

async fn state_of(docker: &Docker, name: &str, row: Option<&tunnel::Model>) -> AppResult<Value> {
    let service_name = service_name(name);
    let service = docker
        .engine()
        .inspect_service(&service_name)
        .await
        .map_err(docker_err)?;
    let tasks = if service.is_some() {
        docker
            .engine()
            .list_tasks(&service_name)
            .await
            .unwrap_or_default()
    } else {
        vec![]
    };
    let running = tasks.iter().any(|t| {
        t.desired_state == Some(crate::services::docker_service::TaskState::RUNNING)
            && t.status.as_ref().and_then(|s| s.state)
                == Some(crate::services::docker_service::TaskState::RUNNING)
    });
    // Swarm replaces an unhealthy task (its exit kept in the history).
    let failed = tasks.iter().any(|t| {
        t.status.as_ref().and_then(|s| s.state)
            == Some(crate::services::docker_service::TaskState::FAILED)
    });
    // A running task is no tunnel yet (a revoked token retries forever):
    // its healthcheck says whether Cloudflare took the connection.
    let (connected, log_error) = if running {
        connection(docker, &tasks).await
    } else {
        (false, None)
    };
    // Connected: what failed before is over. Else why, when known.
    let error = if connected {
        None
    } else {
        log_error.or_else(|| failed.then(|| UNHEALTHY.to_owned()))
    };
    Ok(json!({
        "name": name,
        "enabled": row.is_some_and(|r| r.enabled),
        "token_set": row.is_some(),
        "service": service.is_some(),
        "running": running,
        "connected": connected,
        "error": error,
        "image": IMAGE,
        "origin": format!("http://{}:{PORT}", crate::services::deploy_rules_service::config().traefik_service),
    }))
}

/// Whether the running task's cloudflared is connected to Cloudflare (its
/// healthcheck: `cloudflared tunnel ready`), and else why not. Read within
/// a few seconds (else: not known yet).
async fn connection(
    docker: &Docker,
    tasks: &[crate::services::docker_service::SwarmTask],
) -> (bool, Option<String>) {
    use crate::services::docker_service::TaskState;
    let Some(container) = tasks
        .iter()
        .filter(|t| t.status.as_ref().and_then(|s| s.state) == Some(TaskState::RUNNING))
        .find_map(|t| {
            t.status
                .as_ref()?
                .container_status
                .as_ref()?
                .container_id
                .clone()
        })
    else {
        return (false, None);
    };
    let inspected = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        docker.engine().inspect_container(&container),
    )
    .await;
    match inspected {
        Ok(Ok(document)) => health_news(&document),
        _ => (false, None),
    }
}

/// [`connection`] from a container's inspect document.
fn health_news(document: &Value) -> (bool, Option<String>) {
    match document["State"]["Health"]["Status"].as_str() {
        Some("healthy") => (true, None),
        Some("unhealthy") => (false, Some(UNHEALTHY.to_owned())),
        _ => (false, None),
    }
}

/// Keeps the tunnel `name` (made when new) with `token` (sealed) when given,
/// then runs its cloudflared with the token kept: Traefik gets its
/// entrypoint and the tunnel's network, every route its tunnel router.
pub async fn enable(
    app: &AppState,
    docker: &Docker,
    name: &str,
    token: Option<&str>,
) -> AppResult<()> {
    let db = app.db();
    if !valid_name(name) {
        return Err(ValidationErrors::field(
            "name",
            "is 1-40 lowercase letters, digits and dashes, starting with a letter",
        )
        .into());
    }
    let row = tunnel::named(db, name).await?;
    // A new token is kept only once cloudflared runs with it (the one kept
    // goes on working until then).
    let new = token.map(str::trim).filter(|t| !t.is_empty());
    let token = match new {
        Some(token) => {
            if !valid_token(token) {
                return Err(ValidationErrors::field(
                    "token",
                    "is not a tunnel token (Cloudflare → Zero Trust → Tunnels)",
                )
                .into());
            }
            // One cloudflared per token (two would share its connections).
            let others = tunnel::all(db).await?;
            if others.iter().filter(|o| o.name != name).any(|o| {
                o.token
                    .open(app.keyring(), &tunnel::token_context())
                    .is_ok_and(|t| t == token)
            }) {
                return Err(
                    ValidationErrors::field("token", "is already used by another tunnel").into(),
                );
            }
            token.to_owned()
        }
        None => {
            let Some(row) = &row else {
                return Err(ValidationErrors::field("token", "is required").into());
            };
            row.token
                .open(app.keyring(), &tunnel::token_context())
                .map_err(|_| {
                    ValidationErrors::field(
                        "token",
                        "cannot be read (sealed with another APP_KEY): set it again",
                    )
                })?
        }
    };
    let traefik = app
        .service::<Traefik>()
        .ok_or_else(|| AppError::internal("Traefik is not bound"))?;
    if traefik.service.is_empty() {
        return Err(ValidationErrors::field(
            "token",
            "Traefik is not a Swarm service of the panel's here",
        )
        .into());
    }
    // The network cloudflared and Traefik share (nothing else).
    let labels = HashMap::from([(SYSTEM_LABEL.to_owned(), "true".to_owned())]);
    let network = docker
        .ensure_network(NetworkRequest {
            name: NETWORK.to_owned(),
            driver: "overlay".to_owned(),
            labels: labels.clone(),
            attachable: false,
            internal: false,
        })
        .await
        .map_err(docker_err)?;
    entrypoint(docker, &traefik, &network.id).await?;
    // The token as a secret of its own content's name (a new token: a new secret).
    use sha2::Digest;
    let digest: String = sha2::Sha256::digest(token.as_bytes())[..6]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let secret_name = format!("rdp_tunnel_{digest}");
    let secrets = docker
        .engine()
        .list_secrets(SYSTEM_LABEL, "true")
        .await
        .map_err(docker_err)?;
    let (secret_id, made) = match secrets.iter().find(|(_, name)| *name == secret_name) {
        Some((id, _)) => (id.clone(), false),
        None => (
            docker
                .engine()
                .create_secret(&secret_name, token.as_bytes(), labels.clone())
                .await
                .map_err(docker_err)?,
            true,
        ),
    };
    let service = service_name(name);
    let spec: crate::services::docker_service::ServiceSpec = serde_json::from_value(json!({
        "Name": service,
        "Labels": { SYSTEM_LABEL: "true" },
        "TaskTemplate": {
            "ContainerSpec": {
                "Image": IMAGE,
                "Args": ["tunnel", "--no-autoupdate", "--metrics", "127.0.0.1:2000",
                         "run", "--token-file", "/run/secrets/tunnel_token"],
                "Labels": { SYSTEM_LABEL: "true" },
                // Connected to Cloudflare (`/ready` of its metrics).
                "HealthCheck": {
                    "Test": ["CMD", "cloudflared", "tunnel", "--metrics", "127.0.0.1:2000", "ready"],
                    "Interval": 10_000_000_000u64,
                    "Timeout": 5_000_000_000u64,
                    "Retries": 3,
                    "StartPeriod": 20_000_000_000u64,
                },
                "Secrets": [{
                    "SecretID": secret_id,
                    "SecretName": secret_name,
                    "File": { "Name": "tunnel_token", "UID": CLOUDFLARED_UID, "GID": CLOUDFLARED_UID, "Mode": 0o400 },
                }],
            },
            "Networks": [{ "Target": network.id }],
            "RestartPolicy": { "Condition": "any", "Delay": 5_000_000_000u64 },
        },
        "Mode": { "Replicated": { "Replicas": 1 } },
    }))
    .map_err(AppError::internal)?;
    let ran = async {
        match docker.engine().inspect_service(&service).await? {
            Some(found) => {
                let version = found.version.and_then(|v| v.index).unwrap_or(0);
                docker
                    .engine()
                    .update_service(&service, version, spec, None)
                    .await
                    .map(|_| ())
            }
            None => docker.engine().create_service(spec, None).await.map(|_| ()),
        }
    }
    .await;
    if let Err(err) = ran {
        // The secret made for it, unused.
        if made {
            let _ = docker.engine().delete_secret(&secret_id).await;
        }
        return Err(docker_err(err));
    }
    let now = chrono::Utc::now();
    let sealed = Encrypted::seal(app.keyring(), &tunnel::token_context(), &token)?;
    let txn = begin_write(db).await?;
    match row {
        Some(row) => {
            let mut kept = row.into_active_model();
            if new.is_some() {
                kept.token = Set(sealed);
            }
            kept.enabled = Set(true);
            kept.updated_at = Set(now);
            kept.update(&txn).await?;
        }
        None => {
            tunnel::ActiveModel {
                name: Set(name.to_owned()),
                token: Set(sealed),
                enabled: Set(true),
                created_at: Set(now),
                updated_at: Set(now),
                ..Default::default()
            }
            .insert(&txn)
            .await?;
        }
    }
    if name == DEFAULT {
        // As an older release reads it (one rolled back).
        if new.is_some() {
            let old = Encrypted::seal(app.keyring(), &settings_context(), &token)?;
            panel_setting::set(&txn, TOKEN_KEY, old.envelope()).await?;
        }
        panel_setting::set(&txn, ENABLED_KEY, "true").await?;
    }
    txn.commit().await?;
    // Older tokens' secrets, unused now.
    remove_secrets(docker, Some(&secret_name)).await;
    traefik.set_tunnel(true);
    republish(app).await
}

/// Traefik on the tunnel's network (`id`), with the tunnel's entrypoint
/// trusting that network's forwarded headers, and nothing else of it.
/// Restarted only when that changes.
async fn entrypoint(docker: &Docker, traefik: &Traefik, id: &str) -> AppResult<()> {
    // Swarm gives a new overlay its subnet a moment after creating it.
    let mut subnets = vec![];
    for _ in 0..20 {
        subnets = docker
            .engine()
            .network_subnets(NETWORK)
            .await
            .map_err(docker_err)?;
        if !subnets.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    if subnets.is_empty() {
        return Err(AppError::internal("the tunnel's network has no subnet"));
    }
    let trust = trust_arg(&subnets);
    traefik
        .attach(docker, NETWORK, id)
        .await
        .map_err(docker_err)?;
    traefik
        .update_service(docker.engine(), |spec| {
            let args = spec
                .task_template
                .get_or_insert_with(Default::default)
                .container_spec
                .get_or_insert_with(Default::default)
                .args
                .get_or_insert_with(Vec::new);
            let before = args.clone();
            // The installer gives Traefik the address and the alias
            // strategy: then only the trust changes.
            args.retain(|a| {
                !a.starts_with("--entrypoints.tunnel.")
                    || a == TRAEFIK_ARG
                    || a == TRAEFIK_ALIAS_ARG
                    || *a == trust
            });
            for arg in [TRAEFIK_ARG, trust.as_str()] {
                if !args.iter().any(|a| a == arg) {
                    args.push(arg.to_owned());
                }
            }
            *args != before
        })
        .await
        .map_err(docker_err)?;
    Ok(())
}

/// Stops the tunnel `name`'s cloudflared (its token kept); the routes' tunnel
/// routers go when no tunnel is on.
pub async fn disable(app: &AppState, docker: &Docker, name: &str) -> AppResult<()> {
    // Off first (routes without the tunnel), then cloudflared stopped.
    let txn = begin_write(app.db()).await?;
    if let Some(row) = tunnel::named(&txn, name).await? {
        let mut off = row.into_active_model();
        off.enabled = Set(false);
        off.updated_at = Set(chrono::Utc::now());
        off.update(&txn).await?;
    }
    if name == DEFAULT {
        panel_setting::set(&txn, ENABLED_KEY, "false").await?;
    }
    txn.commit().await?;
    let on = any_enabled(app).await?;
    if let Some(traefik) = app.service::<Traefik>() {
        traefik.set_tunnel(on);
    }
    republish(app).await?;
    match docker.engine().delete_service(&service_name(name)).await {
        Ok(()) | Err(DockerError::NotFound) => {}
        Err(err) => return Err(err.into()),
    }
    remove_secrets(docker, None).await;
    Ok(())
}

/// Stops the tunnel `name` and forgets it (its token, and `default`'s
/// settings: not taken back at the next boot).
pub async fn remove(app: &AppState, docker: &Docker, name: &str) -> AppResult<()> {
    let Some(row) = tunnel::named(app.db(), name).await? else {
        return Err(AppError::NotFound);
    };
    disable(app, docker, name).await?;
    let txn = begin_write(app.db()).await?;
    tunnel::Entity::delete_by_id(row.id).exec(&txn).await?;
    if name == DEFAULT {
        panel_setting::Entity::delete_many()
            .filter(panel_setting::Column::Key.is_in([TOKEN_KEY, ENABLED_KEY]))
            .exec(&txn)
            .await?;
    }
    txn.commit().await?;
    Ok(())
}

/// The tunnels' secrets but `keep`. One in use is refused (another
/// tunnel's, held by its cloudflared; or one still stopping: kept until the
/// next change).
async fn remove_secrets(docker: &Docker, keep: Option<&str>) {
    let Ok(secrets) = docker.engine().list_secrets(SYSTEM_LABEL, "true").await else {
        return;
    };
    for (id, name) in secrets {
        if name.starts_with("rdp_tunnel_") && Some(name.as_str()) != keep {
            let _ = docker.engine().delete_secret(&id).await;
        }
    }
}

/// At boot: each tunnel's cloudflared as its row says (run again when on
/// and missing, stopped when off), in the background.
pub fn reconcile(app: &AppState) {
    let app = app.clone();
    tokio::spawn(async move {
        let Some(docker) = app.service::<Docker>() else {
            return;
        };
        let _changing = exclusive().await;
        let rows = match tunnel::all(app.db()).await {
            Ok(rows) => rows,
            Err(err) => {
                tracing::warn!(%err, "the tunnels not read");
                return;
            }
        };
        for row in rows {
            if let Err(err) = reconcile_one(&app, &docker, &row).await {
                tracing::warn!(%err, tunnel = %row.name, "the tunnel not reconciled");
            }
        }
        // Left by a change cut short.
        remove_secrets(&docker, None).await;
    });
}

async fn reconcile_one(app: &AppState, docker: &Docker, row: &tunnel::Model) -> AppResult<()> {
    let found = docker
        .engine()
        .inspect_service(&service_name(&row.name))
        .await
        .ok()
        .flatten();
    let present = found.is_some();
    // Made by an older panel (no healthcheck, another cloudflared): made
    // again as now (Cloudflare stops supporting old versions).
    let older = found.is_some_and(|s| {
        let container = s
            .spec
            .and_then(|spec| spec.task_template)
            .and_then(|t| t.container_spec);
        container.as_ref().is_none_or(|c| {
            c.health_check.is_none()
                || c.image
                    .as_deref()
                    .is_none_or(|i| i.split('@').next() != Some(IMAGE))
        })
    });
    match (row.enabled, present) {
        (true, false) => enable(app, docker, &row.name, None).await,
        (true, true) if older => enable(app, docker, &row.name, None).await,
        (false, true) => disable(app, docker, &row.name).await,
        (false, false) => Ok(()),
        // Traefik's entrypoint as now (an older panel's trusted nobody).
        (true, true) => match app.service::<Traefik>() {
            Some(traefik) if !traefik.service.is_empty() => {
                match docker.engine().networks().await {
                    Ok(networks) => match networks.into_iter().find(|n| n.name == NETWORK) {
                        Some(network) => entrypoint(docker, &traefik, &network.id).await,
                        None => enable(app, docker, &row.name, None).await,
                    },
                    Err(err) => Err(docker_err(err)),
                }
            }
            _ => Ok(()),
        },
    }
}

/// Every service's routes written again (with or without their tunnel
/// routers), and the panel's own.
async fn republish(app: &AppState) -> AppResult<()> {
    let Some(traefik) = app.service::<Traefik>() else {
        return Ok(());
    };
    let options = crate::services::deployment_service::deploy_options(0);
    crate::services::traefik_service::sync_all(app.db(), &traefik, &options).await?;
    write_panel_route(app)
        .await
        .map_err(|err| AppError::internal(format!("the panel's tunnel route: {err}")))
}

/// At boot, after [`load`], and when the certificate resolvers change: the
/// tunnel's own routes (the panel's domain, the challenges) as now.
pub async fn sync_panel_route(app: &AppState) {
    if let Err(err) = write_panel_route(app).await {
        tracing::warn!(%err, "the panel's tunnel route not written");
    }
}

/// Whether the tunnel routes the panel too: installed with a domain (the
/// installer's `rdockpanel@file` route exists only then). An address or
/// `localhost` (a Mac's install without `--domain`) is none.
pub fn routes_the_panel(cfg: &crate::services::deploy_rules_service::PanelConfig) -> bool {
    let host = &cfg.panel_host;
    cfg.installed
        && !host.is_empty()
        && host != "localhost"
        && host.parse::<std::net::IpAddr>().is_err()
}

/// What the tunnel's entrypoint serves beside the services' domains: Let's
/// Encrypt's HTTP challenges (a domain reached only through the tunnel gets
/// its certificate so: the challenge comes in here, not on port 80), and the
/// panel's own domain (the installer's route serves it on `web`/`websecure`)
/// to the installer's `rdockpanel@file` service. Removed when off.
async fn write_panel_route(app: &AppState) -> std::io::Result<()> {
    let cfg = crate::services::deploy_rules_service::config();
    let Some(traefik) = app.service::<Traefik>() else {
        return Ok(());
    };
    let path = traefik.dir.join("rdp-tunnel-panel.yml");
    let host = cfg.panel_host.clone();
    let panel = routes_the_panel(cfg);
    // Traefik answers HTTP challenges (`acme-http@internal`) for the
    // installer's resolver and the panel's HTTP ones.
    let challenges = !cfg.cert_resolver.is_empty() || {
        use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
        crate::models::cert_resolver::Entity::find()
            .filter(crate::models::cert_resolver::Column::Kind.ne("dns"))
            .one(app.db())
            .await
            .is_ok_and(|r| r.is_some())
    };
    let file = traefik
        .tunnel()
        .then(|| tunnel_file(panel.then_some(host.as_str()), challenges))
        .flatten();
    match file {
        Some(yaml) => std::fs::write(path, yaml),
        None => match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        },
    }
}

/// [`write_panel_route`]'s file: the panel's `host` (if any), the challenges
/// (if the installer's resolver answers them). `None`: nothing to serve.
fn tunnel_file(host: Option<&str>, challenges: bool) -> Option<String> {
    if host.is_none() && !challenges {
        return None;
    }
    let mut routers = String::new();
    if challenges {
        routers.push_str(&format!(
            "    rdp-tunnel-acme:\n      rule: \"PathPrefix(`/.well-known/acme-challenge/`)\"\n      entryPoints: [\"{ENTRYPOINT}\"]\n      service: acme-http@internal\n      priority: 2000000\n"
        ));
    }
    let mut middlewares = String::new();
    if let Some(host) = host {
        // Over HTTPS (Cloudflare's: said to the panel), and a visitor over
        // plain HTTP redirected: passwords never cross plain HTTP.
        let plain = PLAIN_VISITOR.replace('"', "\\\"");
        routers.push_str(&format!(
            "    rdockpanel-tunnel:\n      rule: \"Host(`{host}`)\"\n      entryPoints: [\"{ENTRYPOINT}\"]\n      service: rdockpanel@file\n      middlewares: [\"{HTTPS_VISITOR}\"]\n      priority: 1000000\n    rdockpanel-tunnel-http:\n      rule: \"Host(`{host}`) && {plain}\"\n      entryPoints: [\"{ENTRYPOINT}\"]\n      service: rdockpanel@file\n      middlewares: [\"rdockpanel-https@file\"]\n      priority: 1000001\n"
        ));
        middlewares = format!(
            "  middlewares:\n    {HTTPS_VISITOR}:\n      headers:\n        customRequestHeaders:\n          X-Forwarded-Proto: https\n"
        );
    }
    Some(format!("http:\n  routers:\n{routers}{middlewares}"))
}

/// A visitor over HTTPS at Cloudflare, said so to what they reach (Traefik
/// sees the tunnel's plain HTTP).
pub const HTTPS_VISITOR: &str = "rdp-tunnel-proto";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_from_health() {
        let state = |status: &str| json!({"State": {"Health": {"Status": status}}});
        assert_eq!(health_news(&state("healthy")), (true, None));
        let (connected, error) = health_news(&state("unhealthy"));
        assert!(!connected && error.is_some());
        assert_eq!(health_news(&state("starting")), (false, None));
        assert_eq!(health_news(&json!({})), (false, None));
    }

    #[test]
    fn the_tunnel_file_serves_challenges_and_the_panel() {
        assert_eq!(tunnel_file(None, false), None);
        let both = tunnel_file(Some("panel.example.com"), true).unwrap();
        let yaml: serde_json::Value = serde_yaml_ng::from_str(&both).unwrap();
        let routers = &yaml["http"]["routers"];
        assert_eq!(routers["rdp-tunnel-acme"]["service"], "acme-http@internal");
        assert_eq!(routers["rdp-tunnel-acme"]["entryPoints"], json!(["tunnel"]));
        assert_eq!(
            routers["rdockpanel-tunnel"]["rule"],
            "Host(`panel.example.com`)"
        );
        assert_eq!(
            routers["rdockpanel-tunnel"]["middlewares"],
            json!([HTTPS_VISITOR])
        );
        assert!(
            routers["rdockpanel-tunnel-http"]["rule"]
                .as_str()
                .unwrap()
                .ends_with(PLAIN_VISITOR)
        );
        assert_eq!(
            yaml["http"]["middlewares"][HTTPS_VISITOR]["headers"]["customRequestHeaders"]["X-Forwarded-Proto"],
            "https"
        );
        let challenges = tunnel_file(None, true).unwrap();
        let yaml: serde_json::Value = serde_yaml_ng::from_str(&challenges).unwrap();
        assert!(yaml["http"]["routers"].get("rdockpanel-tunnel").is_none());
        let panel = tunnel_file(Some("panel.example.com"), false).unwrap();
        let yaml: serde_json::Value = serde_yaml_ng::from_str(&panel).unwrap();
        assert!(yaml["http"]["routers"].get("rdp-tunnel-acme").is_none());
    }

    #[test]
    fn tunnel_tokens() {
        assert!(valid_token(
            "eyJhIjoiMDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWYiLCJ0IjoiMDEyMzQ1NjctODlhYi1jZGVmLTAxMjMtNDU2Nzg5YWJjZGVmIiwicyI6ImMyVmpjbVYwTFhObFkzSmxkQzF6WldOeVpYUT0ifQ=="
        ));
        assert!(!valid_token(&"A".repeat(60)));
        assert!(!valid_token("not a token"));
        assert!(!valid_token(""));
    }

    #[test]
    fn the_tunnel_routes_the_panel_only_on_its_domain() {
        let cfg =
            |installed: bool, host: &str| crate::services::deploy_rules_service::PanelConfig {
                installed,
                panel_host: host.to_owned(),
                ..Default::default()
            };
        assert!(routes_the_panel(&cfg(true, "panel.example.com")));
        // No domain: an address (Linux) or localhost (a Mac); or from source.
        assert!(!routes_the_panel(&cfg(true, "203.0.113.7")));
        assert!(!routes_the_panel(&cfg(true, "localhost")));
        assert!(!routes_the_panel(&cfg(true, "")));
        assert!(!routes_the_panel(&cfg(false, "panel.example.com")));
    }
}
