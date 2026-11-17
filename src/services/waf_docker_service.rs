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
/// CrowdSec's volumes (its database, its hub content).
const DATA_VOLUME: &str = "rdp_crowdsec_data";
const CONFIG_VOLUME: &str = "rdp_crowdsec_config";
/// The bouncer key's secrets: `rdp_crowdsec_bouncer_<hash>`.
const KEY_SECRET: &str = "rdp_crowdsec_bouncer";
/// The canary's host (never a real domain) and its backend path in the panel.
pub const CANARY_HOST: &str = "rdp-waf-canary.invalid";
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

    pub async fn store(&self, db: &sea_orm::DatabaseConnection) -> AppResult<()> {
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
fn crowdsec_files(level: Level) -> Vec<(&'static str, String)> {
    let (block, detect) = match level {
        Level::Basic => ("rdp/appsec-block", "rdp/appsec-detect"),
        Level::Strict => ("rdp/appsec-strict", "rdp/appsec-detect-strict"),
    };
    let listener = |config: &str, port: u16| {
        format!(
            "appsec_configs:\n  - {config}\nlabels:\n  type: appsec\nlisten_addr: 0.0.0.0:{port}\nsource: appsec\n"
        )
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
            include_str!("../../resources/waf/crowdsec/rdp-block.yaml").to_owned(),
        ),
        (
            "/etc/crowdsec/appsec-configs/rdp-strict.yaml",
            include_str!("../../resources/waf/crowdsec/rdp-strict.yaml").to_owned(),
        ),
        (
            "/etc/crowdsec/appsec-configs/rdp-detect.yaml",
            include_str!("../../resources/waf/crowdsec/rdp-detect.yaml").to_owned(),
        ),
        (
            "/etc/crowdsec/appsec-configs/rdp-detect-strict.yaml",
            include_str!("../../resources/waf/crowdsec/rdp-detect-strict.yaml").to_owned(),
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
    let mut list: Vec<String> = crate::services::rate_limit_service::CLOUDFLARE
        .iter()
        .map(|c| (*c).to_owned())
        .collect();
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
        http["routers"] = json!({"rdp-waf-canary": {
            "rule": format!("Host(`{CANARY_HOST}`)"),
            // Above every service's router (theirs are their rule's length
            // times 4, at most about 40 000).
            "priority": CANARY_PRIORITY,
            "entryPoints": [crate::services::deploy_service::HTTP_ENTRYPOINT],
            "middlewares": ["rdp-waf-buffer", "rdp-waf-canary", "rdp-waf-canary-path"],
            "service": "rdp-waf-canary",
        }});
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

/// One reconcile at a time (a save, Repair and the boot may overlap); the
/// schedule's check waits for none and skips while one runs.
static RECONCILING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Brings CrowdSec, Traefik and the definitions to the saved settings; then
/// every service's routes are written again (their WAF on or off).
pub async fn reconcile(app: &AppState) -> AppResult<Status> {
    let _one = RECONCILING.lock().await;
    let docker = app
        .service::<Docker>()
        .ok_or_else(|| AppError::internal("Docker is not bound"))?;
    let traefik = app
        .service::<Traefik>()
        .ok_or_else(|| AppError::internal("Traefik is not bound"))?;
    let settings = Settings::load(app.db()).await?;
    let before = Status::load(app.db()).await?;
    // The sites' middlewares are no-ops until the canary proves the plugin
    // (a site never answers 404 meanwhile; requests go through unchecked).
    write_definitions(&traefik, &docker, &settings, false).await?;
    let status = if settings.mode == Mode::Off {
        scale_crowdsec(&docker, 0).await?;
        Status::now("off", None, false)
    } else {
        // Traefik loads its plugin when it starts: after a failure to load
        // it, it is started again (the plugin was copied again).
        let restart = before.detail.as_deref() == Some(PLUGIN_NOT_LOADED);
        match start(app, &docker, &traefik, &settings, restart).await {
            Ok(()) => Status::now("starting", None, false),
            Err(err) => Status::now("unavailable", Some(err.to_string()), false),
        }
    };
    status.store(app.db()).await?;
    let options = crate::services::deployment_service::deploy_options(0);
    crate::services::traefik_service::sync_all(app.db(), &traefik, &options).await?;
    if status.state == "starting" {
        // The canary every 5 s, two minutes at most (CrowdSec and Traefik
        // starting); then the schedule's minute.
        let mut restarted = false;
        for _ in 0..24 {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            let now = probe(app, None).await?;
            if now.state == "on" {
                return Ok(now);
            }
            // Traefik loaded the plugin before it was copied again: started
            // again once, it loads this copy.
            if now.detail.as_deref() == Some(PLUGIN_NOT_LOADED) && !restarted {
                restarted = true;
                restart_traefik(&docker, &traefik).await?;
            }
        }
        return Status::load(app.db()).await;
    }
    Ok(status)
}

async fn start(
    app: &AppState,
    docker: &Docker,
    traefik: &Traefik,
    settings: &Settings,
    restart: bool,
) -> AppResult<()> {
    if traefik.service.is_empty() {
        return Err(AppError::BadRequest(
            "Traefik is not a Swarm service of the panel's here".into(),
        ));
    }
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
    seed_plugin(app, docker).await?;
    let key_secret = bouncer_secret(docker).await?;
    run_crowdsec(docker, settings, &network.id, &key_secret).await?;
    give_traefik_the_plugin(docker, traefik, &network.id, &key_secret, restart).await?;
    Ok(())
}

/// Copies the plugin from the panel's image into its volume (a helper
/// container of the panel's own image).
async fn seed_plugin(app: &AppState, docker: &Docker) -> AppResult<()> {
    let image = crate::services::import_service::helper_image(app, docker)
        .await
        .ok_or_else(|| {
            AppError::BadRequest(
                "the panel's own image is unknown (not the installer's service): the WAF's plugin cannot be copied".into(),
            )
        })?;
    let _ = docker
        .engine()
        .create_volume_labelled(PLUGIN_VOLUME, system_labels())
        .await;
    // Copied beside, checked, then swapped in: Traefik starting meanwhile
    // reads the old copy or the new one, never half of one.
    let script = format!(
        "set -e; test -f {PLUGIN_SOURCE}/VERSION; rm -rf /plugins/.new /plugins/.old; \
         cp -r {PLUGIN_SOURCE}/src /plugins/.new; test -f /plugins/.new/{module}/go.mod; \
         if [ -d /plugins/src ]; then mv /plugins/src /plugins/.old; fi; \
         mv /plugins/.new /plugins/src; rm -rf /plugins/.old; \
         cp {PLUGIN_SOURCE}/VERSION /plugins/VERSION",
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
            mounts: vec![HelperMount {
                source: PLUGIN_VOLUME.to_owned(),
                target: "/plugins".to_owned(),
                read_only: false,
                kind: "volume".to_owned(),
            }],
            import_id: 0,
            cap_add: vec![],
            timeout_s: 60,
        })
        .await
        .map_err(docker_err)?;
    if code != 0 {
        return Err(AppError::internal(format!(
            "the WAF's plugin could not be copied (exit {code})"
        )));
    }
    Ok(())
}

/// The bouncer key's secret in use, else a new one (a random key, never kept
/// elsewhere): (id, name).
async fn bouncer_secret(docker: &Docker) -> AppResult<(String, String)> {
    let secrets = docker
        .engine()
        .list_secrets(SYSTEM_LABEL, "true")
        .await
        .map_err(docker_err)?;
    if let Some(found) = secrets.iter().find(|(_, n)| n.starts_with(KEY_SECRET)) {
        return Ok(found.clone());
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

/// CrowdSec's Swarm configs (named by their content: a change is a new one).
async fn configs(docker: &Docker, level: Level) -> AppResult<Vec<serde_json::Value>> {
    let existing = docker
        .engine()
        .list_configs(SYSTEM_LABEL, "true")
        .await
        .map_err(docker_err)?;
    let mut out = vec![];
    for (target, content) in crowdsec_files(level) {
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
    let configs = configs(docker, settings.level).await?;
    let bouncer = format!(
        "rdpwaf{}",
        key.1.trim_start_matches(KEY_SECRET).trim_matches('_')
    );
    let mut env = vec![format!("COLLECTIONS={COLLECTIONS}")];
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
                    format!("export BOUNCER_KEY_{bouncer}=\"$(cat /run/secrets/{KEY_SECRET})\"; exec /bin/bash /docker_start.sh")],
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
        probe(app, None).await?
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
    let settings = Settings::load(app.db()).await?;
    if settings.mode == Mode::Off {
        return Status::load(app.db()).await;
    }
    let (Some(docker), Some(traefik)) = (app.service::<Docker>(), app.service::<Traefik>()) else {
        return Status::load(app.db()).await;
    };
    let base = base.unwrap_or_else(|| canary_url(&traefik));
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(AppError::internal)?;
    let get = |path: &str| {
        client
            .get(format!("{base}{path}"))
            .header(reqwest::header::HOST, CANARY_HOST)
            .send()
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
                    Status::now("on", None, true)
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
    status.store(app.db()).await?;
    if status.state != before.state {
        tracing::warn!(state = status.state, detail = ?status.detail, "the WAF's state changed");
    }
    Ok(status)
}

pub const PLUGIN_NOT_LOADED: &str = "Traefik could not load the WAF's plugin: the sites are served uninspected; Repair starts it again";

fn canary_url(traefik: &Traefik) -> String {
    std::env::var("RDP_WAF_CANARY_URL").unwrap_or_else(|_| format!("http://{}:80", traefik.service))
}

/// At boot, before the routes are written: the definitions as no-ops (the
/// sites never answer 404 while CrowdSec and the plugin are checked), then
/// [`reconcile`] in the background.
pub async fn boot(state: &AppState) {
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
    let state = state.clone();
    tokio::spawn(async move {
        match reconcile(&state).await {
            Ok(status) => tracing::info!(state = status.state, "the WAF reconciled"),
            Err(err) => tracing::warn!(%err, "the WAF not reconciled"),
        }
    });
}

/// CrowdSec's latest alerts (newest first, at most `limit`): when, the
/// address, what matched, the request's host and path when known. Never
/// request bodies.
pub async fn alerts(docker: &Docker, limit: usize) -> AppResult<Vec<serde_json::Value>> {
    use futures_util::StreamExt;
    let tasks = docker
        .engine()
        .list_tasks(CROWDSEC)
        .await
        .map_err(docker_err)?;
    let Some(container) = tasks.iter().find_map(|t| {
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
    }) else {
        return Ok(vec![]);
    };
    let mark = format!(
        "rdp-waf-{}",
        crate::services::database_service::random_string(12)
    );
    let cmd = vec![
        "cscli".to_owned(),
        "alerts".to_owned(),
        "list".to_owned(),
        "-o".to_owned(),
        "json".to_owned(),
        "-l".to_owned(),
        limit.to_string(),
    ];
    let mut stream = docker
        .engine()
        .run_in_container(&container, cmd, &mark, None)
        .await
        .map_err(docker_err)?;
    let mut out = Vec::new();
    while let Some(chunk) = stream.next().await {
        if let crate::services::docker_service::ExecChunk::Out(bytes) = chunk.map_err(docker_err)? {
            out.extend(bytes);
            if out.len() > 4 * 1024 * 1024 {
                break;
            }
        }
    }
    let text = String::from_utf8_lossy(&out);
    let text = text.trim();
    if text.is_empty() || text == "null" {
        return Ok(vec![]);
    }
    let raw: Vec<serde_json::Value> = serde_json::from_str(text)
        .map_err(|_| AppError::internal("CrowdSec's alerts could not be read"))?;
    // The canary's (it blocks a test attack every minute) are not alerts;
    // CrowdSec drops them (`on_match`), older ones are left out here.
    Ok(raw
        .iter()
        .map(summary)
        .filter(|a| a["host"].as_str() != Some(CANARY_HOST))
        .collect())
}

/// The fields the page shows of an alert.
fn summary(alert: &serde_json::Value) -> serde_json::Value {
    // AppSec's events carry the request's host and path in their meta.
    let meta = |key: &str| {
        alert["events"]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|e| e["meta"].as_array().cloned().unwrap_or_default())
            .find(|m| m["key"] == key)
            .and_then(|m| m["value"].as_str().map(str::to_owned))
    };
    json!({
        "id": alert["id"],
        "at": alert["created_at"],
        "ip": alert["source"]["ip"].as_str().or(alert["source"]["value"].as_str()),
        "rule": alert["scenario"],
        "host": meta("target_fqdn"),
        "path": meta("uri"),
        "message": meta("message"),
        "blocked": alert["scenario"].as_str() != Some("rdp/appsec-detect"),
    })
}
