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
pub const SYSTEM_LABEL: &str = "rust-panel.system";

/// Env names containing one of these have their value replaced in `inspect`.
const SECRET_MARKERS: [&str; 5] = ["PASSWORD", "SECRET", "TOKEN", "KEY", "CREDENTIAL"];
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

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct PruneReport {
    pub containers_deleted: usize,
    pub images_deleted: usize,
    pub space_reclaimed: u64,
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
    /// Progress lines while pulling `image` (`name[:tag]`).
    fn pull_image(&self, image: &str) -> BoxStream<'static, DockerResult<String>>;
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

    /// The inspect document with secret-looking env values replaced.
    pub async fn inspect_container(&self, id: &str, scope: Scope) -> DockerResult<Value> {
        let mut doc = self.0.inspect_container(id).await?;
        guard(&labels_of(&doc), scope)?;
        redact_env(&mut doc);
        Ok(doc)
    }

    pub async fn container_action(
        &self,
        id: &str,
        action: ContainerAction,
        scope: Scope,
    ) -> DockerResult<()> {
        self.guard_container(id, scope).await?;
        self.0.container_action(id, action).await
    }

    pub async fn remove_container(&self, id: &str, force: bool, scope: Scope) -> DockerResult<()> {
        self.guard_container(id, scope).await?;
        self.0.remove_container(id, force).await
    }

    pub async fn logs(
        &self,
        id: &str,
        query: LogQuery,
        scope: Scope,
    ) -> DockerResult<BoxStream<'static, DockerResult<LogLine>>> {
        self.guard_container(id, scope).await?;
        Ok(self.0.logs(id, query))
    }

    pub async fn stats(
        &self,
        id: &str,
        scope: Scope,
    ) -> DockerResult<BoxStream<'static, DockerResult<ContainerStats>>> {
        self.guard_container(id, scope).await?;
        Ok(self.0.stats(id))
    }

    pub async fn images(&self) -> DockerResult<Vec<ImageItem>> {
        self.0.images().await
    }

    pub fn pull_image(&self, image: &str) -> BoxStream<'static, DockerResult<String>> {
        self.0.pull_image(image)
    }

    /// Images carry no system label; an image a system container runs is
    /// protected by Docker itself (in use) unless `force`, which only the
    /// system scope may use.
    pub async fn remove_image(&self, id: &str, force: bool, scope: Scope) -> DockerResult<()> {
        if force && scope != Scope::System {
            return Err(DockerError::Protected);
        }
        self.0.remove_image(id, force).await
    }

    pub async fn prune(&self) -> DockerResult<PruneReport> {
        self.0.prune(SYSTEM_LABEL).await
    }

    pub async fn volumes(&self, scope: Scope) -> DockerResult<Vec<VolumeItem>> {
        Ok(visible(self.0.volumes().await?, scope, |v| &v.labels))
    }

    pub async fn create_volume(&self, name: &str) -> DockerResult<VolumeItem> {
        self.0.create_volume(name).await
    }

    pub async fn remove_volume(&self, name: &str, scope: Scope) -> DockerResult<()> {
        let volume = self
            .0
            .volumes()
            .await?
            .into_iter()
            .find(|v| v.name == name)
            .ok_or(DockerError::NotFound)?;
        guard(&volume.labels, scope)?;
        self.0.remove_volume(name).await
    }

    pub async fn networks(&self, scope: Scope) -> DockerResult<Vec<NetworkItem>> {
        Ok(visible(self.0.networks().await?, scope, |n| &n.labels))
    }

    pub async fn create_network(&self, name: &str, driver: &str) -> DockerResult<NetworkItem> {
        self.0.create_network(name, driver).await
    }

    pub async fn remove_network(&self, id: &str, scope: Scope) -> DockerResult<()> {
        let network = self
            .0
            .networks()
            .await?
            .into_iter()
            .find(|n| n.id == id || n.name == id)
            .ok_or(DockerError::NotFound)?;
        guard(&network.labels, scope)?;
        self.0.remove_network(id).await
    }

    async fn guard_container(&self, id: &str, scope: Scope) -> DockerResult<()> {
        if scope == Scope::System {
            return Ok(());
        }
        let doc = self.0.inspect_container(id).await?;
        guard(&labels_of(&doc), scope)
    }
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
    fn pull_image(&self, _: &str) -> BoxStream<'static, DockerResult<String>> {
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
}

impl Unavailable {
    fn err(&self) -> DockerError {
        DockerError::Unavailable(self.0.clone())
    }
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

/// Replaces the value of every `Config.Env` entry whose name looks secret.
fn redact_env(doc: &mut Value) {
    let Some(Value::Array(env)) = doc.pointer_mut("/Config/Env") else {
        return;
    };
    for entry in env {
        let Some(text) = entry.as_str() else { continue };
        let Some((name, _)) = text.split_once('=') else {
            continue;
        };
        let upper = name.to_ascii_uppercase();
        if SECRET_MARKERS.iter().any(|marker| upper.contains(marker)) {
            *entry = Value::String(format!("{name}={REDACTED}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn redacts_secret_looking_env() {
        let mut doc = json!({"Config": {"Env": ["PATH=/bin", "DB_PASSWORD=hunter2", "api_key=x", "NOEQUALS"]}});
        redact_env(&mut doc);
        assert_eq!(
            doc["Config"]["Env"],
            json!([
                "PATH=/bin",
                "DB_PASSWORD=******",
                "api_key=******",
                "NOEQUALS"
            ])
        );
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
