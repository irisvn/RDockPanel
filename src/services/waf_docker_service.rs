//! The WAF's Docker side (`waf_service` holds the rules): CrowdSec as the
//! service `rdp-crowdsec`, the plugin in Traefik, the definitions file, and
//! the canary that says whether it all works.
//!
//! [`reconcile`] brings everything to the saved settings, whatever it finds
//! (it is run at boot and after each change): nothing in CrowdSec must
//! survive, so any panel version takes it over.
//! - off: CrowdSec scaled to 0, no-op definitions, routes without the WAF;
//! - on: network `rdp-crowdsec` (CrowdSec and Traefik only); the plugin
//!   copied from the panel's image into the volume `rdp_traefik_plugins`
//!   (read-only in Traefik); CrowdSec's files as Swarm configs; the bouncer
//!   key only in a Swarm secret (CrowdSec reads it into its environment at
//!   start, Traefik reads the file); Traefik given the plugin; definitions
//!   written; then the canary.
//!
//! [`check`] (every minute) sends a normal request and an attack through the
//! canary router: a 404 means the plugin is not loaded (the WAF middlewares
//! become no-ops, so no site answers 404); an attack let through means AppSec
//! does not answer (the plugin lets requests through: fail-open).

use std::collections::HashMap;

use rustcms::prelude::*;
use rustcms::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::services::docker_service::{
    Docker, DockerError, HelperMount, HelperSpec, NetworkRequest, SYSTEM_LABEL,
};
use crate::services::traefik_service::Traefik;
use crate::services::waf_service::{
    self, APPSEC_BLOCK_PORT, APPSEC_DETECT_PORT, CROWDSEC, Level, Mode, NETWORK, PLUGIN, Settings,
};

/// CrowdSec, pinned (its digest: the same image on every server).
pub const IMAGE: &str = "crowdsecurity/crowdsec:v1.8.2@sha256:65b74942f7e7ed5c60fde4ac15fc4ba715904c398c8b16dd451f4907d90347d4";
/// The plugin's volume, where Traefik reads it, and the panel image's copy.
pub const PLUGIN_VOLUME: &str = "rdp_traefik_plugins";
const PLUGIN_MOUNT: &str = "/plugins-local";
const PLUGIN_SOURCE: &str = "/app/waf/plugin";
/// CrowdSec's hub and its data files, bundled in the panel's image.
const HUB_SOURCE: &str = "/app/waf/hub";
const HUB_DATA_SOURCE: &str = "/app/waf/hub-data";
/// CrowdSec's volumes (its database, its hub content).
const DATA_VOLUME: &str = "rdp_crowdsec_data";
const CONFIG_VOLUME: &str = "rdp_crowdsec_config";
/// The bouncer key's secrets: `rdp_crowdsec_bouncer_<hash>`.
const KEY_SECRET: &str = "rdp_crowdsec_bouncer";
/// The canary's host (never a real domain) and its backend path in the panel.
pub const CANARY_HOST: &str = "rdp-waf-canary.invalid";
/// Its twin through the Detect listener (it never blocks: CrowdSec's
/// counters prove it checked).
pub const CANARY_DETECT_HOST: &str = "rdp-waf-canary-detect.invalid";
/// An SQL injection only the Strict level (the OWASP Core Rule Set) stops.
const STRICT_PROBE: &str = "/?id=1%27%20OR%20%271%27%3D%271";
const CANARY_PRIORITY: i64 = 1_000_000_000;
/// What the panel's canary route answers.
pub const CANARY_ANSWER: &str = "rdp-waf-canary";
pub const CANARY_PATH: &str = "/api/v1/waf-canary";
/// Where the WAF's state is kept (`panel_settings`).
const STATUS_KEY: &str = "waf.status";
/// Collections installed at start (Strict uses the CRS one).
const COLLECTIONS: &str = "crowdsecurity/appsec-virtual-patching crowdsecurity/appsec-generic-rules crowdsecurity/appsec-crs";

/// What the WAF is doing, as the Settings page shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Status {
    /// `off`, `starting`, `on`, `unavailable`.
    pub state: String,
    /// Why, when not `on`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub checked_at: Option<chrono::DateTime<chrono::Utc>>,
    /// The sites' WAF middlewares are the plugin (else no-ops): set only once
    /// the canary proved the plugin loaded.
    #[serde(default)]
    pub applied: bool,
    /// While starting: the step under way (1 to [`STEPS`]' length), since
    /// when, and the action whose log says more.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<i64>,
}

impl Status {
    pub async fn load<C: sea_orm::ConnectionTrait>(db: &C) -> AppResult<Self> {
        Ok(crate::models::panel_setting::get(db, STATUS_KEY)
            .await?
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_else(|| Status {
                state: "off".into(),
                ..Status::default()
            }))
    }

    pub async fn store<C: sea_orm::ConnectionTrait>(&self, db: &C) -> AppResult<()> {
        let text = serde_json::to_string(self).map_err(AppError::internal)?;
        crate::models::panel_setting::set(db, STATUS_KEY, &text).await?;
        Ok(())
    }

    pub fn now(state: &str, detail: Option<String>, applied: bool) -> Self {
        Status {
            state: state.to_owned(),
            detail,
            checked_at: Some(chrono::Utc::now()),
            applied,
            ..Status::default()
        }
    }

    /// Starting, at `step`, since `started_at`, logged by `action`.
    pub fn starting(
        step: u8,
        started_at: chrono::DateTime<chrono::Utc>,
        action: Option<i64>,
    ) -> Self {
        Status {
            state: "starting".to_owned(),
            checked_at: Some(chrono::Utc::now()),
            step: Some(step),
            started_at: Some(started_at),
            action,
            ..Status::default()
        }
    }
}

/// A reconcile's steps, as the page and its action's log name them.
pub const STEPS: [&str; 4] = [
    "Copying the WAF's plugin and rules",
    "Starting CrowdSec",
    "Giving Traefik the plugin",
    "Checking (CrowdSec loads its rules: about half a minute)",
];

/// How long a whole reconcile may take (the lock, Docker, the checks).
const DEADLINE: std::time::Duration = std::time::Duration::from_secs(180);

/// The action a reconcile reports to (`waf.reconcile`): one is made when
/// none was (the boot's, the minute's mending).
pub const ACTION: &str = "waf.reconcile";

/// A reconcile's progress: the stored state (the page polls it) and its
/// action's log, step by step.
struct Progress<'a> {
    app: &'a AppState,
    row: Option<crate::models::action::Model>,
    log: String,
    started: chrono::DateTime<chrono::Utc>,
}

impl Progress<'_> {
    async fn step(&mut self, n: u8) {
        let label = STEPS[usize::from(n) - 1];
        self.log.push_str(&format!("{n}. {label}…\n"));
        let ours = self.row.as_ref().map(|r| r.id);
        // A newer start or stop announced meanwhile (it waits for this one)
        // keeps the state it showed: actions' ids only grow.
        let newer = match (Status::load(self.app.db()).await, ours) {
            (Ok(now), Some(ours)) => now.action.is_some_and(|theirs| theirs > ours),
            _ => false,
        };
        if !newer {
            let status = Status::starting(n, self.started, ours);
            if let Err(err) = status.store(self.app.db()).await {
                tracing::warn!(%err, "the WAF's progress not stored");
            }
        }
        self.note().await;
    }

    async fn note(&mut self) {
        if let Some(row) = self.row.take() {
            match crate::services::action_service::note(self.app.db(), row.clone(), &self.log).await
            {
                Ok(row) => self.row = Some(row),
                Err(err) => {
                    tracing::warn!(%err, "the WAF's action not noted");
                    self.row = Some(row);
                }
            }
        }
    }

    async fn finish(&mut self, error: Option<String>) {
        match &error {
            None => self.log.push_str("Done.\n"),
            Some(err) => self.log.push_str(&format!("Failed: {err}\n")),
        }
        if let Some(row) = self.row.take()
            && let Err(err) =
                crate::services::action_service::finish(self.app.db(), row, error, Some(&self.log))
                    .await
        {
            tracing::warn!(%err, "the WAF's action not finished");
        }
    }
}

fn docker_err(e: DockerError) -> AppError {
    AppError::internal(e.to_string())
}

fn system_labels() -> HashMap<String, String> {
    HashMap::from([(SYSTEM_LABEL.to_owned(), "true".to_owned())])
}

/// CrowdSec's files: the two AppSec listeners (Block's config by level),
/// the appsec configs and the Detect scenario (path in the container, text).
fn crowdsec_files(settings: &Settings) -> Vec<(&'static str, String)> {
    let (block, detect) = match settings.level {
        Level::Basic => ("rdp/appsec-block", "rdp/appsec-detect"),
        Level::Strict => ("rdp/appsec-strict", "rdp/appsec-detect-strict"),
    };
    let listener = |config: &str, port: u16| {
        format!(
            "appsec_configs:\n  - {config}\nlabels:\n  type: appsec\nlisten_addr: 0.0.0.0:{port}\nsource: appsec\n"
        )
    };
    // Every appsec config spares the domains their exceptions' rules.
    let hooks = waf_service::exception_hooks(&settings.exceptions);
    let appsec = |text: &str| {
        if hooks.is_empty() {
            text.to_owned()
        } else {
            format!("{}\n{hooks}", text.trim_end())
        }
    };
    vec![
        (
            "/etc/crowdsec/acquis.d/rdp-appsec-block.yaml",
            listener(block, APPSEC_BLOCK_PORT),
        ),
        (
            "/etc/crowdsec/acquis.d/rdp-appsec-detect.yaml",
            listener(detect, APPSEC_DETECT_PORT),
        ),
        (
            "/etc/crowdsec/appsec-configs/rdp-block.yaml",
            appsec(include_str!("../../resources/waf/crowdsec/rdp-block.yaml")),
        ),
        (
            "/etc/crowdsec/appsec-configs/rdp-strict.yaml",
            appsec(include_str!("../../resources/waf/crowdsec/rdp-strict.yaml")),
        ),
        (
            "/etc/crowdsec/appsec-configs/rdp-detect.yaml",
            appsec(include_str!("../../resources/waf/crowdsec/rdp-detect.yaml")),
        ),
        (
            "/etc/crowdsec/appsec-configs/rdp-detect-strict.yaml",
            appsec(include_str!(
                "../../resources/waf/crowdsec/rdp-detect-strict.yaml"
            )),
        ),
        (
            "/etc/crowdsec/config.yaml.local",
            include_str!("../../resources/waf/crowdsec/config.yaml.local").to_owned(),
        ),
        (
            "/etc/crowdsec/profiles.yaml",
            include_str!("../../resources/waf/crowdsec/profiles.yaml").to_owned(),
        ),
        (
            "/etc/crowdsec/scenarios/rdp-appsec-detect.yaml",
            include_str!("../../resources/waf/crowdsec/rdp-appsec-detect-scenario.yaml").to_owned(),
        ),
    ]
}

fn short_hash(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes)[..6]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The proxies whose `X-Forwarded-For` names the visitor, as Traefik's
/// entry points trust them: Cloudflare's ranges and the tunnel's network.
async fn trusted(docker: &Docker) -> Vec<String> {
    let mut list: Vec<String> = crate::services::cloudflare_ranges_service::ranges();
    if let Ok(subnets) = docker
        .engine()
        .network_subnets(crate::services::tunnel_service::NETWORK)
        .await
    {
        list.extend(subnets);
    }
    list
}

/// Writes `rdp-waf.yml`: the plugin's middlewares when `healthy` (and the
/// canary's router), no-ops otherwise.
pub async fn write_definitions(
    traefik: &Traefik,
    docker: &Docker,
    settings: &Settings,
    healthy: bool,
) -> AppResult<()> {
    let backend = crate::services::deploy_rules_service::config()
        .internal_url
        .trim_end_matches('/')
        .to_owned();
    let mut file = waf_service::definitions(settings, healthy, &trusted(docker).await);
    if settings.mode != Mode::Off && !backend.is_empty() {
        let http = &mut file["http"];
        http["middlewares"]["rdp-waf-canary-path"] = json!({"replacePath": {"path": CANARY_PATH}});
        let router = |host: &str, middleware: &str| {
            json!({
                "rule": format!("Host(`{host}`)"),
                // Above every service's router (theirs are their rule's length
                // times 4, at most about 40 000).
                "priority": CANARY_PRIORITY,
                "entryPoints": [crate::services::deploy_service::HTTP_ENTRYPOINT],
                "middlewares": ["rdp-waf-buffer", middleware, "rdp-waf-canary-path"],
                "service": "rdp-waf-canary",
            })
        };
        http["routers"] = json!({
            "rdp-waf-canary": router(CANARY_HOST, "rdp-waf-canary"),
            "rdp-waf-canary-detect": router(CANARY_DETECT_HOST, "rdp-waf-canary-detect"),
        });
        http["services"] =
            json!({"rdp-waf-canary": {"loadBalancer": {"servers": [{"url": backend}]}}});
    }
    let yaml = serde_yaml_ng::to_string(&file).map_err(AppError::internal)?;
    let dir = traefik.dir.clone();
    tokio::task::spawn_blocking(move || {
        std::fs::create_dir_all(&dir)?;
        // The same text: not written again (Traefik would reload for nothing).
        if std::fs::read(dir.join(waf_service::FILE)).ok().as_deref() == Some(yaml.as_bytes()) {
            return Ok(());
        }
        crate::services::certificate_service::write_private(
            &dir,
            waf_service::FILE,
            yaml.as_bytes(),
        )
    })
    .await
    .map_err(AppError::internal)?
    .map_err(AppError::internal)
}

/// The definitions written again as they are (the plugin's when proved,
/// else no-ops) with what they depend on now (Cloudflare's ranges).
pub async fn refresh_definitions(app: &AppState) -> AppResult<()> {
    let (Some(docker), Some(traefik)) = (app.service::<Docker>(), app.service::<Traefik>()) else {
        return Ok(());
    };
    let settings = Settings::load(app.db()).await?;
    let applied = Status::load(app.db()).await?.applied;
    write_definitions(&traefik, &docker, &settings, applied).await
}

/// One reconcile at a time (a save, Repair and the boot may overlap); the
/// schedule's check waits for none and skips while one runs.
pub(crate) static RECONCILING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Brings CrowdSec, Traefik and the definitions to the saved settings; then
/// every service's routes are written again (their WAF on or off).
pub async fn reconcile(app: &AppState) -> AppResult<Status> {
    reconcile_with(app, false, None).await
}

/// [`reconcile`], with a new bouncer key when `rotate`: CrowdSec, then
/// Traefik given it; once the canary proves it, the previous bouncers and
/// secrets go. Interrupted, the previous key still works (its bouncer stays
/// until then), and the next reconcile carries on with the key Traefik has.
/// It reports to `action` (made by the caller with its audit), else to one
/// of its own; three minutes at most, all of it.
pub async fn reconcile_with(
    app: &AppState,
    rotate: bool,
    action: Option<crate::models::action::Model>,
) -> AppResult<Status> {
    let row = match action {
        Some(row) => Some(row),
        None => crate::services::action_service::begin(app.db(), ACTION, CROWDSEC)
            .await
            .map_err(|err| tracing::warn!(%err, "the WAF's action not started"))
            .ok(),
    };
    // One at a time, from its first step to its recorded result: another
    // start waits here (its deadline not running yet), so no two write their
    // steps or results over each other.
    let _one = RECONCILING.lock().await;
    let mut progress = Progress {
        app,
        row,
        log: String::new(),
        started: chrono::Utc::now(),
    };
    let before = Status::load(app.db()).await?;
    let outcome = tokio::time::timeout(DEADLINE, run_reconcile(app, rotate, &mut progress)).await;
    let status = match outcome {
        Ok(Ok(status)) => status,
        Ok(Err(err)) => Status::now("unavailable", Some(err.to_string()), false),
        // The deadline: what the canary says now.
        Err(_) => match verdict(app, None).await {
            Ok(Some(now)) if now.state == "on" => now,
            Ok(Some(now)) => Status::now(
                "unavailable",
                Some(
                    now.detail
                        .unwrap_or_else(|| "the WAF did not start within three minutes".to_owned()),
                ),
                now.applied,
            ),
            _ => Status::now(
                "unavailable",
                Some("the WAF did not start within three minutes".to_owned()),
                false,
            ),
        },
    };
    // A newer start or stop announced meanwhile records its own result
    // (it runs next): this one's would only flash in between.
    let ours = progress.row.as_ref().map(|r| r.id);
    let superseded = match (Status::load(app.db()).await, ours) {
        (Ok(now), Some(ours)) => now.action.is_some_and(|theirs| theirs > ours),
        _ => false,
    };
    if !superseded {
        record(app, &before, &status).await?;
    }
    let error = match status.state.as_str() {
        "on" | "off" => None,
        _ => Some(
            status
                .detail
                .clone()
                .unwrap_or_else(|| status.state.clone()),
        ),
    };
    progress.finish(error).await;
    Ok(status)
}

/// The reconcile itself, step by step (its deadline and its lock are the
/// caller's).
async fn run_reconcile(
    app: &AppState,
    rotate: bool,
    progress: &mut Progress<'_>,
) -> AppResult<Status> {
    let docker = app
        .service::<Docker>()
        .ok_or_else(|| AppError::internal("Docker is not bound"))?;
    let traefik = app
        .service::<Traefik>()
        .ok_or_else(|| AppError::internal("Traefik is not bound"))?;
    let settings = Settings::load(app.db()).await?;
    let before = Status::load(app.db()).await?;
    let options = crate::services::deployment_service::deploy_options(0);
    // The sites' middlewares are no-ops until the canary proves the plugin
    // (a site never answers 404 meanwhile; requests go through unchecked).
    write_definitions(&traefik, &docker, &settings, false).await?;
    if settings.mode == Mode::Off {
        progress
            .log
            .push_str("Stopping CrowdSec; the routes go without the WAF.\n");
        scale_crowdsec(&docker, 0).await?;
        crate::services::traefik_service::sync_all(app.db(), &traefik, &options).await?;
        return Ok(Status::now("off", None, false));
    }
    if traefik.service.is_empty() {
        return Err(AppError::BadRequest(
            "Traefik is not a Swarm service of the panel's here".into(),
        ));
    }
    progress.step(1).await;
    let network = docker
        .ensure_network(NetworkRequest {
            name: NETWORK.to_owned(),
            driver: "overlay".to_owned(),
            labels: system_labels(),
            attachable: false,
            internal: false,
        })
        .await
        .map_err(docker_err)?;
    seed(app, &docker, settings.rules).await?;
    progress.step(2).await;
    let key_secret = bouncer_secret(&docker, &traefik, rotate).await?;
    run_crowdsec(&docker, &settings, &network.id, &key_secret).await?;
    progress.step(3).await;
    // Traefik loads its plugin when it starts: after a failure to load it,
    // it is started again (the plugin was copied again).
    let restart = before.detail.as_deref() == Some(PLUGIN_NOT_LOADED);
    give_traefik_the_plugin(&docker, &traefik, &network.id, &key_secret, restart).await?;
    crate::services::traefik_service::sync_all(app.db(), &traefik, &options).await?;
    progress.step(4).await;
    // The canary every 5 s until it says on (the deadline is the caller's);
    // what it says meanwhile is not stored: the state stays "starting".
    let mut restarted = false;
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        let Some(now) = verdict(app, None).await? else {
            return Status::load(app.db()).await;
        };
        if now.state == "on" {
            // The key in use proved: the others go.
            if let Err(err) = forget_other_keys(&docker, &traefik).await {
                tracing::warn!(%err, "the WAF's previous keys not removed");
            }
            return Ok(now);
        }
        // Traefik loaded the plugin before it was copied again: started
        // again once, it loads this copy.
        if now.detail.as_deref() == Some(PLUGIN_NOT_LOADED) && !restarted {
            restarted = true;
            progress
                .log
                .push_str("Traefik started again to load the plugin.\n");
            progress.note().await;
            restart_traefik(&docker, &traefik).await?;
        }
    }
}

/// Copies from the panel's image (a helper container of it) the plugin into
/// its volume, and CrowdSec's rules (the hub bundled at build time, and their
/// data files) into CrowdSec's volumes: it starts from them, never from the
/// network.
async fn seed(app: &AppState, docker: &Docker, rules: waf_service::RulesSource) -> AppResult<()> {
    let image = crate::services::import_service::helper_image(app, docker)
        .await
        .ok_or_else(|| {
            AppError::BadRequest(
                "the panel's own image is unknown (not the installer's service): the WAF's plugin cannot be copied".into(),
            )
        })?;
    for volume in [PLUGIN_VOLUME, CONFIG_VOLUME, DATA_VOLUME] {
        let _ = docker
            .engine()
            .create_volume_labelled(volume, system_labels())
            .await;
    }
    // Copied beside, checked, then swapped in: Traefik starting meanwhile
    // reads the old copy or the new one, never half of one.
    let hub = match rules {
        // The bundle: the hub and its data copied over what is there.
        waf_service::RulesSource::Bundled => format!(
            "; test -f {HUB_SOURCE}/.index.json; rm -rf /crowdsec/.hub-new; \
             cp -r {HUB_SOURCE} /crowdsec/.hub-new; rm -rf /crowdsec/hub; \
             mv /crowdsec/.hub-new /crowdsec/hub; \
             cp -r {HUB_DATA_SOURCE}/. /crowdsec-data/"
        ),
        // The latest rules an administrator installed: kept.
        waf_service::RulesSource::Latest => String::new(),
    };
    let script = format!(
        "set -e; test -f {PLUGIN_SOURCE}/VERSION; rm -rf /plugins/.new /plugins/.old; \
         cp -r {PLUGIN_SOURCE}/src /plugins/.new; test -f /plugins/.new/{module}/go.mod; \
         if [ -d /plugins/src ]; then mv /plugins/src /plugins/.old; fi; \
         mv /plugins/.new /plugins/src; rm -rf /plugins/.old; \
         cp {PLUGIN_SOURCE}/VERSION /plugins/VERSION{hub}",
        module = waf_service::MODULE
    );
    let code = docker
        .engine()
        .run_helper(HelperSpec {
            // Its own name each time: one left behind by a crash never blocks.
            name: format!(
                "rdp-waf-seed-{}",
                crate::services::database_service::random_string(8)
            ),
            image,
            cmd: vec!["sh".into(), "-c".into(), script],
            mounts: [
                (PLUGIN_VOLUME, "/plugins"),
                (CONFIG_VOLUME, "/crowdsec"),
                (DATA_VOLUME, "/crowdsec-data"),
            ]
            .into_iter()
            .map(|(source, target)| HelperMount {
                source: source.to_owned(),
                target: target.to_owned(),
                read_only: false,
                kind: "volume".to_owned(),
            })
            .collect(),
            import_id: 0,
            cap_add: vec![],
            timeout_s: 60,
        })
        .await
        .map_err(docker_err)?;
    if code != 0 {
        return Err(AppError::internal(format!(
            "the WAF's plugin and rules could not be copied (exit {code})"
        )));
    }
    Ok(())
}

/// The bouncer key's secret: the one Traefik reads (else any left), or a
/// new one (a random key, never kept elsewhere) when there is none or when
/// `rotate`: (id, name).
async fn bouncer_secret(
    docker: &Docker,
    traefik: &Traefik,
    rotate: bool,
) -> AppResult<(String, String)> {
    let secrets: Vec<(String, String)> = docker
        .engine()
        .list_secrets(SYSTEM_LABEL, "true")
        .await
        .map_err(docker_err)?
        .into_iter()
        .filter(|(_, n)| n.starts_with(KEY_SECRET))
        .collect();
    if !rotate {
        let in_use = traefiks_key(docker, traefik).await;
        if let Some(found) = secrets
            .iter()
            .find(|(_, n)| Some(n) == in_use.as_ref())
            .or_else(|| secrets.first())
        {
            return Ok(found.clone());
        }
    }
    let key = crate::services::database_service::random_string(40);
    let name = format!("{KEY_SECRET}_{}", short_hash(key.as_bytes()));
    let id = docker
        .engine()
        .create_secret(&name, key.as_bytes(), system_labels())
        .await
        .map_err(docker_err)?;
    Ok((id, name))
}

/// The bouncer keys' secrets there are: (id, name).
async fn keys(docker: &Docker) -> Vec<(String, String)> {
    docker
        .engine()
        .list_secrets(SYSTEM_LABEL, "true")
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|(_, n)| n.starts_with(KEY_SECRET))
        .collect()
}

/// The bouncer key's secret Traefik mounts (its name).
async fn traefiks_key(docker: &Docker, traefik: &Traefik) -> Option<String> {
    let found = docker
        .engine()
        .inspect_service(&traefik.service)
        .await
        .ok()??;
    found
        .spec?
        .task_template?
        .container_spec?
        .secrets?
        .into_iter()
        .find(|s| s.file.as_ref().and_then(|f| f.name.as_deref()) == Some(KEY_SECRET))?
        .secret_name
}

/// The bouncer's name in CrowdSec for a key's secret.
fn bouncer_name(secret: &str) -> String {
    format!(
        "rdpwaf{}",
        secret.trim_start_matches(KEY_SECRET).trim_matches('_')
    )
}

/// Once the key Traefik reads is proved: CrowdSec's other bouncers of the
/// panel and their secrets removed (a rotation's previous key, or one an
/// interrupted rotation left).
async fn forget_other_keys(docker: &Docker, traefik: &Traefik) -> AppResult<()> {
    let Some(in_use) = traefiks_key(docker, traefik).await else {
        return Ok(());
    };
    // Every running Traefik task has the key in use (an old one still
    // serving would still need the previous key): else later.
    let tasks = docker
        .engine()
        .list_tasks(&traefik.service)
        .await
        .map_err(docker_err)?;
    let running: Vec<_> = tasks
        .iter()
        .filter(|t| {
            t.status.as_ref().and_then(|s| s.state)
                == Some(crate::services::docker_service::TaskState::RUNNING)
        })
        .collect();
    let converged = !running.is_empty()
        && running.iter().all(|t| {
            t.spec
                .as_ref()
                .and_then(|s| s.container_spec.as_ref())
                .and_then(|c| c.secrets.as_ref())
                .is_some_and(|secrets| {
                    secrets
                        .iter()
                        .any(|s| s.secret_name.as_deref() == Some(in_use.as_str()))
                })
        });
    if !converged {
        return Ok(());
    }
    let keep = bouncer_name(&in_use);
    if let Some(text) = cscli(docker, &["bouncers", "list", "-o", "json"]).await? {
        let list: Vec<serde_json::Value> = serde_json::from_str(text.trim()).unwrap_or_default();
        for name in list.iter().filter_map(|b| b["name"].as_str()) {
            if name.starts_with("rdpwaf") && name != keep {
                cscli(docker, &["bouncers", "delete", name]).await?;
            }
        }
    }
    for (id, name) in docker
        .engine()
        .list_secrets(SYSTEM_LABEL, "true")
        .await
        .map_err(docker_err)?
    {
        if name.starts_with(KEY_SECRET) && name != in_use {
            // Still used by a service: left for the next time.
            let _ = docker.engine().delete_secret(&id).await;
        }
    }
    Ok(())
}

/// CrowdSec's Swarm configs (named by their content: a change is a new one).
async fn configs(docker: &Docker, settings: &Settings) -> AppResult<Vec<serde_json::Value>> {
    let existing = docker
        .engine()
        .list_configs(SYSTEM_LABEL, "true")
        .await
        .map_err(docker_err)?;
    let mut out = vec![];
    for (target, content) in crowdsec_files(settings) {
        let base = target
            .rsplit('/')
            .next()
            .unwrap_or("file")
            .replace('.', "-");
        let name = format!("rdp_waf_{base}_{}", short_hash(content.as_bytes()));
        let id = match existing.iter().find(|(_, n)| *n == name) {
            Some((id, _)) => id.clone(),
            None => match docker
                .engine()
                .create_config(&name, content.as_bytes(), system_labels())
                .await
            {
                Ok(id) => id,
                // Made meanwhile by another reconcile.
                Err(DockerError::Conflict(_)) => docker
                    .engine()
                    .list_configs(SYSTEM_LABEL, "true")
                    .await
                    .map_err(docker_err)?
                    .into_iter()
                    .find(|(_, n)| *n == name)
                    .map(|(id, _)| id)
                    .ok_or_else(|| AppError::internal("a WAF config vanished"))?,
                Err(e) => return Err(docker_err(e)),
            },
        };
        out.push(json!({
            "ConfigID": id, "ConfigName": name,
            "File": {"Name": target, "UID": "0", "GID": "0", "Mode": 0o444},
        }));
    }
    Ok(out)
}

async fn run_crowdsec(
    docker: &Docker,
    settings: &Settings,
    network: &str,
    key: &(String, String),
) -> AppResult<()> {
    let configs = configs(docker, settings).await?;
    let bouncer = bouncer_name(&key.1);
    let mut env = vec![
        format!("COLLECTIONS={COLLECTIONS}"),
        // Installed from the bundled hub, never upgraded from the network.
        "NO_HUB_UPGRADE=true".to_owned(),
        // A new panel's bundle: CrowdSec started again to load it.
        format!("RDP_WAF_BUNDLE={}", env!("CARGO_PKG_VERSION")),
        // Switching between the bundle and the latest rules restarts it.
        format!(
            "RDP_WAF_RULES={}",
            match settings.rules {
                waf_service::RulesSource::Bundled => "bundled",
                waf_service::RulesSource::Latest => "latest",
            }
        ),
    ];
    if !settings.sharing {
        // Nothing sent to CrowdSec's online API (the image registers by default).
        env.push("DISABLE_ONLINE_API=true".to_owned());
    }
    let spec: crate::services::docker_service::ServiceSpec = serde_json::from_value(json!({
        "Name": CROWDSEC,
        "Labels": { SYSTEM_LABEL: "true" },
        "TaskTemplate": {
            "ContainerSpec": {
                "Image": IMAGE,
                // The key from its secret into the variable the image registers
                // the bouncer from; never in the service's spec.
                "Command": ["/bin/bash", "-c",
                    // The hub's index kept fresh: the start script never fetches
                    // another (the rules are the bundled ones).
                    format!("export BOUNCER_KEY_{bouncer}=\"$(cat /run/secrets/{KEY_SECRET})\"; touch /etc/crowdsec/hub/.index.json 2>/dev/null; exec /bin/bash /docker_start.sh")],
                "Env": env,
                "Labels": { SYSTEM_LABEL: "true" },
                "Mounts": [
                    {"Type": "volume", "Source": DATA_VOLUME, "Target": "/var/lib/crowdsec/data"},
                    {"Type": "volume", "Source": CONFIG_VOLUME, "Target": "/etc/crowdsec"},
                ],
                "Configs": configs,
                "Secrets": [{
                    "SecretID": key.0, "SecretName": key.1,
                    "File": {"Name": KEY_SECRET, "UID": "0", "GID": "0", "Mode": 0o400},
                }],
            },
            "Networks": [{ "Target": network }],
            "Placement": { "Constraints": ["node.labels.rdockpanel.main==true"] },
            "Resources": {
                "Limits": { "NanoCPUs": 500_000_000u64, "MemoryBytes": 300u64 * 1024 * 1024 },
                "Reservations": { "MemoryBytes": 100u64 * 1024 * 1024 },
            },
            "RestartPolicy": { "Condition": "any", "Delay": 5_000_000_000u64 },
        },
        "Mode": { "Replicated": { "Replicas": 1 } },
    }))
    .map_err(AppError::internal)?;
    match docker
        .engine()
        .inspect_service(CROWDSEC)
        .await
        .map_err(docker_err)?
    {
        Some(found) => {
            let version = found.version.and_then(|v| v.index).unwrap_or(0);
            docker
                .engine()
                .update_service(CROWDSEC, version, spec, None)
                .await
                .map_err(docker_err)?;
        }
        None => {
            docker
                .engine()
                .create_service(spec, None)
                .await
                .map_err(docker_err)?;
        }
    }
    Ok(())
}

async fn scale_crowdsec(docker: &Docker, replicas: u64) -> AppResult<()> {
    let Some(found) = docker
        .engine()
        .inspect_service(CROWDSEC)
        .await
        .map_err(docker_err)?
    else {
        return Ok(());
    };
    let mut spec = found.spec.clone().unwrap_or_default();
    let current = spec
        .mode
        .as_ref()
        .and_then(|m| m.replicated.as_ref())
        .and_then(|r| r.replicas);
    if current == Some(i64::try_from(replicas).unwrap_or(0)) {
        return Ok(());
    }
    spec.mode = Some(
        serde_json::from_value(json!({"Replicated": {"Replicas": replicas}}))
            .map_err(AppError::internal)?,
    );
    let version = found.version.and_then(|v| v.index).unwrap_or(0);
    docker
        .engine()
        .update_service(CROWDSEC, version, spec, None)
        .await
        .map_err(docker_err)?;
    Ok(())
}

/// Traefik with the plugin: its volume (read-only), the key's secret, the
/// network to CrowdSec and the static argument. Only these items change.
async fn give_traefik_the_plugin(
    docker: &Docker,
    traefik: &Traefik,
    network: &str,
    key: &(String, String),
    restart: bool,
) -> AppResult<()> {
    traefik
        .attach(docker, NETWORK, network)
        .await
        .map_err(docker_err)?;
    let arg = format!(
        "--experimental.localPlugins.{PLUGIN}.moduleName={}",
        waf_service::MODULE
    );
    let (secret_id, secret_name) = key.clone();
    traefik
        .update_service(docker.engine(), move |spec| {
            let template = spec.task_template.get_or_insert_with(Default::default);
            let mut changed = false;
            if restart {
                // Started again: its plugin loaded from the copy just made.
                template.force_update = Some(template.force_update.unwrap_or(0) + 1);
                changed = true;
            }
            let container = template.container_spec.get_or_insert_with(Default::default);
            let args = container.args.get_or_insert_with(Vec::new);
            if !args.contains(&arg) {
                args.retain(|a| !a.starts_with(&format!("--experimental.localPlugins.{PLUGIN}.")));
                args.push(arg.clone());
                changed = true;
            }
            let mounts = container.mounts.get_or_insert_with(Vec::new);
            if !mounts.iter().any(|m| m.target.as_deref() == Some(PLUGIN_MOUNT)) {
                mounts.push(
                    serde_json::from_value(json!({"Type": "volume", "Source": PLUGIN_VOLUME, "Target": PLUGIN_MOUNT, "ReadOnly": true}))
                        .expect("a mount"),
                );
                changed = true;
            }
            let secrets = container.secrets.get_or_insert_with(Vec::new);
            let file = KEY_SECRET.to_owned();
            if !secrets
                .iter()
                .any(|s| s.secret_name.as_deref() == Some(&secret_name))
            {
                // The previous key's mount replaced (same file name).
                secrets.retain(|s| {
                    s.file.as_ref().and_then(|f| f.name.as_deref()) != Some(file.as_str())
                });
                secrets.push(
                    serde_json::from_value(json!({
                        "SecretID": secret_id, "SecretName": secret_name,
                        "File": {"Name": file, "UID": "0", "GID": "0", "Mode": 0o400},
                    }))
                    .expect("a secret reference"),
                );
                changed = true;
            }
            changed
        })
        .await
        .map_err(docker_err)?;
    Ok(())
}

/// CrowdSec started again (its rules loaded anew), nothing else changed.
pub async fn restart_crowdsec(docker: &Docker) -> AppResult<()> {
    let Some(found) = docker
        .engine()
        .inspect_service(CROWDSEC)
        .await
        .map_err(docker_err)?
    else {
        return Err(AppError::BadRequest(
            "CrowdSec is not running: turn the WAF on".into(),
        ));
    };
    let mut spec = found.spec.clone().unwrap_or_default();
    let template = spec.task_template.get_or_insert_with(Default::default);
    template.force_update = Some(template.force_update.unwrap_or(0) + 1);
    let version = found.version.and_then(|v| v.index).unwrap_or(0);
    docker
        .engine()
        .update_service(CROWDSEC, version, spec, None)
        .await
        .map_err(docker_err)?;
    Ok(())
}

/// Traefik started again (its plugin loaded anew), nothing else changed.
async fn restart_traefik(docker: &Docker, traefik: &Traefik) -> AppResult<()> {
    traefik
        .update_service(docker.engine(), |spec| {
            let template = spec.task_template.get_or_insert_with(Default::default);
            template.force_update = Some(template.force_update.unwrap_or(0) + 1);
            true
        })
        .await
        .map_err(docker_err)?;
    Ok(())
}

/// The schedule's check: [`probe`], unless a reconcile runs (it probes).
/// A plugin Traefik did not load (Traefik restarted by anyone, the
/// installer included) is mended by a reconcile, which starts Traefik again:
/// at most once every ten minutes.
pub async fn check(app: &AppState) -> AppResult<Status> {
    use std::sync::atomic::{AtomicI64, Ordering};
    static LAST_MEND: AtomicI64 = AtomicI64::new(0);
    let status = {
        let Ok(_one) = RECONCILING.try_lock() else {
            return Status::load(app.db()).await;
        };
        let status = probe(app, None).await?;
        // A rotation the reconcile's wait did not see finish: its previous
        // key goes once the new one works.
        if status.state == "on"
            && let (Some(docker), Some(traefik)) =
                (app.service::<Docker>(), app.service::<Traefik>())
            && keys(&docker).await.len() > 1
            && let Err(err) = forget_other_keys(&docker, &traefik).await
        {
            tracing::warn!(%err, "the WAF's previous keys not removed");
        }
        // The minute's records (CrowdSec may run while the WAF is not on).
        if status.state != "off"
            && let Some(docker) = app.service::<Docker>()
        {
            keep_records(app, &docker).await;
        }
        status
    };
    let now = chrono::Utc::now().timestamp();
    if status.detail.as_deref() == Some(PLUGIN_NOT_LOADED)
        && now - LAST_MEND.load(Ordering::Relaxed) >= 600
    {
        LAST_MEND.store(now, Ordering::Relaxed);
        reconcile_in_background(app);
    }
    Ok(status)
}

/// The canary's verdict: a normal request and an attack through Traefik
/// (at `base`, Traefik's own address when `None`). The sites' definitions
/// follow it every time (the file is the truth, not the stored status): the
/// plugin once it is loaded, no-ops when it is not.
pub async fn probe(app: &AppState, base: Option<String>) -> AppResult<Status> {
    let Some(status) = verdict(app, base).await? else {
        return Status::load(app.db()).await;
    };
    let before = Status::load(app.db()).await?;
    record(app, &before, &status).await?;
    Ok(status)
}

/// What the canary says now, nothing stored but the definitions it implies
/// (the plugin's once proved, the no-ops when Traefik could not load it):
/// `None` when the WAF is off or Docker unbound.
async fn verdict(app: &AppState, base: Option<String>) -> AppResult<Option<Status>> {
    let settings = Settings::load(app.db()).await?;
    if settings.mode == Mode::Off {
        return Ok(None);
    }
    let (Some(docker), Some(traefik)) = (app.service::<Docker>(), app.service::<Traefik>()) else {
        return Ok(None);
    };
    let base = base.unwrap_or_else(|| canary_url(&traefik));
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(AppError::internal)?;
    let get = |path: &str| {
        let sent = client
            .get(format!("{base}{path}"))
            .header(reqwest::header::HOST, CANARY_HOST)
            .send();
        async move {
            let answer = sent.await;
            count_canary(&answer);
            answer
        }
    };
    let before = Status::load(app.db()).await?;
    // The canary's router is the plugin whatever the sites have: a 404 there
    // only means the plugin is not loaded.
    // The status and, for a success, the body.
    let normal = match get("/").await {
        Err(err) => Err(err),
        Ok(r) if r.status().is_success() => {
            let code = r.status();
            Ok((code, r.text().await.unwrap_or_default()))
        }
        Ok(r) => Ok((r.status(), String::new())),
    };
    let status = match normal {
        Err(err) => Status::now(
            "unavailable",
            Some(format!("Traefik did not answer: {err}")),
            before.applied,
        ),
        Ok((code, _)) if code == reqwest::StatusCode::NOT_FOUND => {
            // Never a 404 on the sites: their WAF middlewares become no-ops.
            write_definitions(&traefik, &docker, &settings, false).await?;
            Status::now("unavailable", Some(PLUGIN_NOT_LOADED.to_owned()), false)
        }
        Ok((code, _)) if !code.is_success() => Status::now(
            "unavailable",
            Some(format!("The canary answered {code}")),
            before.applied,
        ),
        // Not the panel's answer: the canary's route is not the one Traefik
        // follows (the sites keep what they have).
        Ok((_, body)) if body != CANARY_ANSWER => Status::now(
            "unavailable",
            Some("The canary was answered by something else than the panel".to_owned()),
            before.applied,
        ),
        // The plugin is loaded: the sites may have it (it lets requests
        // through when CrowdSec does not answer).
        Ok(_) => {
            write_definitions(&traefik, &docker, &settings, true).await?;
            match get("/.env").await {
                Ok(attack) if attack.status() == reqwest::StatusCode::FORBIDDEN => {
                    match mode_and_level_answer(&docker, &settings, &client, &base).await {
                        None => Status::now("on", None, true),
                        Some(reason) => Status::now("unavailable", Some(reason), true),
                    }
                }
                Ok(_) => Status::now(
                    "unavailable",
                    Some("CrowdSec does not answer: requests go through uninspected".to_owned()),
                    true,
                ),
                Err(err) => Status::now("unavailable", Some(err.to_string()), true),
            }
        }
    };
    Ok(Some(status))
}

/// Stores the WAF's state; a change is logged, and its stopping checking
/// requests or checking them again is said to the notification channels.
async fn record(app: &AppState, before: &Status, status: &Status) -> AppResult<()> {
    status.store(app.db()).await?;
    if status.state != before.state {
        tracing::warn!(state = status.state, detail = ?status.detail, "the WAF's state changed");
        // It stopped checking requests, or checks them again (never while
        // starting: the page says so).
        let event = match (before.state.as_str(), status.state.as_str()) {
            ("on", "unavailable") => Some(true),
            ("unavailable", "on") => Some(false),
            _ => None,
        };
        if let Some(down) = event {
            crate::services::notify_service::notify(
                app,
                crate::services::notify_service::Event::Waf {
                    down,
                    detail: status.detail.clone(),
                },
            );
        }
    }
    Ok(())
}

/// Beyond Basic's Block (proved already): the Strict rules block an SQL
/// injection, and in Detect mode the Detect listener checks requests
/// (CrowdSec's counter of its rules goes up). Why not, else `None`.
async fn mode_and_level_answer(
    docker: &Docker,
    settings: &Settings,
    client: &reqwest::Client,
    base: &str,
) -> Option<String> {
    let send = |host: &'static str, path: &str| {
        let sent = client
            .get(format!("{base}{path}"))
            .header(reqwest::header::HOST, host)
            .send();
        async move {
            let answer = sent.await;
            count_canary(&answer);
            answer
        }
    };
    if settings.level == Level::Strict {
        match send(CANARY_HOST, STRICT_PROBE).await {
            Ok(r) if r.status() == reqwest::StatusCode::FORBIDDEN => {}
            _ => return Some(STRICT_NOT_ANSWERING.to_owned()),
        }
    }
    if settings.mode == Mode::Detect {
        let before = detect_matches(docker).await;
        let sent = send(CANARY_DETECT_HOST, "/.env").await;
        // Counted when CrowdSec has answered the plugin; a moment for its
        // metrics to show it.
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let after = detect_matches(docker).await;
        let checked = matches!(&sent, Ok(r) if r.status().is_success())
            && matches!((before, after), (Some(b), Some(a)) if a > b);
        if !checked {
            return Some(DETECT_NOT_ANSWERING.to_owned());
        }
    }
    None
}

pub const STRICT_NOT_ANSWERING: &str =
    "The Strict rules (OWASP Core Rule Set) do not answer: only the Basic ones check requests";
pub const DETECT_NOT_ANSWERING: &str =
    "Detect mode does not answer: requests go through unrecorded";

/// How many times the Detect listener's rules matched since CrowdSec started
/// (its metrics); `None` when they cannot be read.
async fn detect_matches(docker: &Docker) -> Option<u64> {
    let text = cscli(docker, &["metrics", "show", "appsec", "-o", "json"])
        .await
        .ok()??;
    let metrics: serde_json::Value = serde_json::from_str(&text).ok()?;
    let listener = format!(":{APPSEC_DETECT_PORT}/");
    let rules = metrics["appsec-rule"].as_object()?;
    Some(
        rules
            .iter()
            .filter(|(engine, _)| engine.ends_with(&listener))
            .flat_map(|(_, by_rule)| by_rule.as_object().into_iter().flatten())
            .filter_map(|(_, r)| r["triggered"].as_u64())
            .sum(),
    )
}

/// `cscli <args>` in CrowdSec's running container: its output, `None` when
/// none runs.
/// One of the canary's requests sent: counted, the statistics leave it out.
fn count_canary(answer: &Result<reqwest::Response, reqwest::Error>) {
    crate::services::waf_stats_service::canary_sent(
        matches!(answer, Ok(r) if r.status() == reqwest::StatusCode::FORBIDDEN),
    );
}

/// CrowdSec's running container (its id), if one runs.
pub(crate) async fn crowdsec_container(docker: &Docker) -> AppResult<Option<String>> {
    let tasks = docker
        .engine()
        .list_tasks(CROWDSEC)
        .await
        .map_err(docker_err)?;
    Ok(tasks.iter().find_map(|t| {
        (t.status.as_ref()?.state? == crate::services::docker_service::TaskState::RUNNING)
            .then(|| {
                t.status
                    .as_ref()?
                    .container_status
                    .as_ref()?
                    .container_id
                    .clone()
            })
            .flatten()
    }))
}

/// The minute's records: CrowdSec's counters sampled, its new alerts
/// copied, old rows pruned (each failure only logged).
async fn keep_records(app: &AppState, docker: &Docker) {
    use crate::services::waf_stats_service as stats;
    let Ok(Some(container)) = crowdsec_container(docker).await else {
        return;
    };
    match cscli(docker, &["metrics", "show", "appsec", "-o", "json"]).await {
        Ok(Some(text)) => match serde_json::from_str::<serde_json::Value>(text.trim()) {
            Ok(metrics) => {
                if let Err(err) = stats::sample(app.db(), &container, &metrics).await {
                    tracing::warn!(%err, "the WAF's statistics not sampled");
                }
            }
            Err(err) => tracing::warn!(%err, "CrowdSec's metrics could not be read"),
        },
        Ok(None) => {}
        Err(err) => tracing::warn!(%err, "CrowdSec's metrics not read"),
    }
    let since = match stats::since(app.db()).await {
        Ok(since) => since,
        Err(err) => {
            tracing::warn!(%err, "the WAF's alerts checkpoint not read");
            return;
        }
    };
    let limit = stats::mirror_limit().to_string();
    match cscli(
        docker,
        &[
            "alerts", "list", "-o", "json", "--since", &since, "-l", &limit,
        ],
    )
    .await
    {
        Ok(Some(text)) => {
            let text = text.trim();
            if !text.is_empty() && text != "null" {
                match serde_json::from_str::<Vec<serde_json::Value>>(text) {
                    Ok(alerts) => {
                        if let Err(err) = stats::mirror(app.db(), &alerts).await {
                            tracing::warn!(%err, "the WAF's alerts not copied");
                        }
                    }
                    Err(err) => tracing::warn!(%err, "CrowdSec's alerts could not be read"),
                }
            }
        }
        Ok(None) => {}
        Err(err) => tracing::warn!(%err, "CrowdSec's alerts not read"),
    }
    if let Err(err) = stats::prune(app.db()).await {
        tracing::warn!(%err, "the WAF's old records not pruned");
    }
    // CrowdSec's own scenarios never ban (profiles.yaml); a ban one made
    // before (an older panel) goes, the blocklist being the community's
    // and the administrators' only.
    if let Ok(settings) = Settings::load(app.db()).await
        && settings.blocklist
        && let Err(err) = cscli(docker, &["decisions", "delete", "--origin", "crowdsec"]).await
    {
        tracing::warn!(%err, "CrowdSec's own bans not removed");
    }
}

pub(crate) async fn cscli(docker: &Docker, args: &[&str]) -> AppResult<Option<String>> {
    use futures_util::StreamExt;
    let Some(container) = crowdsec_container(docker).await? else {
        return Ok(None);
    };
    let mark = format!(
        "rdp-waf-{}",
        crate::services::database_service::random_string(12)
    );
    let mut cmd = vec!["cscli".to_owned()];
    cmd.extend(args.iter().map(|a| (*a).to_owned()));
    let mut stream = docker
        .engine()
        .run_in_container(&container, cmd, &mark, None)
        .await
        .map_err(docker_err)?;
    use crate::services::docker_service::ExecChunk;
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let mut code = 0;
    while let Some(chunk) = stream.next().await {
        match chunk.map_err(docker_err)? {
            ExecChunk::Out(bytes) if out.len() < 4 * 1024 * 1024 => out.extend(bytes),
            ExecChunk::Err(bytes) if err.len() < 4096 => err.extend(bytes),
            ExecChunk::Exit(c) => code = c,
            _ => {}
        }
    }
    if code != 0 {
        return Err(AppError::internal(format!(
            "cscli {} failed ({code}): {}",
            args.first().copied().unwrap_or_default(),
            String::from_utf8_lossy(&err).trim()
        )));
    }
    Ok(Some(String::from_utf8_lossy(&out).into_owned()))
}

pub const PLUGIN_NOT_LOADED: &str = "Traefik could not load the WAF's plugin: the sites are served uninspected; Repair starts it again";

fn canary_url(traefik: &Traefik) -> String {
    std::env::var("RDP_WAF_CANARY_URL").unwrap_or_else(|_| format!("http://{}:80", traefik.service))
}

/// At boot, before the routes are written: the definitions as no-ops (the
/// sites never answer 404 while CrowdSec and the plugin are checked), then
/// [`reconcile`] in the background.
pub async fn boot(state: &AppState) {
    // A reconcile, restart, rules update or Cloudflare apply the panel's
    // stop cut short: failed, interrupted (the reconcile below starts the
    // WAF again with the stored settings; a Cloudflare list still pending
    // is spread again).
    if let Err(err) = crate::services::action_service::interrupted(
        state.db(),
        &[
            ACTION,
            "waf.restart",
            crate::services::waf_rules_service::UPDATE_ACTION,
            crate::services::cloudflare_ranges_service::ACTION,
        ],
    )
    .await
    {
        tracing::warn!(%err, "the WAF's interrupted actions not closed");
    }
    let (Some(docker), Some(traefik)) = (state.service::<Docker>(), state.service::<Traefik>())
    else {
        return;
    };
    let settings = match Settings::load(state.db()).await {
        Ok(s) => s,
        Err(err) => {
            tracing::warn!(%err, "the WAF settings cannot be read");
            Settings::default()
        }
    };
    if let Err(err) = write_definitions(&traefik, &docker, &settings, false).await {
        tracing::warn!(%err, "the WAF's definitions not written");
    }
    // Off but not stored as off (turned off just before the panel stopped):
    // CrowdSec stopped and the state stored now, under the reconcile's lock.
    let stored_off = Status::load(state.db())
        .await
        .map(|s| s.state == "off")
        .unwrap_or(false);
    if settings.mode != Mode::Off || !stored_off {
        reconcile_in_background(state);
    }
}

/// [`reconcile`] apart from the request (it may take a minute: a pull, a
/// Traefik restart); the Settings page follows the status.
pub fn reconcile_in_background(state: &AppState) {
    reconcile_in_background_with(state, false, None);
}

/// [`reconcile_with`] apart from the request, reporting to `action`.
pub fn reconcile_in_background_with(
    state: &AppState,
    rotate: bool,
    action: Option<crate::models::action::Model>,
) {
    let state = state.clone();
    tokio::spawn(async move {
        match reconcile_with(&state, rotate, action).await {
            Ok(status) => tracing::info!(state = status.state, "the WAF reconciled"),
            Err(err) => tracing::warn!(%err, "the WAF not reconciled"),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_rules_come_from_the_crowdsec_that_runs() {
        // The image's build stage fetches the hub with the image CrowdSec
        // runs as, and installs the collections it is started with.
        let dockerfile = include_str!("../../Dockerfile");
        assert!(
            dockerfile.contains(&format!("FROM {IMAGE} AS waf-hub")),
            "Dockerfile's waf-hub stage is not {IMAGE}"
        );
        assert!(dockerfile.contains(&format!("cscli collections install {COLLECTIONS}")));
        assert!(dockerfile.contains(&format!("/out/hub {HUB_SOURCE}")));
        assert!(dockerfile.contains(&format!("/out/data {HUB_DATA_SOURCE}")));
    }

    #[test]
    fn a_bouncer_is_named_by_its_keys_secret() {
        assert_eq!(bouncer_name("rdp_crowdsec_bouncer_ab12"), "rdpwafab12");
    }
}
