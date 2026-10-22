//! Docker: what the panel may see and do on the Docker engine.
//!
//! [`DockerApi`] is the engine (Bollard over the local socket in production,
//! a fake in tests). [`Docker`] wraps it and is what the app binds and
//! callers use: it hides the panel's own objects (label [`SYSTEM_LABEL`]:
//! the panel, its MySQL, Traefik) and refuses to touch them unless the caller
//! may manage the system, keeps prune away from them, and redacts secrets
//! out of `inspect`.

mod bollard;

use std::collections::HashMap;
use std::sync::Arc;

use futures_util::StreamExt;
use futures_util::stream::BoxStream;
use rustcms::{AppError, async_trait};
use serde::Serialize;
use serde_json::Value;

pub use self::bollard::BollardDocker;

/// Label the installer puts on the panel's own services, volumes, networks
/// and secrets.
pub const SYSTEM_LABEL: &str = "rdockpanel.system";

/// Env names containing one of these have their value replaced in `inspect`.
const SECRET_MARKERS: [&str; 9] = [
    "PASSWORD",
    "PASSWD",
    "PASS",
    "PWD",
    "SECRET",
    "TOKEN",
    "KEY",
    "CREDENTIAL",
    "AUTH",
];
const REDACTED: &str = "******";

#[derive(Debug, thiserror::Error)]
pub enum DockerError {
    #[error("no such Docker object")]
    NotFound,
    #[error("Docker refused: {0}")]
    Conflict(String),
    #[error("the panel's own objects are protected")]
    Protected,
    #[error("Docker is unavailable: {0}")]
    Unavailable(String),
    #[error("Docker error: {0}")]
    Other(String),
}

impl From<DockerError> for AppError {
    fn from(err: DockerError) -> Self {
        match err {
            DockerError::NotFound => AppError::NotFound,
            DockerError::Protected => AppError::Forbidden,
            DockerError::Conflict(_) => AppError::Conflict {
                code: "docker_conflict",
                reason: "docker",
                message: "Docker refused the operation in the object's current state",
            },
            DockerError::Unavailable(_) | DockerError::Other(_) => AppError::internal(err),
        }
    }
}

pub type DockerResult<T> = Result<T, DockerError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SwarmRole {
    Manager,
    Worker,
    Inactive,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DockerInfo {
    pub name: String,
    pub server_version: String,
    pub operating_system: String,
    pub kernel_version: String,
    pub architecture: String,
    pub cpus: i64,
    pub mem_total: i64,
    pub containers: i64,
    pub containers_running: i64,
    pub images: i64,
    pub swarm: SwarmRole,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ContainerItem {
    pub id: String,
    pub name: String,
    pub image: String,
    /// The image's id (`sha256:…`), whatever tag `image` shows.
    pub image_id: String,
    /// `running`, `exited`, `paused`, …
    pub state: String,
    /// Human status (`Up 3 hours`).
    pub status: String,
    /// Unix seconds.
    pub created: i64,
    /// `0.0.0.0:8080->80/tcp`, …
    pub ports: Vec<String>,
    pub labels: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ImageItem {
    pub id: String,
    pub tags: Vec<String>,
    pub size: i64,
    pub created: i64,
    /// Containers using it (-1 when Docker did not count).
    pub containers: i64,
    pub labels: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VolumeItem {
    pub name: String,
    pub driver: String,
    pub mountpoint: String,
    pub labels: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NetworkItem {
    pub id: String,
    pub name: String,
    pub driver: String,
    pub scope: String,
    pub labels: HashMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContainerAction {
    Start,
    Stop,
    Restart,
    Kill,
    Pause,
    Unpause,
}

#[derive(Debug, Clone, Default)]
pub struct LogQuery {
    pub follow: bool,
    /// Lines from the end (`None`: all).
    pub tail: Option<u32>,
    /// Unix seconds.
    pub since: Option<i64>,
    pub timestamps: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LogStream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LogLine {
    pub stream: LogStream,
    pub text: String,
}

/// One reading of a container's resources; network and block I/O are totals
/// since the container started.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ContainerStats {
    pub cpu_percent: f64,
    pub mem_used: u64,
    pub mem_limit: u64,
    pub net_rx: u64,
    pub net_tx: u64,
    pub blk_read: u64,
    pub blk_write: u64,
}

pub use ::bollard::models::{
    ContainerStatus, NetworkAttachmentConfig, ObjectVersion, Service as SwarmService, ServiceSpec,
    ServiceUpdateStatus, ServiceUpdateStatusStateEnum, Task as SwarmTask, TaskState, TaskStatus,
};

/// Marks the Swarm services, networks and volumes the panel deploys (with
/// `rdockpanel.project` and `rdockpanel.service`): it never touches a
/// service without it.
pub const MANAGED_LABEL: &str = "rdockpanel.managed";

/// A line of an image's build, or its end: the image's id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildLine {
    Log(String),
    Image(String),
}

/// What a command run in a container ([`DockerApi::run_in_container`])
/// writes, then its exit code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecChunk {
    Out(Vec<u8>),
    Err(Vec<u8>),
    Exit(i64),
}

/// Registry credentials for pulling a private image (sent with service
/// create/update so every node can pull it).
#[derive(Clone, Default, PartialEq, Eq)]
pub struct RegistryAuth {
    pub server: String,
    pub username: String,
    pub password: String,
}

impl std::fmt::Debug for RegistryAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegistryAuth")
            .field("server", &self.server)
            .field("username", &self.username)
            .finish_non_exhaustive()
    }
}

/// What the panel needs of an image: its id and the registry digests it was
/// pulled as (`repo@sha256:…`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImageDetails {
    pub id: String,
    pub repo_digests: Vec<String>,
}

/// A network the panel creates (with its labels).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NetworkRequest {
    pub name: String,
    pub driver: String,
    pub labels: HashMap<String, String>,
    pub attachable: bool,
    pub internal: bool,
}

/// An interactive shell in a container (a TTY): its output, its input, and
/// the exec's id for resizing. Dropping it does not end the shell (Docker
/// keeps exec processes after their connection closes): [`Shell`] does.
pub struct ExecSession {
    pub id: String,
    pub output: BoxStream<'static, DockerResult<Vec<u8>>>,
    pub input: std::pin::Pin<Box<dyn tokio::io::AsyncWrite + Send>>,
}

/// A terminal's size in characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
pub struct TermSize {
    pub cols: u16,
    pub rows: u16,
}

impl TermSize {
    /// Within what a screen can show (Docker refuses 0).
    pub fn clamped(self) -> Self {
        Self {
            cols: self.cols.clamp(10, 500),
            rows: self.rows.clamp(4, 200),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct PruneReport {
    pub containers_deleted: usize,
    pub images_deleted: usize,
    #[serde(skip_serializing_if = "is_zero")]
    pub volumes_deleted: usize,
    #[serde(skip_serializing_if = "is_zero")]
    pub build_caches_deleted: usize,
    pub space_reclaimed: u64,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// What Settings → Panel → Docker removes, each on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PruneKind {
    /// Stopped containers.
    Containers,
    /// Images no container uses (tagged ones too).
    Images,
    /// The builder's cache.
    BuildCache,
    /// Volumes no container uses, but the panel's services' and Compose's
    /// ([`Docker::prune_volumes`]).
    Volumes,
}

/// The Docker engine. Implementations report what Docker says, unfiltered;
/// [`Docker`] applies the panel's rules.
#[async_trait]
pub trait DockerApi: Send + Sync {
    async fn info(&self) -> DockerResult<DockerInfo>;
    async fn containers(&self) -> DockerResult<Vec<ContainerItem>>;
    /// Docker's inspect document.
    async fn inspect_container(&self, id: &str) -> DockerResult<Value>;
    async fn container_action(&self, id: &str, action: ContainerAction) -> DockerResult<()>;
    async fn remove_container(&self, id: &str, force: bool) -> DockerResult<()>;
    fn logs(&self, id: &str, query: LogQuery) -> BoxStream<'static, DockerResult<LogLine>>;
    fn stats(&self, id: &str) -> BoxStream<'static, DockerResult<ContainerStats>>;
    async fn images(&self) -> DockerResult<Vec<ImageItem>>;
    /// Progress lines while pulling `image` (`name[:tag]`), with `auth` for
    /// a private registry.
    fn pull_image(
        &self,
        image: &str,
        auth: Option<RegistryAuth>,
    ) -> BoxStream<'static, DockerResult<String>>;
    async fn remove_image(&self, id: &str, force: bool) -> DockerResult<()>;
    /// Removes stopped containers and dangling images, except those
    /// labelled `exclude_label=true`.
    async fn prune(&self, exclude_label: &str) -> DockerResult<PruneReport>;
    async fn volumes(&self) -> DockerResult<Vec<VolumeItem>>;
    async fn create_volume(&self, name: &str) -> DockerResult<VolumeItem>;
    async fn remove_volume(&self, name: &str) -> DockerResult<()>;
    async fn networks(&self) -> DockerResult<Vec<NetworkItem>>;
    async fn create_network(&self, name: &str, driver: &str) -> DockerResult<NetworkItem>;
    async fn remove_network(&self, id: &str) -> DockerResult<()>;
    /// Creates the network unless one of that name exists; the network as it
    /// is (callers check it: [`Docker::ensure_network`]).
    async fn ensure_network(&self, request: NetworkRequest) -> DockerResult<NetworkItem>;
    /// The subnets of the network `name` (its IPAM's; none known: empty).
    async fn network_subnets(&self, _name: &str) -> DockerResult<Vec<String>> {
        Ok(vec![])
    }
    async fn inspect_image(&self, reference: &str) -> DockerResult<ImageDetails>;
    /// The Swarm service of that name, if any.
    async fn inspect_service(&self, name: &str) -> DockerResult<Option<SwarmService>>;
    /// Creates a service; its id.
    async fn create_service(
        &self,
        spec: ServiceSpec,
        auth: Option<RegistryAuth>,
    ) -> DockerResult<String>;
    /// Updates a service at `version` (from its inspect); Docker's warnings.
    async fn update_service(
        &self,
        name: &str,
        version: u64,
        spec: ServiceSpec,
        auth: Option<RegistryAuth>,
    ) -> DockerResult<Vec<String>>;
    async fn delete_service(&self, name: &str) -> DockerResult<()>;
    /// Builds `context` (a tar) with its Dockerfile at `dockerfile`, tagged
    /// `tag` (`no_cache`: none of the builder's cache): the build's lines,
    /// then the image's id.
    fn build_image(
        &self,
        context: Vec<u8>,
        dockerfile: &str,
        tag: &str,
        no_cache: bool,
    ) -> BoxStream<'static, DockerResult<BuildLine>>;
    /// Creates the Swarm config `name` holding `data`; its id (a name taken:
    /// `Conflict`).
    async fn create_config(
        &self,
        name: &str,
        data: &[u8],
        labels: HashMap<String, String>,
    ) -> DockerResult<String>;
    /// The configs labelled `key=value`: (id, name).
    async fn list_configs(&self, key: &str, value: &str) -> DockerResult<Vec<(String, String)>>;
    /// Removes a config (refused while a service uses it: `Conflict`).
    async fn delete_config(&self, id: &str) -> DockerResult<()>;
    /// Creates the Swarm secret `name` holding `data`; its id (a name taken:
    /// `Conflict`). Secrets are never read back.
    async fn create_secret(
        &self,
        name: &str,
        data: &[u8],
        labels: HashMap<String, String>,
    ) -> DockerResult<String>;
    /// The secrets labelled `key=value`: (id, name).
    async fn list_secrets(&self, key: &str, value: &str) -> DockerResult<Vec<(String, String)>>;
    /// Removes a secret (refused while a service uses it: `Conflict`).
    async fn delete_secret(&self, id: &str) -> DockerResult<()>;
    /// The service's tasks (every slot, current and past).
    async fn list_tasks(&self, service: &str) -> DockerResult<Vec<SwarmTask>>;
    /// The memory (bytes) the tasks meant to run and placed on a node
    /// reserve, on every node.
    async fn reserved_memory(&self) -> DockerResult<u64> {
        Ok(0)
    }
    fn service_logs(
        &self,
        service: &str,
        query: LogQuery,
    ) -> BoxStream<'static, DockerResult<LogLine>>;
    /// A shell (bash when present, else sh) on a TTY of `size`, its
    /// processes marked with the environment variable [`SESSION_ENV`]`=token`.
    async fn exec(&self, container: &str, size: TermSize, token: &str)
    -> DockerResult<ExecSession>;
    async fn resize_exec(&self, exec_id: &str, size: TermSize) -> DockerResult<()>;
    /// Runs `cmd` in the container (no TTY; the container's environment),
    /// `input` fed to its standard input: what it writes, then its exit code.
    /// Its processes carry [`SESSION_ENV`]`=mark`: killed if the caller goes
    /// away, or later by [`DockerApi::kill_marked`].
    async fn run_in_container(
        &self,
        container: &str,
        cmd: Vec<String>,
        mark: &str,
        input: Option<BoxStream<'static, Vec<u8>>>,
    ) -> DockerResult<BoxStream<'static, DockerResult<ExecChunk>>>;
    /// Kills the container's processes carrying [`SESSION_ENV`]`=mark`.
    async fn kill_marked(&self, container: &str, mark: &str) -> DockerResult<()>;
    /// `docker compose --project-directory dir -p project -f file args…`
    /// (the Docker CLI's Compose): its exit code and output.
    async fn compose(
        &self,
        dir: &std::path::Path,
        project: &str,
        file: &str,
        args: &[&str],
    ) -> DockerResult<(i32, String)>;
    /// Ends every process of the container's sessions marked with `token`:
    /// hangs them up, then kills what is left.
    async fn end_exec(&self, container: &str, token: &str) -> DockerResult<()>;
    /// Removes stopped containers, unused images or the build cache, except
    /// what is labelled `exclude_label=true`.
    async fn prune_kind(&self, kind: PruneKind, exclude_label: &str) -> DockerResult<PruneReport> {
        let _ = (kind, exclude_label);
        Err(DockerError::Unavailable(
            "prune is not supported".to_owned(),
        ))
    }
    /// Volumes no container (running or stopped) mounts.
    async fn unused_volumes(&self) -> DockerResult<Vec<VolumeItem>> {
        Err(DockerError::Unavailable(
            "volume usage is not supported".to_owned(),
        ))
    }
    /// The digest the registry serves `reference` (`name:tag`) as now: a
    /// multi-platform image's index. None: the registry did not say.
    async fn registry_digest(&self, reference: &str) -> DockerResult<Option<String>> {
        let _ = reference;
        Err(DockerError::Unavailable(
            "registry lookups are not supported".to_owned(),
        ))
    }
    /// Starts a helper: a privileged container of `image` (local) in the
    /// host's process namespace, idle for at most `lifetime` seconds, kept
    /// (no auto-remove) until [`DockerApi::remove_container`]. Host commands
    /// run in it through [`DockerApi::run_in_container`] (`nsenter -t 1`):
    /// their output and exit code are read before it goes. Its id.
    async fn start_host_helper(&self, image: &str, lifetime: u32) -> DockerResult<String> {
        let _ = (image, lifetime);
        Err(DockerError::Unavailable(
            "host commands are not supported".to_owned(),
        ))
    }
    /// Starts `cmd` in a new privileged container of `image` (local) in the
    /// host's process namespace, removed when it ends; not waited for.
    async fn run_on_host(&self, image: &str, cmd: Vec<String>) -> DockerResult<()> {
        let _ = (image, cmd);
        Err(DockerError::Unavailable(
            "host commands are not supported".to_owned(),
        ))
    }
}

/// Whether the caller may see and touch the panel's own objects
/// (`system.manage`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    User,
    System,
}

/// The Docker service the app binds (`app.service::<Docker>()`).
#[derive(Clone)]
pub struct Docker(Arc<dyn DockerApi>);

impl Docker {
    pub fn new(api: impl DockerApi + 'static) -> Self {
        Self(Arc::new(api))
    }

    /// Bollard on the local socket (or `DOCKER_HOST`); when no client can be
    /// built, every call reports [`DockerError::Unavailable`] instead of the
    /// app failing to boot.
    pub fn from_env() -> Self {
        match BollardDocker::connect() {
            Ok(api) => Self::new(api),
            Err(err) => {
                tracing::warn!(%err, "Docker client unavailable");
                Self::new(Unavailable(err.to_string()))
            }
        }
    }

    pub async fn info(&self) -> DockerResult<DockerInfo> {
        self.0.info().await
    }

    pub async fn containers(&self, scope: Scope) -> DockerResult<Vec<ContainerItem>> {
        Ok(visible(self.0.containers().await?, scope, |c| &c.labels))
    }

    /// The inspect document with secrets in env values replaced.
    pub async fn inspect_container(&self, id: &str, scope: Scope) -> DockerResult<Value> {
        let (_, mut doc) = self.resolve_container(id, scope).await?;
        redact_env(&mut doc);
        Ok(doc)
    }

    pub async fn container_action(
        &self,
        id: &str,
        action: ContainerAction,
        scope: Scope,
    ) -> DockerResult<()> {
        let (id, _) = self.resolve_container(id, scope).await?;
        self.0.container_action(&id, action).await
    }

    pub async fn remove_container(&self, id: &str, force: bool, scope: Scope) -> DockerResult<()> {
        let (id, _) = self.resolve_container(id, scope).await?;
        self.0.remove_container(&id, force).await
    }

    pub async fn logs(
        &self,
        id: &str,
        query: LogQuery,
        scope: Scope,
    ) -> DockerResult<BoxStream<'static, DockerResult<LogLine>>> {
        let (id, _) = self.resolve_container(id, scope).await?;
        Ok(self.0.logs(&id, query))
    }

    pub async fn stats(
        &self,
        id: &str,
        scope: Scope,
    ) -> DockerResult<BoxStream<'static, DockerResult<ContainerStats>>> {
        let (id, _) = self.resolve_container(id, scope).await?;
        Ok(self.0.stats(&id))
    }

    pub async fn images(&self) -> DockerResult<Vec<ImageItem>> {
        self.0.images().await
    }

    pub fn pull_image(&self, image: &str) -> BoxStream<'static, DockerResult<String>> {
        self.0.pull_image(image, None)
    }

    /// [`Self::pull_image`] from a private registry.
    pub fn pull_image_with(
        &self,
        image: &str,
        auth: Option<RegistryAuth>,
    ) -> BoxStream<'static, DockerResult<String>> {
        self.0.pull_image(image, auth)
    }

    /// Removes the image `reference` (its id, or one of its tags exactly as
    /// listed). Images carry no label: one a panel container runs is the
    /// panel's, and `force` needs the system scope too.
    pub async fn remove_image(
        &self,
        reference: &str,
        force: bool,
        scope: Scope,
    ) -> DockerResult<()> {
        let image = self
            .0
            .images()
            .await?
            .into_iter()
            .find(|i| {
                i.id == reference
                    || short_id(&i.id) == reference
                    || i.tags.iter().any(|t| t == reference)
            })
            .ok_or(DockerError::NotFound)?;
        if scope == Scope::User {
            if force {
                return Err(DockerError::Protected);
            }
            let containers = self.0.containers().await?;
            if containers
                .iter()
                .any(|c| is_system(&c.labels) && c.image_id == image.id)
            {
                return Err(DockerError::Protected);
            }
        }
        // Docker's own string for it: never the caller's, which could carry
        // path segments into the Engine API URL.
        let target = if image.id == reference {
            image.id.clone()
        } else {
            reference_from(&image, reference)
        };
        self.0.remove_image(&target, force).await
    }

    pub async fn prune(&self) -> DockerResult<PruneReport> {
        self.0.prune(SYSTEM_LABEL).await
    }

    /// Each image with how its containers are: running (or paused,
    /// restarting) and stopped (exited, created, dead). `None`: Docker did
    /// not list its containers (unknown, never guessed).
    pub async fn images_with_use(&self) -> DockerResult<Vec<(ImageItem, Option<(usize, usize)>)>> {
        let images = self.0.images().await?;
        let containers = self.0.containers().await.ok();
        Ok(images
            .into_iter()
            .map(|image| {
                let usage = containers.as_ref().map(|list| {
                    let mine = list.iter().filter(|c| c.image_id == image.id);
                    mine.fold((0, 0), |(running, stopped), c| {
                        if is_stopped(&c.state) {
                            (running, stopped + 1)
                        } else {
                            (running + 1, stopped)
                        }
                    })
                });
                (image, usage)
            })
            .collect())
    }

    /// The panel's own old versions (its services': the panel, Traefik,
    /// MySQL, cloudflared): their old tasks' stopped containers, then the
    /// images left without a container. Kept: every image a container runs,
    /// and each service's previous image (its newest stopped task of
    /// another image: what a rollback or a look back needs). `apply`:
    /// removed (never forced), else only said.
    pub async fn system_cleanup(&self, apply: bool) -> DockerResult<SystemCleanup> {
        const SERVICE: &str = "com.docker.swarm.service.name";
        let containers = self.0.containers().await?;
        let mut kept: std::collections::HashSet<String> = containers
            .iter()
            .filter(|c| !is_stopped(&c.state))
            .map(|c| c.image_id.clone())
            .collect();
        let mut services: HashMap<&str, Vec<&ContainerItem>> = HashMap::new();
        for c in containers.iter().filter(|c| is_system(&c.labels)) {
            if let Some(service) = c.labels.get(SERVICE) {
                services.entry(service.as_str()).or_default().push(c);
            }
        }
        for tasks in services.values_mut() {
            tasks.sort_by_key(|c| std::cmp::Reverse(c.created));
            // Its current version: what runs, else (stopped, failing) its
            // newest task's.
            let mut current: std::collections::HashSet<&str> = tasks
                .iter()
                .filter(|c| !is_stopped(&c.state))
                .map(|c| c.image_id.as_str())
                .collect();
            if current.is_empty()
                && let Some(newest) = tasks.first()
            {
                current.insert(newest.image_id.as_str());
            }
            kept.extend(current.iter().map(|id| (*id).to_owned()));
            if let Some(previous) = tasks
                .iter()
                .find(|c| !current.contains(c.image_id.as_str()))
            {
                kept.insert(previous.image_id.clone());
            }
        }
        let old: Vec<&ContainerItem> = services
            .values()
            .flatten()
            .copied()
            .filter(|c| is_stopped(&c.state) && !kept.contains(&c.image_id))
            .collect();
        let candidates: std::collections::HashSet<&str> =
            old.iter().map(|c| c.image_id.as_str()).collect();
        let mut report = SystemCleanup::default();
        let images = self.0.images().await?;
        for image in images.iter().filter(|i| candidates.contains(i.id.as_str())) {
            // Only those no other container uses.
            let others = containers
                .iter()
                .any(|c| c.image_id == image.id && !old.iter().any(|o| o.id == c.id));
            if !others {
                report.images.push(serde_json::json!({
                    "id": image.id,
                    "tags": image.tags,
                    "size": image.size,
                }));
                report.space += u64::try_from(image.size).unwrap_or(0);
            }
        }
        report.containers = old.len();
        if !apply {
            return Ok(report);
        }
        for c in &old {
            if let Err(err) = self.0.remove_container(&c.id, false).await {
                tracing::warn!(%err, container = c.name, "an old task's container not removed");
            }
        }
        let mut removed = Vec::new();
        for image in std::mem::take(&mut report.images) {
            let id = image["id"].as_str().unwrap_or_default().to_owned();
            match self.0.remove_image(&id, false).await {
                Ok(()) => removed.push(image),
                Err(err) => {
                    report.space = report
                        .space
                        .saturating_sub(image["size"].as_u64().unwrap_or(0));
                    tracing::warn!(%err, image = id, "an old image not removed");
                }
            }
        }
        report.images = removed;
        Ok(report)
    }

    /// Removes what `kind` names; never the panel's own objects, nor (for
    /// volumes) data a service of the panel's or a Compose project keeps.
    pub async fn prune_kind(&self, kind: PruneKind) -> DockerResult<PruneReport> {
        match kind {
            PruneKind::Volumes => self.prune_volumes().await,
            kind => self.0.prune_kind(kind, SYSTEM_LABEL).await,
        }
    }

    /// Unused volumes, but those the panel's services keep (`rdp_<project>_…`,
    /// kept when a service is destroyed), Compose's and Swarm stacks', and
    /// the panel's own.
    async fn prune_volumes(&self) -> DockerResult<PruneReport> {
        let mut report = PruneReport::default();
        for volume in self.0.unused_volumes().await? {
            if !disposable_volume(&volume) {
                continue;
            }
            match self.0.remove_volume(&volume.name).await {
                Ok(()) => report.volumes_deleted += 1,
                // Taken meanwhile, or gone: left.
                Err(DockerError::Conflict(_) | DockerError::NotFound) => {}
                Err(err) => return Err(err),
            }
        }
        Ok(report)
    }

    pub async fn volumes(&self, scope: Scope) -> DockerResult<Vec<VolumeItem>> {
        Ok(visible(self.0.volumes().await?, scope, |v| &v.labels))
    }

    /// Creates `name`; an existing volume is a conflict (Docker would hand it
    /// back, panel's own included).
    pub async fn create_volume(&self, name: &str) -> DockerResult<VolumeItem> {
        checked_name(name)?;
        if self.0.volumes().await?.iter().any(|v| v.name == name) {
            return Err(DockerError::Conflict(format!("volume {name} exists")));
        }
        self.0.create_volume(name).await
    }

    pub async fn remove_volume(&self, name: &str, scope: Scope) -> DockerResult<()> {
        checked_name(name)?;
        let volume = self
            .0
            .volumes()
            .await?
            .into_iter()
            .find(|v| v.name == name)
            .ok_or(DockerError::NotFound)?;
        guard(&volume.labels, scope)?;
        self.0.remove_volume(&volume.name).await
    }

    pub async fn networks(&self, scope: Scope) -> DockerResult<Vec<NetworkItem>> {
        Ok(visible(self.0.networks().await?, scope, |n| &n.labels))
    }

    pub async fn create_network(&self, name: &str, driver: &str) -> DockerResult<NetworkItem> {
        checked_name(name)?;
        if self.0.networks().await?.iter().any(|n| n.name == name) {
            return Err(DockerError::Conflict(format!("network {name} exists")));
        }
        let created = self.0.create_network(name, driver).await;
        // A new bridge: the firewall trusts it at once.
        crate::services::firewall_service::NETWORKS_CHANGED.notify_one();
        created
    }

    /// `id` is a network's full id, else its name (Docker's order); the one
    /// found is checked and removed by its id.
    pub async fn remove_network(&self, id: &str, scope: Scope) -> DockerResult<()> {
        checked_name(id)?;
        let networks = self.0.networks().await?;
        let network = networks
            .iter()
            .find(|n| n.id == id)
            .or_else(|| networks.iter().find(|n| n.name == id))
            .ok_or(DockerError::NotFound)?;
        guard(&network.labels, scope)?;
        self.0.remove_network(&network.id).await
    }

    /// A shell in the container `id` names, if `scope` may use it, marked
    /// with `token` ([`session_token`]); it ends when the returned [`Shell`]
    /// is dropped, also when this future is (the shell may already run).
    pub async fn exec(
        &self,
        id: &str,
        size: TermSize,
        scope: Scope,
        token: &str,
    ) -> DockerResult<Shell> {
        checked_name(token)?;
        let (container, _) = self.resolve_container(id, scope).await?;
        let guard = EndOnDrop {
            docker: self.clone(),
            container: container.clone(),
            token: token.to_owned(),
            armed: true,
        };
        let session = match self.0.exec(&container, size.clamped(), token).await {
            Ok(session) => session,
            // An image without a shell (distroless, scratch): no terminal,
            // and nothing to end.
            Err(err) if no_shell(&err.to_string()) => {
                let mut guard = guard;
                guard.armed = false;
                return Err(DockerError::Conflict(
                    "this image has no shell (/bin/sh): a terminal cannot open in it".to_owned(),
                ));
            }
            Err(err) => return Err(err),
        };
        Ok(Shell {
            session,
            _end: guard,
        })
    }

    /// Ends the processes of the shell marked `token` in `container`.
    pub async fn end_exec(&self, container: &str, token: &str) -> DockerResult<()> {
        checked_name(container)?;
        checked_name(token)?;
        self.0.end_exec(container, token).await
    }

    pub async fn resize_exec(&self, exec_id: &str, size: TermSize) -> DockerResult<()> {
        checked_name(exec_id)?;
        self.0.resize_exec(exec_id, size.clamped()).await
    }

    /// The full id of the container `id` names, if `scope` may see it.
    pub async fn container_id(&self, id: &str, scope: Scope) -> DockerResult<String> {
        self.resolve_container(id, scope).await.map(|(id, _)| id)
    }

    /// The network `request` describes: created, or the existing one of that
    /// name if it is exactly that (driver, and every requested label). A
    /// network of that name made by something else is a conflict: services
    /// are never attached to it.
    pub async fn ensure_network(&self, request: NetworkRequest) -> DockerResult<NetworkItem> {
        checked_name(&request.name)?;
        let network = self.0.ensure_network(request.clone()).await?;
        // Swarm reports a just-created overlay without its driver for a moment:
        // the labels (set only at creation) decide; a different driver refuses.
        // A network made before panels marked theirs (no instance label) is
        // still this panel's; another panel's (another instance) never is.
        let same = (network.driver.is_empty() || network.driver == request.driver)
            && request.labels.iter().all(|(k, v)| {
                let found = network.labels.get(k);
                found == Some(v) || (k == "rdockpanel.instance" && found.is_none())
            });
        if same {
            Ok(network)
        } else {
            Err(DockerError::Conflict(format!(
                "network {} is not this panel's (driver {}, labels {:?})",
                request.name, network.driver, network.labels
            )))
        }
    }

    /// The Swarm service `name` if it is the panel's for `project`/`service`
    /// (`None`: no such service). A service of that name the panel did not
    /// deploy is a conflict: it is never taken over.
    pub async fn managed_service(
        &self,
        name: &str,
        project: &str,
        service: &str,
    ) -> DockerResult<Option<SwarmService>> {
        checked_name(name)?;
        let Some(found) = self.0.inspect_service(name).await? else {
            return Ok(None);
        };
        let labels = found
            .spec
            .as_ref()
            .and_then(|s| s.labels.clone())
            .unwrap_or_default();
        let ours = labels.get(MANAGED_LABEL).is_some_and(|v| v == "true")
            && labels
                .get("rdockpanel.project")
                .is_some_and(|v| v == project)
            && labels
                .get("rdockpanel.service")
                .is_some_and(|v| v == service);
        if ours {
            Ok(Some(found))
        } else {
            Err(DockerError::Conflict(format!(
                "{name} is not deployed by this panel"
            )))
        }
    }

    /// Engine access for the deployment code, which checks ownership with
    /// [`Self::managed_service`] first.
    pub fn engine(&self) -> &dyn DockerApi {
        self.0.as_ref()
    }

    /// The container `id` names (id, short id or name), checked against
    /// `scope`: its full id, which every later call uses so a rename can't
    /// swap the target, and its inspect document.
    async fn resolve_container(&self, id: &str, scope: Scope) -> DockerResult<(String, Value)> {
        checked_name(id)?;
        let doc = self.0.inspect_container(id).await?;
        guard(&labels_of(&doc), scope)?;
        let full_id = doc
            .get("Id")
            .and_then(Value::as_str)
            .ok_or(DockerError::NotFound)?
            .to_owned();
        checked_name(&full_id)?;
        Ok((full_id, doc))
    }
}

/// Container, volume and network identifiers as Docker allows them; anything
/// else (`/`, `..`, `%`) never reaches an Engine API URL.
/// Whether an unused volume may go: none a service of the panel's
/// (`rdp_…`), a Compose project, a Swarm stack or the panel keeps.
pub fn disposable_volume(volume: &VolumeItem) -> bool {
    const KEPT_BY: [&str; 3] = [
        SYSTEM_LABEL,
        "com.docker.compose.project",
        "com.docker.stack.namespace",
    ];
    !volume.name.starts_with("rdp_") && !KEPT_BY.iter().any(|l| volume.labels.contains_key(*l))
}

/// Docker's answer for an image without `/bin/sh`.
fn no_shell(error: &str) -> bool {
    error.contains("/bin/sh") && error.contains("no such file or directory")
}

fn checked_name(name: &str) -> DockerResult<()> {
    let mut chars = name.chars();
    let valid = name.len() <= 255
        && chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'));
    if valid {
        Ok(())
    } else {
        Err(DockerError::NotFound)
    }
}

/// `sha256:abcdef…` → `abcdef…` (the id Docker also accepts).
fn short_id(id: &str) -> &str {
    id.strip_prefix("sha256:").unwrap_or(id)
}

/// The listed string `reference` matched on `image`.
fn reference_from(image: &ImageItem, reference: &str) -> String {
    if short_id(&image.id) == reference {
        return image.id.clone();
    }
    image
        .tags
        .iter()
        .find(|t| *t == reference)
        .cloned()
        .unwrap_or_else(|| image.id.clone())
}

/// Marks a terminal's processes, so they can be ended with it.
pub const SESSION_ENV: &str = "RDOCKPANEL_SESSION";

/// A shell from [`Docker::exec`]: dropping it ends its processes in the
/// container. A panel stopped before that ends them at its next boot
/// (`action_service::interrupted_terminals`).
pub struct Shell {
    pub session: ExecSession,
    _end: EndOnDrop,
}

struct EndOnDrop {
    docker: Docker,
    container: String,
    token: String,
    /// Off: nothing to end (no shell started).
    armed: bool,
}

impl Drop for EndOnDrop {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let (docker, container, token) = (
            self.docker.clone(),
            self.container.clone(),
            self.token.clone(),
        );
        runtime.spawn(async move {
            match docker.0.end_exec(&container, &token).await {
                Ok(()) => {}
                // No shell to end one with: there was none.
                Err(err) if no_shell(&err.to_string()) => {
                    tracing::debug!(%err, container, "no shell to end");
                }
                Err(err) => tracing::warn!(%err, container, "could not end a terminal's shell"),
            }
        });
    }
}

/// Unique per terminal (hex digits only: it goes into a shell script).
pub fn session_token() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!(
        "{nanos:x}{:x}{:x}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

/// The engine when no client could be built.
struct Unavailable(String);

#[async_trait]
impl DockerApi for Unavailable {
    async fn info(&self) -> DockerResult<DockerInfo> {
        Err(self.err())
    }
    async fn containers(&self) -> DockerResult<Vec<ContainerItem>> {
        Err(self.err())
    }
    async fn inspect_container(&self, _: &str) -> DockerResult<Value> {
        Err(self.err())
    }
    async fn container_action(&self, _: &str, _: ContainerAction) -> DockerResult<()> {
        Err(self.err())
    }
    async fn remove_container(&self, _: &str, _: bool) -> DockerResult<()> {
        Err(self.err())
    }
    fn logs(&self, _: &str, _: LogQuery) -> BoxStream<'static, DockerResult<LogLine>> {
        futures_util::stream::iter([Err(self.err())]).boxed()
    }
    fn stats(&self, _: &str) -> BoxStream<'static, DockerResult<ContainerStats>> {
        futures_util::stream::iter([Err(self.err())]).boxed()
    }
    async fn images(&self) -> DockerResult<Vec<ImageItem>> {
        Err(self.err())
    }
    fn pull_image(
        &self,
        _: &str,
        _: Option<RegistryAuth>,
    ) -> BoxStream<'static, DockerResult<String>> {
        futures_util::stream::iter([Err(self.err())]).boxed()
    }
    async fn remove_image(&self, _: &str, _: bool) -> DockerResult<()> {
        Err(self.err())
    }
    async fn prune(&self, _: &str) -> DockerResult<PruneReport> {
        Err(self.err())
    }
    async fn volumes(&self) -> DockerResult<Vec<VolumeItem>> {
        Err(self.err())
    }
    async fn create_volume(&self, _: &str) -> DockerResult<VolumeItem> {
        Err(self.err())
    }
    async fn remove_volume(&self, _: &str) -> DockerResult<()> {
        Err(self.err())
    }
    async fn networks(&self) -> DockerResult<Vec<NetworkItem>> {
        Err(self.err())
    }
    async fn create_network(&self, _: &str, _: &str) -> DockerResult<NetworkItem> {
        Err(self.err())
    }
    async fn remove_network(&self, _: &str) -> DockerResult<()> {
        Err(self.err())
    }
    async fn ensure_network(&self, _: NetworkRequest) -> DockerResult<NetworkItem> {
        Err(self.err())
    }
    async fn inspect_image(&self, _: &str) -> DockerResult<ImageDetails> {
        Err(self.err())
    }
    async fn inspect_service(&self, _: &str) -> DockerResult<Option<SwarmService>> {
        Err(self.err())
    }
    async fn create_service(
        &self,
        _: ServiceSpec,
        _: Option<RegistryAuth>,
    ) -> DockerResult<String> {
        Err(self.err())
    }
    async fn update_service(
        &self,
        _: &str,
        _: u64,
        _: ServiceSpec,
        _: Option<RegistryAuth>,
    ) -> DockerResult<Vec<String>> {
        Err(self.err())
    }
    async fn delete_service(&self, _: &str) -> DockerResult<()> {
        Err(self.err())
    }
    async fn list_tasks(&self, _: &str) -> DockerResult<Vec<SwarmTask>> {
        Err(self.err())
    }
    fn build_image(
        &self,
        _: Vec<u8>,
        _: &str,
        _: &str,
        _: bool,
    ) -> BoxStream<'static, DockerResult<BuildLine>> {
        futures_util::stream::iter([Err(self.err())]).boxed()
    }
    async fn create_config(
        &self,
        _: &str,
        _: &[u8],
        _: HashMap<String, String>,
    ) -> DockerResult<String> {
        Err(self.err())
    }
    async fn list_configs(&self, _: &str, _: &str) -> DockerResult<Vec<(String, String)>> {
        Err(self.err())
    }
    async fn delete_config(&self, _: &str) -> DockerResult<()> {
        Err(self.err())
    }
    async fn create_secret(
        &self,
        _: &str,
        _: &[u8],
        _: HashMap<String, String>,
    ) -> DockerResult<String> {
        Err(self.err())
    }
    async fn list_secrets(&self, _: &str, _: &str) -> DockerResult<Vec<(String, String)>> {
        Err(self.err())
    }
    async fn delete_secret(&self, _: &str) -> DockerResult<()> {
        Err(self.err())
    }
    fn service_logs(&self, _: &str, _: LogQuery) -> BoxStream<'static, DockerResult<LogLine>> {
        futures_util::stream::iter([Err(self.err())]).boxed()
    }
    async fn exec(&self, _: &str, _: TermSize, _: &str) -> DockerResult<ExecSession> {
        Err(self.err())
    }
    async fn end_exec(&self, _: &str, _: &str) -> DockerResult<()> {
        Err(self.err())
    }
    async fn resize_exec(&self, _: &str, _: TermSize) -> DockerResult<()> {
        Err(self.err())
    }
    async fn run_in_container(
        &self,
        _: &str,
        _: Vec<String>,
        _: &str,
        _: Option<BoxStream<'static, Vec<u8>>>,
    ) -> DockerResult<BoxStream<'static, DockerResult<ExecChunk>>> {
        Err(self.err())
    }
    async fn kill_marked(&self, _: &str, _: &str) -> DockerResult<()> {
        Err(self.err())
    }
    async fn compose(
        &self,
        _: &std::path::Path,
        _: &str,
        _: &str,
        _: &[&str],
    ) -> DockerResult<(i32, String)> {
        Err(self.err())
    }
}

impl Unavailable {
    fn err(&self) -> DockerError {
        DockerError::Unavailable(self.0.clone())
    }
}

/// A container's state that is not running (nor paused or restarting).
fn is_stopped(state: &str) -> bool {
    matches!(state, "exited" | "created" | "dead")
}

/// [`Docker::system_cleanup`]'s report: old tasks' containers, and the
/// images they leave (tags, size).
#[derive(Debug, Default, Serialize)]
pub struct SystemCleanup {
    pub containers: usize,
    pub images: Vec<Value>,
    pub space: u64,
}

fn is_system(labels: &HashMap<String, String>) -> bool {
    labels.get(SYSTEM_LABEL).is_some_and(|v| v == "true")
}

fn visible<T>(
    items: Vec<T>,
    scope: Scope,
    labels: impl Fn(&T) -> &HashMap<String, String>,
) -> Vec<T> {
    match scope {
        Scope::System => items,
        Scope::User => items
            .into_iter()
            .filter(|item| !is_system(labels(item)))
            .collect(),
    }
}

fn guard(labels: &HashMap<String, String>, scope: Scope) -> DockerResult<()> {
    if scope == Scope::User && is_system(labels) {
        Err(DockerError::Protected)
    } else {
        Ok(())
    }
}

/// `Config.Labels` of an inspect document (empty when absent).
fn labels_of(doc: &Value) -> HashMap<String, String> {
    doc.pointer("/Config/Labels")
        .and_then(Value::as_object)
        .map(|labels| {
            labels
                .iter()
                .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned())))
                .collect()
        })
        .unwrap_or_default()
}

/// Replaces secrets in `Config.Env`: the whole value when the name looks
/// secret, else the password of a URL's userinfo
/// (`DATABASE_URL=mysql://user:******@host/db`).
fn redact_env(doc: &mut Value) {
    let Some(Value::Array(env)) = doc.pointer_mut("/Config/Env") else {
        return;
    };
    for entry in env {
        let Some(text) = entry.as_str() else { continue };
        let Some((name, value)) = text.split_once('=') else {
            continue;
        };
        let upper = name.to_ascii_uppercase();
        let redacted = if SECRET_MARKERS.iter().any(|marker| upper.contains(marker)) {
            Some(REDACTED.to_owned())
        } else {
            redact_userinfo(value)
        };
        if let Some(value) = redacted {
            *entry = Value::String(format!("{name}={value}"));
        }
    }
}

/// `scheme://user:pass@rest` → `scheme://user:******@rest`; `None` when the
/// value carries no password.
fn redact_userinfo(value: &str) -> Option<String> {
    let (scheme, rest) = value.split_once("://")?;
    let (userinfo, host) = rest.split_once('@')?;
    if userinfo.contains('/') {
        return None;
    }
    let user = userinfo.split_once(':').map_or(userinfo, |(user, _)| user);
    Some(format!("{scheme}://{user}:{REDACTED}@{host}"))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn redacts_secret_looking_env() {
        let mut doc = json!({"Config": {"Env": [
            "PATH=/bin", "DB_PASSWORD=hunter2", "api_key=x", "NOEQUALS",
            "DATABASE_URL=mysql://panel:hunter2@db:3306/panel", "REDIS_URL=redis://:pw@cache",
            "HOME_URL=https://x.io/a@b"
        ]}});
        redact_env(&mut doc);
        assert_eq!(
            doc["Config"]["Env"],
            json!([
                "PATH=/bin",
                "DB_PASSWORD=******",
                "api_key=******",
                "NOEQUALS",
                "DATABASE_URL=mysql://panel:******@db:3306/panel",
                "REDIS_URL=redis://:******@cache",
                "HOME_URL=https://x.io/a@b"
            ])
        );
    }

    #[test]
    fn only_docker_identifiers_pass() {
        for ok in ["web", "a1b2c3", "my_app.v2-1"] {
            assert!(checked_name(ok).is_ok(), "{ok}");
        }
        for bad in ["", "../containers/panel", "a/b", ".hidden", "a%2Fb", "a b"] {
            assert!(checked_name(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn system_objects_are_guarded_for_users_only() {
        let system = HashMap::from([(SYSTEM_LABEL.to_owned(), "true".to_owned())]);
        assert!(matches!(
            guard(&system, Scope::User),
            Err(DockerError::Protected)
        ));
        assert!(guard(&system, Scope::System).is_ok());
        assert!(guard(&HashMap::new(), Scope::User).is_ok());
        let not_true = HashMap::from([(SYSTEM_LABEL.to_owned(), "false".to_owned())]);
        assert!(guard(&not_true, Scope::User).is_ok());
    }

    #[test]
    fn reads_labels_of_inspect_documents() {
        let doc = json!({"Config": {"Labels": {SYSTEM_LABEL: "true", "n": 1}}});
        assert_eq!(
            labels_of(&doc),
            HashMap::from([(SYSTEM_LABEL.to_owned(), "true".to_owned())])
        );
        assert!(labels_of(&json!({})).is_empty());
    }
}
