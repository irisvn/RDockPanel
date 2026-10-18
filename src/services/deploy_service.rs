//! What a service becomes in Docker Swarm and Traefik.
//!
//! [`Snapshot`] is a service's configuration as one deployment applies it
//! (stored with the deployment: history, rollback; never a secret value).
//! [`desired`] turns it into the Swarm service spec and the Traefik routes,
//! and [`Snapshot::hash`] tells whether the configuration changed since the
//! last deployment. All pure: no database, no Docker.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::models::{project, service, service_domain, service_mount, service_port};
use crate::services::docker_service::{ImageDetails, MANAGED_LABEL, ServiceSpec};

/// Traefik's entry points (the installer's and `scripts/dev-swarm.sh`'s).
pub const HTTP_ENTRYPOINT: &str = "web";
pub const HTTPS_ENTRYPOINT: &str = "websecure";

/// Seconds Swarm watches a new task before it counts as updated (and rolls
/// back on failure).
const MONITOR_SECS: u64 = 30;
const NANOS: u64 = 1_000_000_000;

/// The Swarm service of `service` in `project`.
pub fn service_name(project: &str, service: &str) -> String {
    format!("{project}_{service}")
}

/// The overlay network of a project (its services, and Traefik).
pub fn network_name(project: &str) -> String {
    format!("rdp_{project}")
}

/// A service's named volume.
pub fn volume_name(project: &str, service: &str, volume: &str) -> String {
    format!("rdp_{project}_{service}_{volume}")
}

/// A service's configuration as a deployment applies it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub project: String,
    pub service: String,
    /// As configured (`nginx:1.27`): the deployment pins it to a digest.
    pub image: String,
    pub command: Option<String>,
    pub replicas: u32,
    pub running: bool,
    pub cpu_limit_millis: Option<u32>,
    pub mem_limit_mb: Option<u32>,
    pub cpu_reservation_millis: Option<u32>,
    pub mem_reservation_mb: Option<u32>,
    pub zero_downtime: bool,
    pub healthcheck: Option<Healthcheck>,
    pub domains: Vec<Domain>,
    pub ports: Vec<Port>,
    pub mounts: Vec<Mount>,
    /// The environment's variable names; their values are sealed apart
    /// (`deployments.env`), never in a snapshot.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env_names: Vec<String>,
    /// A digest of the environment keyed with `APP_KEY` ([`env_digest`]):
    /// a change of a value changes the snapshot's hash, and reveals nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env_digest: Option<String>,
    /// The private registry's user (its password is sealed apart).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry_username: Option<String>,
    /// The host of its primary domain (the first added): `$(PRIMARY_DOMAIN)`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_domain: Option<String>,
    /// How its image is built (`None`: pulled, `image`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build: Option<Build>,
    /// A Compose service: run by `docker compose`, not as a Swarm service.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compose: Option<Compose>,
    /// Redirects on its domains (those enabled, in order).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub redirects: Vec<Redirect>,
    /// HTTP basic authentication on its domains: `user:<digest of its
    /// hash>`, by user (a password change is a change; the hashes
    /// themselves stay in their table, read when the routes are written).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub basic_auth: Vec<String>,
}

/// A redirect: Traefik's `redirectRegex`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Redirect {
    pub regex: String,
    pub replacement: String,
    pub permanent: bool,
}

/// A Compose service's file: written in the panel, or of its repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Compose {
    /// The file written in the panel (`None`: the repository's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inline: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_branch: Option<String>,
    /// The Compose project's directory in the repository (`/`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_path: Option<String>,
    /// The file in that directory (`docker-compose.yml`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
}

/// A service's image built by the panel: from a Git repository, or from a
/// Dockerfile written in the panel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Build {
    /// `git` or `dockerfile`.
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_branch: Option<String>,
    /// The context in the repository (`/`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_path: Option<String>,
    /// `dockerfile` or `nixpacks`.
    pub builder: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dockerfile_path: Option<String>,
    /// The Dockerfile written in the panel (`source = dockerfile`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dockerfile: Option<String>,
}

/// The environment's digest, keyed (`key`: derived from `APP_KEY`): HMAC-SHA256
/// of its JSON. `None` for no variable.
pub fn env_digest(env: &BTreeMap<String, String>, key: &[u8]) -> Option<String> {
    use hmac::{KeyInit, Mac};
    if env.is_empty() {
        return None;
    }
    let mut mac = hmac::Hmac::<Sha256>::new_from_slice(key).expect("HMAC takes keys of any length");
    mac.update(&serde_json::to_vec(env).expect("an environment serializes"));
    Some(
        mac.finalize()
            .into_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    )
}

/// An environment variable's name: a letter or `_`, then letters, digits
/// and `_` (what every shell takes), at most 128.
pub fn valid_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    name.len() <= 128
        && chars
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Healthcheck {
    pub command: String,
    pub interval_s: u32,
    pub retries: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Domain {
    pub host: String,
    pub path: String,
    pub port: u16,
    pub https: bool,
    /// A Compose service's container the domain leads to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compose_service: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Port {
    pub published: u16,
    pub target: u16,
    pub protocol: String,
    pub mode: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Mount {
    pub target: String,
    /// `volume`, `bind` or `file`.
    pub kind: String,
    /// The volume's name, or the host directory (empty for a file).
    pub source: String,
    pub read_only: bool,
    /// A file's content, as a digest keyed with `APP_KEY` (the content is
    /// sealed apart: `deployments.files`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_digest: Option<String>,
}

impl Mount {
    /// A file's Swarm config: `rdp_<hash>` of what makes it (project,
    /// service, path, content's digest), so a new content is a new config.
    pub fn config_name(&self, snapshot: &Snapshot) -> Option<String> {
        let content = self.content_digest.as_deref()?;
        let id = sha256_hex(
            format!(
                "{}\0{}\0{}\0{content}",
                snapshot.project, snapshot.service, self.target
            )
            .as_bytes(),
        );
        Some(format!("rdp_{}", &id[..32]))
    }
}

/// What a deployment resolved for its run: the environment (opened,
/// placeholders expanded) and the Swarm configs of its file mounts (by path).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Runtime {
    pub env: BTreeMap<String, String>,
    /// The keyed digest of `env` as resolved: what the task template's hash
    /// counts (a placeholder resolving differently restarts the tasks).
    pub env_digest: Option<String>,
    pub configs: BTreeMap<String, ConfigRef>,
}

/// A Swarm config a file mount uses: its id and name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigRef {
    pub id: String,
    pub name: String,
}

fn positive(value: Option<i32>) -> Option<u32> {
    value.and_then(|v| u32::try_from(v).ok()).filter(|v| *v > 0)
}

impl Snapshot {
    /// From the rows (lists sorted: the same configuration, the same snapshot).
    pub fn of(
        project: &project::Model,
        service: &service::Model,
        domains: &[service_domain::Model],
        ports: &[service_port::Model],
        mounts: &[service_mount::Model],
    ) -> Self {
        let healthcheck = service
            .healthcheck_cmd
            .as_deref()
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .map(|command| Healthcheck {
                command: command.to_owned(),
                interval_s: positive(service.healthcheck_interval_s).unwrap_or(10),
                retries: positive(service.healthcheck_retries).unwrap_or(3),
            });
        // The first added (lowest id) is the primary one.
        let primary_domain = domains.iter().min_by_key(|d| d.id).map(|d| d.host.clone());
        let mut domains: Vec<Domain> = domains
            .iter()
            .filter_map(|d| {
                Some(Domain {
                    host: d.host.clone(),
                    path: d.path.clone(),
                    port: u16::try_from(d.port).ok()?,
                    https: d.https,
                    compose_service: d.compose_service.clone(),
                })
            })
            .collect();
        domains.sort();
        let mut ports: Vec<Port> = ports
            .iter()
            .filter_map(|p| {
                Some(Port {
                    published: u16::try_from(p.published).ok()?,
                    target: u16::try_from(p.target).ok()?,
                    protocol: p.protocol.clone(),
                    mode: p.mode.clone(),
                })
            })
            .collect();
        ports.sort();
        let mut mounts: Vec<Mount> = mounts
            .iter()
            .filter_map(|m| {
                let source = match m.kind.as_str() {
                    "bind" => m.source.clone()?,
                    "file" => String::new(),
                    _ => m.name.clone()?,
                };
                Some(Mount {
                    target: m.target.clone(),
                    kind: m.kind.clone(),
                    source,
                    read_only: m.read_only,
                    // Set from the sealed content ([`Snapshot::with_files`]).
                    content_digest: None,
                })
            })
            .collect();
        mounts.sort();
        Self {
            project: project.name.clone(),
            service: service.name.clone(),
            image: service.image.trim().to_owned(),
            command: service
                .command
                .as_deref()
                .map(str::trim)
                .filter(|c| !c.is_empty())
                .map(str::to_owned),
            replicas: positive(Some(service.replicas)).unwrap_or(1),
            running: service.desired_state != "stopped",
            cpu_limit_millis: positive(service.cpu_limit_millis),
            mem_limit_mb: positive(service.mem_limit_mb),
            cpu_reservation_millis: positive(service.cpu_reservation_millis),
            mem_reservation_mb: positive(service.mem_reservation_mb),
            zero_downtime: service.zero_downtime,
            healthcheck,
            domains,
            ports,
            mounts,
            // Set from the sealed environment ([`Snapshot::with_env`]).
            env_names: vec![],
            env_digest: None,
            primary_domain,
            // Set from their tables ([`Snapshot::with_access`]).
            redirects: vec![],
            basic_auth: vec![],
            compose: (service.kind == "compose").then(|| Compose {
                inline: (service.source != "git")
                    .then(|| service.compose.clone())
                    .flatten(),
                git_url: (service.source == "git")
                    .then(|| service.git_url.clone())
                    .flatten(),
                git_branch: (service.source == "git")
                    .then(|| service.git_branch.clone())
                    .flatten(),
                git_path: (service.source == "git")
                    .then(|| service.git_path.clone())
                    .flatten(),
                file: (service.source == "git")
                    .then(|| service.compose_file.clone())
                    .flatten(),
            }),
            build: match service.source.as_str() {
                "git" | "dockerfile" if service.kind != "compose" => Some(Build {
                    source: service.source.clone(),
                    git_url: (service.source == "git")
                        .then(|| service.git_url.clone())
                        .flatten(),
                    git_branch: (service.source == "git")
                        .then(|| service.git_branch.clone())
                        .flatten(),
                    git_path: (service.source == "git")
                        .then(|| service.git_path.clone())
                        .flatten(),
                    builder: if service.source == "git" {
                        service.builder.clone()
                    } else {
                        "dockerfile".to_owned()
                    },
                    dockerfile_path: (service.source == "git")
                        .then(|| service.dockerfile_path.clone())
                        .flatten(),
                    dockerfile: (service.source == "dockerfile")
                        .then(|| service.dockerfile.clone())
                        .flatten(),
                }),
                _ => None,
            },
            registry_username: service
                .registry_username
                .as_deref()
                .map(str::trim)
                .filter(|u| !u.is_empty())
                .map(str::to_owned),
        }
    }

    /// With its files' contents (by path): their digests keyed with `key`.
    pub fn with_files(mut self, files: &BTreeMap<String, String>, key: &[u8]) -> Self {
        for mount in self.mounts.iter_mut().filter(|m| m.kind == "file") {
            mount.content_digest = files.get(&mount.target).and_then(|content| {
                env_digest(&BTreeMap::from([(String::new(), content.clone())]), key)
            });
        }
        self
    }

    /// With its redirects (enabled ones, in order) and basic authentication.
    pub fn with_access(
        mut self,
        redirects: &[crate::models::service_redirect::Model],
        users: &[crate::models::service_basic_auth::Model],
    ) -> Self {
        self.redirects = redirects
            .iter()
            .filter(|r| r.enabled)
            .map(|r| Redirect {
                regex: r.regex.clone(),
                replacement: r.replacement.clone(),
                permanent: r.permanent,
            })
            .collect();
        let mut users: Vec<String> = users
            .iter()
            .map(|u| auth_digest(&u.username, &u.password_hash))
            .collect();
        users.sort();
        self.basic_auth = users;
        self
    }

    /// With the environment `env` (its names, and its digest keyed with `key`).
    pub fn with_env(mut self, env: &BTreeMap<String, String>, key: &[u8]) -> Self {
        self.env_names = env.keys().cloned().collect();
        self.env_digest = env_digest(env, key);
        self
    }

    /// Tells configurations apart (`services.deployed_spec_hash`): SHA-256 of
    /// the snapshot's JSON (fields in declaration order, lists sorted).
    pub fn hash(&self) -> String {
        sha256_hex(&serde_json::to_vec(self).expect("a snapshot serializes"))
    }

    pub fn service_name(&self) -> String {
        service_name(&self.project, &self.service)
    }

    /// Whether a deployment is rolled without downtime: start-first needs
    /// a health check to know the new task is ready, and a port published
    /// on the host cannot be held by two tasks at once.
    pub fn zero_downtime_possible(&self) -> bool {
        self.healthcheck.is_some() && !self.ports.iter().any(|p| p.mode == "host")
    }
}

/// The command a service runs instead of the image's (`CMD`), split like a
/// shell would (quotes, no expansion); `None` when it cannot be split.
pub fn command_args(command: &str) -> Option<Vec<String>> {
    shell_words::split(command)
        .ok()
        .filter(|args| !args.is_empty())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PinnedImage {
    /// `name[:tag]@sha256:…`: every node runs the same image.
    Digest(String),
    /// No registry digest (built or loaded on this node): its image id
    /// (`sha256:…`), the same content again, but only this node has it.
    Local(String),
}

impl PinnedImage {
    pub fn reference(&self) -> &str {
        match self {
            Self::Digest(r) | Self::Local(r) => r,
        }
    }

    pub fn digest(&self) -> Option<&str> {
        match self {
            Self::Digest(r) => r.split_once('@').map(|(_, d)| d),
            Self::Local(_) => None,
        }
    }

    /// What to tell the user about it, if anything.
    pub fn warning(&self) -> Option<&'static str> {
        match self {
            Self::Digest(_) => None,
            Self::Local(_) => {
                Some("the image has no registry digest: it runs only on the node that has it")
            }
        }
    }
}

/// The repository of a reference, as Docker Hub and registries name it the
/// same way (`nginx`, `library/nginx`, `docker.io/library/nginx` are one).
fn repository(reference: &str) -> String {
    let without_digest = reference.split('@').next().unwrap_or(reference);
    // A tag is after the last `:` that follows the last `/` (a registry may have a port).
    let name = match without_digest.rsplit_once(':') {
        Some((name, tag)) if !tag.contains('/') => name,
        _ => without_digest,
    };
    let name = name
        .strip_prefix("docker.io/")
        .or_else(|| name.strip_prefix("index.docker.io/"))
        .unwrap_or(name);
    name.strip_prefix("library/").unwrap_or(name).to_owned()
}

/// The digest a pull reported (`Digest: sha256:…` in Docker's progress).
pub fn pulled_digest<'a>(progress: impl IntoIterator<Item = &'a str>) -> Option<String> {
    progress
        .into_iter()
        .filter_map(|line| line.trim().strip_prefix("Digest: "))
        .map(str::trim)
        .find(|digest| valid_digest(digest))
        .map(str::to_owned)
}

fn valid_digest(digest: &str) -> bool {
    digest
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// `image` (as configured) pinned to what was just pulled: the digest the
/// pull reported, else the image's only registry digest for that repository
/// (several are ambiguous: another multi-arch index may share this node's
/// image), else its local image id.
pub fn pin_image(image: &str, pulled: Option<&str>, details: &ImageDetails) -> PinnedImage {
    if image.contains("@sha256:") {
        return PinnedImage::Digest(image.to_owned());
    }
    if let Some(digest) = pulled.filter(|d| valid_digest(d)) {
        return PinnedImage::Digest(format!("{image}@{digest}"));
    }
    let wanted = repository(image);
    let mut digests: Vec<&str> = details
        .repo_digests
        .iter()
        .filter_map(|entry| entry.split_once('@'))
        .filter(|(repo, _)| repository(repo) == wanted)
        .map(|(_, digest)| digest)
        .collect();
    digests.sort_unstable();
    digests.dedup();
    match digests.as_slice() {
        [digest] => PinnedImage::Digest(format!("{image}@{digest}")),
        _ => PinnedImage::Local(details.id.clone()),
    }
}

/// What a deployment needs besides the snapshot.
#[derive(Debug, Clone)]
pub struct DeployOptions {
    /// Bumped to restart tasks without another change (`force_update`).
    pub force_update: u64,
    /// Traefik's certificate resolver for HTTPS routes (`""`: its default
    /// certificate, as in development).
    pub cert_resolver: String,
    /// The public HTTPS port redirects go to (443; 8443 in development).
    pub https_port: u16,
}

impl Default for DeployOptions {
    fn default() -> Self {
        Self {
            force_update: 0,
            cert_resolver: String::new(),
            https_port: 443,
        }
    }
}

/// What a snapshot becomes: the Swarm service and its routes. Its `Debug`
/// leaves the spec out (it holds the environment's values).
#[derive(Clone, PartialEq)]
pub struct Desired {
    pub spec: ServiceSpec,
    /// `None` when the service has no domain or is stopped (no file).
    pub routes: Option<TraefikFile>,
    pub hash: String,
    /// The task template's own hash, also a container label
    /// ([`TEMPLATE_LABEL`]): which tasks run this spec.
    pub template: String,
}

impl std::fmt::Debug for Desired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Desired")
            .field("name", &self.spec.name)
            .field("hash", &self.hash)
            .field("template", &self.template)
            .finish_non_exhaustive()
    }
}

/// The container label naming the task template a task runs.
pub const TEMPLATE_LABEL: &str = "rdockpanel.template";

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The Swarm service and Traefik routes of `snapshot`, running `image` with
/// the environment `env` (the snapshot's, opened).
pub fn desired(
    snapshot: &Snapshot,
    image: &PinnedImage,
    runtime: &Runtime,
    options: &DeployOptions,
) -> Desired {
    let env = &runtime.env;
    let hash = snapshot.hash();
    let name = snapshot.service_name();
    let labels: BTreeMap<&str, &str> = BTreeMap::from([
        (MANAGED_LABEL, "true"),
        ("rdockpanel.project", snapshot.project.as_str()),
        ("rdockpanel.service", snapshot.service.as_str()),
        ("rdockpanel.spec-hash", hash.as_str()),
    ]);
    // Containers carry who they belong to, not the configuration's hash:
    // the task template changes only with what tasks run (a new domain or
    // more replicas restart nothing).
    let owner: BTreeMap<&str, &str> = labels
        .iter()
        .filter(|(k, _)| **k != "rdockpanel.spec-hash")
        .map(|(k, v)| (*k, *v))
        .collect();
    let mut container = json!({
        "Image": image.reference(),
        "Labels": owner,
        "Mounts": snapshot
            .mounts
            .iter()
            .filter(|m| m.kind != "file")
            .map(|m| mount(snapshot, m))
            .collect::<Vec<_>>(),
        "Init": true,
    });
    // Files: their configs, read-only for everyone in the container.
    let configs: Vec<Value> = snapshot
        .mounts
        .iter()
        .filter(|m| m.kind == "file")
        .filter_map(|m| {
            let config = runtime.configs.get(&m.target)?;
            Some(json!({
                "File": {"Name": m.target, "UID": "0", "GID": "0", "Mode": 0o444},
                "ConfigID": config.id,
                "ConfigName": config.name,
            }))
        })
        .collect();
    if !configs.is_empty() {
        container["Configs"] = json!(configs);
    }
    if let Some(args) = snapshot.command.as_deref().and_then(command_args) {
        container["Args"] = json!(args);
    }
    if let Some(check) = &snapshot.healthcheck {
        container["HealthCheck"] = json!({
            "Test": ["CMD-SHELL", check.command],
            "Interval": u64::from(check.interval_s) * NANOS,
            "Timeout": 5 * NANOS,
            "Retries": check.retries,
            "StartPeriod": 10 * NANOS,
        });
    }
    let mut resources = json!({});
    if snapshot.cpu_limit_millis.is_some() || snapshot.mem_limit_mb.is_some() {
        resources["Limits"] = limits(snapshot.cpu_limit_millis, snapshot.mem_limit_mb);
    }
    if snapshot.cpu_reservation_millis.is_some() || snapshot.mem_reservation_mb.is_some() {
        resources["Reservations"] =
            limits(snapshot.cpu_reservation_millis, snapshot.mem_reservation_mb);
    }
    let order = if snapshot.zero_downtime && snapshot.zero_downtime_possible() {
        "start-first"
    } else {
        "stop-first"
    };
    let mut template = json!({
        "ContainerSpec": container,
        "Resources": resources,
        "RestartPolicy": {"Condition": "any", "Delay": 5 * NANOS, "MaxAttempts": 0},
        "Networks": [{"Target": network_name(&snapshot.project), "Aliases": [snapshot.service]}],
        "ForceUpdate": options.force_update,
    });
    // Tasks tell which template they run (a restart, a new command or mount
    // with the same image are told apart from the previous tasks).
    // The environment counts by its keyed digest: the label never tells
    // anything of its values.
    let mut hashed = template.clone();
    if let Some(digest) = runtime.env_digest.as_ref().or(snapshot.env_digest.as_ref()) {
        hashed["ContainerSpec"]["Env"] = json!(digest);
    }
    let template_hash = sha256_hex(&serde_json::to_vec(&hashed).expect("a template serializes"));
    template["ContainerSpec"]["Labels"][TEMPLATE_LABEL] = json!(template_hash);
    if !env.is_empty() {
        template["ContainerSpec"]["Env"] = json!(
            env.iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
        );
    }
    let spec = json!({
        "Name": name,
        "Labels": labels,
        "TaskTemplate": template,
        "Mode": {"Replicated": {"Replicas": if snapshot.running { snapshot.replicas } else { 0 }}},
        "UpdateConfig": {
            "Parallelism": 1,
            "FailureAction": "rollback",
            "Monitor": MONITOR_SECS * NANOS,
            "Order": order,
        },
        "RollbackConfig": {"Parallelism": 1, "FailureAction": "pause", "Monitor": MONITOR_SECS * NANOS, "Order": order},
        "EndpointSpec": {
            "Mode": "vip",
            "Ports": snapshot.ports.iter().map(|p| json!({
                "Protocol": p.protocol,
                "TargetPort": p.target,
                "PublishedPort": p.published,
                "PublishMode": p.mode,
            })).collect::<Vec<_>>(),
        },
    });
    let spec: ServiceSpec = serde_json::from_value(spec).expect("a valid service spec");
    let routes = if snapshot.running {
        routes(snapshot, options)
    } else {
        None
    };
    Desired {
        spec,
        routes,
        hash,
        template: template_hash,
    }
}

fn limits(cpu_millis: Option<u32>, mem_mb: Option<u32>) -> Value {
    let mut out = json!({});
    if let Some(cpu) = cpu_millis {
        out["NanoCPUs"] = json!(u64::from(cpu) * 1_000_000);
    }
    if let Some(mem) = mem_mb {
        out["MemoryBytes"] = json!(u64::from(mem) * 1024 * 1024);
    }
    out
}

fn mount(snapshot: &Snapshot, mount: &Mount) -> Value {
    match mount.kind.as_str() {
        "bind" => {
            json!({"Type": "bind", "Source": mount.source, "Target": mount.target, "ReadOnly": mount.read_only})
        }
        _ => json!({
            "Type": "volume",
            "Source": volume_name(&snapshot.project, &snapshot.service, &mount.source),
            "Target": mount.target,
            "ReadOnly": mount.read_only,
            "VolumeOptions": {"Labels": {
                MANAGED_LABEL: "true",
                "rdockpanel.project": snapshot.project,
                "rdockpanel.service": snapshot.service,
            }},
        }),
    }
}

fn redirect_to_https(port: u16) -> Value {
    if port == 443 {
        json!({"redirectScheme": {"scheme": "https", "permanent": true}})
    } else {
        json!({"redirectScheme": {"scheme": "https", "port": port.to_string(), "permanent": true}})
    }
}

/// A Traefik dynamic configuration file (file provider).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TraefikFile {
    pub http: TraefikHttp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TraefikHttp {
    pub routers: BTreeMap<String, Router>,
    pub services: BTreeMap<String, LoadBalanced>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub middlewares: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Router {
    pub rule: String,
    #[serde(rename = "entryPoints")]
    pub entry_points: Vec<String>,
    pub service: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub middlewares: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tls: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LoadBalanced {
    #[serde(rename = "loadBalancer")]
    pub load_balancer: Value,
}

impl TraefikFile {
    pub fn to_yaml(&self) -> String {
        serde_yaml_ng::to_string(self).expect("routes serialize")
    }
}

/// A basic authentication user as a snapshot keeps it: `user:<digest of
/// its hash>`.
pub fn auth_digest(user: &str, hash: &str) -> String {
    format!("{user}:{}", &sha256_hex(hash.as_bytes())[..16])
}

/// A basic authentication user name: letters, digits, `_ - . @`.
pub fn valid_auth_user(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '@'))
}

/// A redirect Traefik takes: a regular expression of Go's syntax (RE2:
/// inline flags `i`, `m`, `s`, `U` only) and a replacement; neither with
/// `{{` or `}}` (Traefik reads its files as Go templates first).
pub fn check_redirect(regex: &str, replacement: &str) -> Result<(), &'static str> {
    for text in [regex, replacement] {
        if text.contains("{{") || text.contains("}}") {
            return Err("may not contain {{ or }}");
        }
    }
    if regex.trim().is_empty() || regex.len() > 500 || regex::Regex::new(regex).is_err() {
        return Err("is not a regular expression");
    }
    // Inline flags Go knows: `(?i)`, `(?-s:…)`, … (not Rust's `x`, `u`, `R`).
    let bytes = regex.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'\\' {
            i += 2;
            continue;
        }
        if bytes[i] == b'(' && bytes[i + 1] == b'?' {
            let flags: Vec<u8> = bytes[i + 2..]
                .iter()
                .copied()
                .take_while(|b| b.is_ascii_alphabetic() || *b == b'-')
                .collect();
            let next = bytes.get(i + 2 + flags.len()).copied();
            if !flags.is_empty()
                && matches!(next, Some(b')') | Some(b':'))
                && !flags
                    .iter()
                    .all(|f| matches!(f, b'i' | b'm' | b's' | b'U' | b'-'))
            {
                return Err("uses a flag Traefik does not know (only i, m, s, U)");
            }
        }
        i += 1;
    }
    if replacement.trim().is_empty() || replacement.len() > 500 {
        return Err("needs a replacement");
    }
    Ok(())
}

/// A Compose service's container on its project's network:
/// `<project>_<service>_<container>`.
pub fn compose_alias(base: &str, container: &str) -> String {
    format!("{base}_{container}")
}

/// A container of a Compose file as the panel names it (letters, digits,
/// `_`, `-`, `.`).
pub fn valid_compose_service(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

/// One router per domain (HTTPS ones with an HTTP router redirecting to
/// them), one Traefik service per container port, reached through the
/// service's name (a Compose container's alias) on the project network. Names start with the Swarm
/// service's, so files never collide.
pub fn routes(snapshot: &Snapshot, options: &DeployOptions) -> Option<TraefikFile> {
    if snapshot.domains.is_empty() {
        return None;
    }
    let base = snapshot.service_name();
    let mut http = TraefikHttp {
        routers: BTreeMap::new(),
        services: BTreeMap::new(),
        middlewares: BTreeMap::new(),
    };
    let redirect = format!("{base}-https");
    // Its redirects, then its authentication: on every router serving it.
    let mut access = vec![];
    for (index, r) in snapshot.redirects.iter().enumerate() {
        let name = format!("{base}-redirect-{index}");
        http.middlewares.insert(
            name.clone(),
            json!({"redirectRegex": {"regex": r.regex, "replacement": r.replacement, "permanent": r.permanent}}),
        );
        access.push(name);
    }
    if !snapshot.basic_auth.is_empty() {
        let name = format!("{base}-auth");
        http.middlewares.insert(
            name.clone(),
            json!({"basicAuth": {"users": snapshot.basic_auth}}),
        );
        access.push(name);
    }
    for (index, domain) in snapshot.domains.iter().enumerate() {
        // A Compose service's container: its alias on the project network.
        let (target, host) = match (&snapshot.compose, &domain.compose_service) {
            (Some(_), Some(sub)) => (
                format!("{base}-{sub}-{}", domain.port),
                compose_alias(&base, sub),
            ),
            _ => (format!("{base}-{}", domain.port), base.clone()),
        };
        http.services.entry(target.clone()).or_insert_with(|| LoadBalanced {
            load_balancer: json!({"servers": [{"url": format!("http://{host}:{}", domain.port)}]}),
        });
        // Host and path were checked (no backticks, nothing that ends a rule).
        let mut rule = format!("Host(`{}`)", domain.host);
        if domain.path != "/" {
            rule.push_str(&format!(" && PathPrefix(`{}`)", domain.path));
        }
        let router = format!("{base}-{index}");
        if domain.https {
            let tls = if options.cert_resolver.is_empty() {
                json!({})
            } else {
                json!({"certResolver": options.cert_resolver})
            };
            http.routers.insert(
                router.clone(),
                Router {
                    rule: rule.clone(),
                    entry_points: vec![HTTPS_ENTRYPOINT.to_owned()],
                    service: target.clone(),
                    middlewares: access.clone(),
                    tls: Some(tls),
                },
            );
            http.routers.insert(
                format!("{router}-http"),
                Router {
                    rule,
                    entry_points: vec![HTTP_ENTRYPOINT.to_owned()],
                    service: target,
                    middlewares: vec![redirect.clone()],
                    tls: None,
                },
            );
            http.middlewares
                .entry(redirect.clone())
                .or_insert_with(|| redirect_to_https(options.https_port));
        } else {
            http.routers.insert(
                router,
                Router {
                    rule,
                    entry_points: vec![HTTP_ENTRYPOINT.to_owned()],
                    service: target,
                    middlewares: access.clone(),
                    tls: None,
                },
            );
        }
    }
    Some(TraefikFile { http })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redirects_traefik_takes() {
        assert!(check_redirect("^https?://a.example/(.*)", "https://b.example/${1}").is_ok());
        assert!(check_redirect("(?i)^http://A/(.*)", "x").is_ok());
        for (regex, replacement) in [
            ("^x", "{{ env \"SECRET\" }}"),
            ("{{.}}", "x"),
            ("([", "x"),
            ("(?x)^a b", "x"),
            ("^x", " "),
        ] {
            assert!(
                check_redirect(regex, replacement).is_err(),
                "{regex} {replacement}"
            );
        }
        assert!(
            valid_auth_user("ops@shop") && !valid_auth_user("a:b") && !valid_auth_user("{{x}}")
        );
    }

    /// The spec as JSON with every object's keys sorted (labels are hash
    /// maps: their order changes between runs).
    fn canonical(spec: &ServiceSpec) -> String {
        fn sort(value: Value) -> Value {
            match value {
                Value::Object(map) => {
                    let sorted: BTreeMap<String, Value> =
                        map.into_iter().map(|(k, v)| (k, sort(v))).collect();
                    Value::Object(sorted.into_iter().collect())
                }
                Value::Array(items) => Value::Array(items.into_iter().map(sort).collect()),
                other => other,
            }
        }
        let sorted = sort(serde_json::to_value(spec).unwrap());
        // Printed through a BTreeMap again: serde_json may keep insertion order.
        let printable: BTreeMap<String, Value> = sorted
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        serde_json::to_string_pretty(&printable).unwrap()
    }

    fn snapshot() -> Snapshot {
        Snapshot {
            project: "shop".to_owned(),
            service: "web".to_owned(),
            image: "nginx:1.27".to_owned(),
            command: None,
            replicas: 2,
            running: true,
            cpu_limit_millis: Some(500),
            mem_limit_mb: Some(256),
            cpu_reservation_millis: None,
            mem_reservation_mb: Some(64),
            zero_downtime: true,
            healthcheck: Some(Healthcheck {
                command: "wget -qO- http://127.0.0.1/ || exit 1".to_owned(),
                interval_s: 10,
                retries: 3,
            }),
            domains: vec![
                Domain {
                    host: "shop.example.com".to_owned(),
                    path: "/".to_owned(),
                    port: 80,
                    https: true,
                    compose_service: None,
                },
                Domain {
                    host: "shop.example.com".to_owned(),
                    path: "/api".to_owned(),
                    port: 8080,
                    https: true,
                    compose_service: None,
                },
                Domain {
                    host: "shop.localhost".to_owned(),
                    path: "/".to_owned(),
                    port: 80,
                    https: false,
                    compose_service: None,
                },
            ],
            ports: vec![],
            mounts: vec![Mount {
                target: "/usr/share/nginx/html".to_owned(),
                kind: "volume".to_owned(),
                source: "html".to_owned(),
                read_only: true,
                content_digest: None,
            }],
            env_names: vec![],
            env_digest: None,
            registry_username: None,
            primary_domain: None,
            build: None,
            compose: None,
            redirects: vec![],
            basic_auth: vec![],
        }
    }

    fn pinned() -> PinnedImage {
        PinnedImage::Digest("nginx:1.27@sha256:abc".to_owned())
    }

    fn production() -> DeployOptions {
        DeployOptions {
            force_update: 0,
            cert_resolver: "letsencrypt".to_owned(),
            https_port: 443,
        }
    }

    #[test]
    fn a_web_service_with_domains() {
        let out = desired(&snapshot(), &pinned(), &Runtime::default(), &production());
        insta::assert_snapshot!("web_service_spec", canonical(&out.spec));
        insta::assert_snapshot!("web_service_routes", out.routes.unwrap().to_yaml());
    }

    #[test]
    fn a_stopped_service_has_no_tasks_and_no_routes() {
        let mut stopped = snapshot();
        stopped.running = false;
        let out = desired(&stopped, &pinned(), &Runtime::default(), &production());
        assert_eq!(out.spec.mode.unwrap().replicated.unwrap().replicas, Some(0));
        assert!(out.routes.is_none());
    }

    #[test]
    fn a_worker_with_ports_command_and_bind_mount() {
        let worker = Snapshot {
            service: "worker".to_owned(),
            image: "ghcr.io/org/worker:2".to_owned(),
            command: Some("worker --queue 'high priority'".to_owned()),
            replicas: 1,
            zero_downtime: true,
            healthcheck: None,
            domains: vec![],
            ports: vec![Port {
                published: 5432,
                target: 5432,
                protocol: "tcp".to_owned(),
                mode: "host".to_owned(),
            }],
            mounts: vec![Mount {
                target: "/data".to_owned(),
                kind: "bind".to_owned(),
                source: "/srv/shop/data".to_owned(),
                read_only: false,
                content_digest: None,
            }],
            cpu_limit_millis: None,
            mem_limit_mb: None,
            cpu_reservation_millis: None,
            mem_reservation_mb: None,
            ..snapshot()
        };
        let out = desired(
            &worker,
            &PinnedImage::Local("sha256:local".to_owned()),
            &Runtime::default(),
            &DeployOptions::default(),
        );
        // Zero downtime asked, but without a health check (and with a host port) it stops first.
        assert!(!worker.zero_downtime_possible());
        insta::assert_snapshot!("worker_service_spec", canonical(&out.spec));
        assert!(out.routes.is_none());
    }

    #[test]
    fn development_https_uses_the_default_certificate_and_its_port() {
        let dev = DeployOptions {
            https_port: 8443,
            ..DeployOptions::default()
        };
        let yaml = desired(&snapshot(), &pinned(), &Runtime::default(), &dev)
            .routes
            .unwrap()
            .to_yaml();
        assert!(
            !yaml.contains("certResolver") && yaml.contains("tls: {}"),
            "{yaml}"
        );
        insta::assert_snapshot!("development_redirect", yaml);
    }

    #[test]
    fn routing_and_scaling_changes_keep_the_task_template() {
        let base = desired(&snapshot(), &pinned(), &Runtime::default(), &production());
        let mut changed = snapshot();
        changed.replicas = 5;
        changed.domains.pop();
        let after = desired(&changed, &pinned(), &Runtime::default(), &production());
        assert_ne!(after.hash, base.hash);
        assert_eq!(after.spec.task_template, base.spec.task_template);
    }

    #[test]
    fn the_environment_stays_out_of_labels_and_debug() {
        let env = BTreeMap::from([("DB_PASSWORD".to_owned(), "s3cret-7f3a".to_owned())]);
        let with_env = snapshot().with_env(&env, b"a key");
        assert_eq!(with_env.env_names, ["DB_PASSWORD"]);
        assert!(!serde_json::to_string(&with_env).unwrap().contains("s3cret"));
        let out = desired(
            &with_env,
            &pinned(),
            &Runtime {
                env: env.clone(),
                ..Runtime::default()
            },
            &production(),
        );
        let container = out
            .spec
            .task_template
            .clone()
            .unwrap()
            .container_spec
            .unwrap();
        assert_eq!(container.env.unwrap(), ["DB_PASSWORD=s3cret-7f3a"]);
        assert!(!format!("{:?}", container.labels).contains("s3cret"));
        assert!(!format!("{out:?}").contains("s3cret"));
        // Another value, another template (the tasks restart) and hash.
        let other = BTreeMap::from([("DB_PASSWORD".to_owned(), "rotated".to_owned())]);
        let changed = snapshot().with_env(&other, b"a key");
        assert_ne!(changed.hash(), with_env.hash());
        assert_ne!(
            desired(
                &changed,
                &pinned(),
                &Runtime {
                    env: other.clone(),
                    ..Runtime::default()
                },
                &production()
            )
            .template,
            out.template
        );
        // Keyed: another key, another digest.
        assert_ne!(
            snapshot().with_env(&env, b"another").env_digest,
            with_env.env_digest
        );
    }

    #[test]
    fn the_template_follows_what_tasks_run() {
        let options = DeployOptions::default();
        let template = |s: &Snapshot, o: &DeployOptions| {
            desired(s, &pinned(), &Runtime::default(), o).template
        };
        let base = template(&snapshot(), &options);
        // More replicas, another domain: the same tasks.
        let mut scaled = snapshot();
        scaled.replicas += 3;
        scaled.domains.pop();
        assert_eq!(template(&scaled, &options), base);
        // A restart, another command: new tasks.
        let restarted = DeployOptions {
            force_update: 1,
            ..DeployOptions::default()
        };
        assert_ne!(template(&snapshot(), &restarted), base);
        let mut command = snapshot();
        command.command = Some("nginx -g 'daemon off;'".to_owned());
        assert_ne!(template(&command, &options), base);
        // Carried by the containers.
        let out = desired(&snapshot(), &pinned(), &Runtime::default(), &options);
        let labels = out
            .spec
            .task_template
            .unwrap()
            .container_spec
            .unwrap()
            .labels
            .unwrap();
        assert_eq!(labels[TEMPLATE_LABEL], base);
    }

    #[test]
    fn hashes_follow_the_configuration_only() {
        let base = snapshot();
        let mut reordered = base.clone();
        reordered.domains.reverse();
        let rows_order = |s: &Snapshot| {
            let mut s = s.clone();
            s.domains.sort();
            s
        };
        assert_eq!(rows_order(&reordered).hash(), base.hash());
        let mut changed = base.clone();
        changed.replicas = 3;
        assert_ne!(changed.hash(), base.hash());
        assert_eq!(base.hash().len(), 64);
        // A restart (force_update) is not a configuration change.
        let a = desired(
            &base,
            &pinned(),
            &Runtime::default(),
            &DeployOptions {
                force_update: 1,
                ..production()
            },
        );
        assert_eq!(a.hash, base.hash());
    }

    #[test]
    fn commands_split_like_a_shell() {
        assert_eq!(
            command_args("npm run start"),
            Some(vec!["npm".into(), "run".into(), "start".into()])
        );
        assert_eq!(
            command_args(r#"sh -c "echo 'a b'""#),
            Some(vec!["sh".into(), "-c".into(), "echo 'a b'".into()])
        );
        assert_eq!(command_args("unclosed 'quote"), None);
        assert_eq!(command_args("   "), None);
    }

    #[test]
    fn images_pin_to_what_was_pulled() {
        let details = |digests: &[&str]| ImageDetails {
            id: "sha256:local".to_owned(),
            repo_digests: digests.iter().map(|d| (*d).to_owned()).collect(),
        };
        let hub = details(&["nginx@sha256:aaa"]);
        let pulled = format!("sha256:{}", "d".repeat(64));
        // The digest the pull reported wins.
        assert_eq!(
            pin_image("nginx:1.27", Some(&pulled), &hub).digest(),
            Some(pulled.as_str())
        );
        assert_eq!(
            pin_image("nginx:1.27", Some("sha256:short"), &hub).digest(),
            Some("sha256:aaa")
        );
        // Else the repository's only digest, however Docker Hub is named.
        assert_eq!(
            pin_image("nginx:1.27", None, &hub),
            PinnedImage::Digest("nginx:1.27@sha256:aaa".to_owned())
        );
        assert_eq!(
            pin_image("docker.io/library/nginx:1.27", None, &hub).digest(),
            Some("sha256:aaa")
        );
        let mirror = details(&["mirror.local:5000/nginx@sha256:bbb", "nginx@sha256:aaa"]);
        assert_eq!(
            pin_image("mirror.local:5000/nginx:1.27", None, &mirror).digest(),
            Some("sha256:bbb")
        );
        // Several for one repository: ambiguous, in either order.
        for order in [
            ["nginx@sha256:aaa", "nginx@sha256:ccc"],
            ["nginx@sha256:ccc", "nginx@sha256:aaa"],
        ] {
            assert_eq!(
                pin_image("nginx:1.27", None, &details(&order)),
                PinnedImage::Local("sha256:local".to_owned())
            );
        }
        // Built here: its image id, with a warning.
        let local = pin_image("app:dev", None, &details(&[]));
        assert_eq!(local, PinnedImage::Local("sha256:local".to_owned()));
        assert!(local.warning().is_some());
        // Already pinned: as configured.
        assert_eq!(
            pin_image("nginx@sha256:ccc", Some(&pulled), &hub).digest(),
            Some("sha256:ccc")
        );
    }

    #[test]
    fn pulls_report_their_digest() {
        let digest = format!("sha256:{}", "a".repeat(64));
        let lines = [
            "1.27: Pulling from library/nginx",
            "Pull complete",
            &format!("Digest: {digest}"),
            "Status: Downloaded",
        ];
        assert_eq!(pulled_digest(lines), Some(digest));
        assert_eq!(pulled_digest(["Digest: sha256:nothex!"]), None);
    }
}
