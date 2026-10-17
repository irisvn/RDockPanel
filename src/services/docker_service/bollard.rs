//! [`DockerApi`] over the Docker Engine API (Bollard, local socket or
//! `DOCKER_HOST`).

use std::collections::HashMap;

use ::bollard::Docker as Client;
use ::bollard::container::LogOutput;
use ::bollard::errors::Error;
use ::bollard::models::{
    ContainerStatsResponse, ContainerSummary, LocalNodeState, NetworkCreateRequest, PortSummary,
    VolumeCreateRequest,
};
use ::bollard::query_parameters::{
    CreateImageOptionsBuilder, InspectContainerOptions, KillContainerOptions,
    ListContainersOptionsBuilder, ListImagesOptions, ListNetworksOptions, ListVolumesOptions,
    LogsOptionsBuilder, PruneContainersOptionsBuilder, PruneImagesOptionsBuilder,
    RemoveContainerOptionsBuilder, RemoveImageOptionsBuilder, RemoveVolumeOptions,
    RestartContainerOptions, StartContainerOptions, StatsOptionsBuilder, StopContainerOptions,
};
use futures_util::stream::{BoxStream, StreamExt};
use rustcms::async_trait;
use serde_json::Value;

use super::{
    ContainerAction, ContainerItem, ContainerStats, DockerApi, DockerError, DockerInfo,
    DockerResult, ImageItem, LogLine, LogQuery, LogStream, NetworkItem, PruneReport, SwarmRole,
    VolumeItem,
};

pub struct BollardDocker {
    client: Client,
}

impl BollardDocker {
    /// Connects lazily: nothing is sent until the first call, so a missing
    /// daemon shows as [`DockerError::Unavailable`] on use, not at boot.
    pub fn connect() -> DockerResult<Self> {
        let client = Client::connect_with_local_defaults()
            .map_err(|e| DockerError::Unavailable(e.to_string()))?;
        Ok(Self { client })
    }
}

fn map_err(err: Error) -> DockerError {
    match err {
        Error::DockerResponseServerError {
            status_code: 404, ..
        } => DockerError::NotFound,
        Error::DockerResponseServerError {
            status_code: 304 | 409,
            message,
        } => DockerError::Conflict(message),
        Error::DockerResponseServerError { message, .. } => DockerError::Other(message),
        Error::IOError { err } => DockerError::Unavailable(err.to_string()),
        Error::HyperResponseError { err } => DockerError::Unavailable(err.to_string()),
        other => DockerError::Other(other.to_string()),
    }
}

fn container_item(c: ContainerSummary) -> ContainerItem {
    let name = c
        .names
        .unwrap_or_default()
        .into_iter()
        .next()
        .unwrap_or_default();
    ContainerItem {
        id: c.id.unwrap_or_default(),
        name: name.trim_start_matches('/').to_owned(),
        image: c.image.unwrap_or_default(),
        state: c.state.map(|s| s.to_string()).unwrap_or_default(),
        status: c.status.unwrap_or_default(),
        created: c.created.unwrap_or_default(),
        ports: c.ports.unwrap_or_default().iter().map(port).collect(),
        labels: c.labels.unwrap_or_default(),
    }
}

/// `0.0.0.0:8080->80/tcp`, or `80/tcp` when not published.
fn port(p: &PortSummary) -> String {
    let proto = p
        .typ
        .map(|t| t.to_string())
        .unwrap_or_else(|| "tcp".to_owned());
    match p.public_port {
        Some(public) => format!(
            "{}:{public}->{}/{proto}",
            p.ip.as_deref().unwrap_or("0.0.0.0"),
            p.private_port
        ),
        None => format!("{}/{proto}", p.private_port),
    }
}

/// CPU as `docker stats` shows it (100% = one core) and memory without the
/// page cache.
pub(super) fn container_stats(s: &ContainerStatsResponse) -> ContainerStats {
    let total = |cpu: &Option<::bollard::models::ContainerCpuStats>| {
        cpu.as_ref()
            .and_then(|c| c.cpu_usage.as_ref())
            .and_then(|u| u.total_usage)
            .unwrap_or(0)
    };
    let system = |cpu: &Option<::bollard::models::ContainerCpuStats>| {
        cpu.as_ref().and_then(|c| c.system_cpu_usage).unwrap_or(0)
    };
    let cpu_delta = total(&s.cpu_stats).saturating_sub(total(&s.precpu_stats)) as f64;
    let system_delta = system(&s.cpu_stats).saturating_sub(system(&s.precpu_stats)) as f64;
    let cpus = s
        .cpu_stats
        .as_ref()
        .and_then(|c| c.online_cpus)
        .unwrap_or(1)
        .max(1) as f64;
    let cpu_percent = if system_delta > 0.0 {
        cpu_delta / system_delta * cpus * 100.0
    } else {
        0.0
    };

    let memory = s.memory_stats.as_ref();
    let usage = memory.and_then(|m| m.usage).unwrap_or(0);
    let cache = memory
        .and_then(|m| m.stats.as_ref())
        .and_then(|st| {
            st.get("inactive_file")
                .or_else(|| st.get("total_inactive_file"))
                .copied()
        })
        .unwrap_or(0);

    let (net_rx, net_tx) = s
        .networks
        .iter()
        .flat_map(HashMap::values)
        .fold((0, 0), |(rx, tx), n| {
            (rx + n.rx_bytes.unwrap_or(0), tx + n.tx_bytes.unwrap_or(0))
        });
    let (blk_read, blk_write) = s
        .blkio_stats
        .as_ref()
        .and_then(|b| b.io_service_bytes_recursive.as_ref())
        .into_iter()
        .flatten()
        .fold((0, 0), |(r, w), e| {
            match e.op.as_deref().map(str::to_ascii_lowercase).as_deref() {
                Some("read") => (r + e.value.unwrap_or(0), w),
                Some("write") => (r, w + e.value.unwrap_or(0)),
                _ => (r, w),
            }
        });

    ContainerStats {
        cpu_percent,
        mem_used: usage.saturating_sub(cache),
        mem_limit: memory.and_then(|m| m.limit).unwrap_or(0),
        net_rx,
        net_tx,
        blk_read,
        blk_write,
    }
}

#[async_trait]
impl DockerApi for BollardDocker {
    async fn info(&self) -> DockerResult<DockerInfo> {
        let info = self.client.info().await.map_err(map_err)?;
        let swarm = match info.swarm.as_ref() {
            Some(s) if s.local_node_state == Some(LocalNodeState::ACTIVE) => {
                if s.control_available == Some(true) {
                    SwarmRole::Manager
                } else {
                    SwarmRole::Worker
                }
            }
            _ => SwarmRole::Inactive,
        };
        Ok(DockerInfo {
            name: info.name.unwrap_or_default(),
            server_version: info.server_version.unwrap_or_default(),
            operating_system: info.operating_system.unwrap_or_default(),
            kernel_version: info.kernel_version.unwrap_or_default(),
            architecture: info.architecture.unwrap_or_default(),
            cpus: info.ncpu.unwrap_or_default(),
            mem_total: info.mem_total.unwrap_or_default(),
            containers: info.containers.unwrap_or_default(),
            containers_running: info.containers_running.unwrap_or_default(),
            images: info.images.unwrap_or_default(),
            swarm,
        })
    }

    async fn containers(&self) -> DockerResult<Vec<ContainerItem>> {
        let options = ListContainersOptionsBuilder::new().all(true).build();
        let list = self
            .client
            .list_containers(Some(options))
            .await
            .map_err(map_err)?;
        Ok(list.into_iter().map(container_item).collect())
    }

    async fn inspect_container(&self, id: &str) -> DockerResult<Value> {
        let doc = self
            .client
            .inspect_container(id, None::<InspectContainerOptions>)
            .await
            .map_err(map_err)?;
        serde_json::to_value(doc).map_err(|e| DockerError::Other(e.to_string()))
    }

    async fn container_action(&self, id: &str, action: ContainerAction) -> DockerResult<()> {
        let c = &self.client;
        match action {
            ContainerAction::Start => c.start_container(id, None::<StartContainerOptions>).await,
            ContainerAction::Stop => c.stop_container(id, None::<StopContainerOptions>).await,
            ContainerAction::Restart => {
                c.restart_container(id, None::<RestartContainerOptions>)
                    .await
            }
            ContainerAction::Kill => c.kill_container(id, None::<KillContainerOptions>).await,
            ContainerAction::Pause => c.pause_container(id).await,
            ContainerAction::Unpause => c.unpause_container(id).await,
        }
        .map_err(map_err)
    }

    async fn remove_container(&self, id: &str, force: bool) -> DockerResult<()> {
        let options = RemoveContainerOptionsBuilder::new().force(force).build();
        self.client
            .remove_container(id, Some(options))
            .await
            .map_err(map_err)
    }

    fn logs(&self, id: &str, query: LogQuery) -> BoxStream<'static, DockerResult<LogLine>> {
        let mut options = LogsOptionsBuilder::new()
            .follow(query.follow)
            .stdout(true)
            .stderr(true)
            .timestamps(query.timestamps)
            .tail(
                &query
                    .tail
                    .map_or_else(|| "all".to_owned(), |n| n.to_string()),
            );
        if let Some(since) = query.since {
            options = options.since(i32::try_from(since).unwrap_or(i32::MAX));
        }
        self.client
            .logs(id, Some(options.build()))
            .map(|item| {
                let (stream, message) = match item.map_err(map_err)? {
                    LogOutput::StdErr { message } => (LogStream::Stderr, message),
                    LogOutput::StdOut { message }
                    | LogOutput::Console { message }
                    | LogOutput::StdIn { message } => (LogStream::Stdout, message),
                };
                Ok(LogLine {
                    stream,
                    text: String::from_utf8_lossy(&message).into_owned(),
                })
            })
            .boxed()
    }

    fn stats(&self, id: &str) -> BoxStream<'static, DockerResult<ContainerStats>> {
        let options = StatsOptionsBuilder::new().stream(true).build();
        self.client
            .stats(id, Some(options))
            .map(|item| Ok(container_stats(&item.map_err(map_err)?)))
            .boxed()
    }

    async fn images(&self) -> DockerResult<Vec<ImageItem>> {
        let list = self
            .client
            .list_images(None::<ListImagesOptions>)
            .await
            .map_err(map_err)?;
        Ok(list
            .into_iter()
            .map(|i| ImageItem {
                id: i.id,
                tags: i.repo_tags,
                size: i.size,
                created: i.created,
                containers: i.containers,
                labels: i.labels,
            })
            .collect())
    }

    fn pull_image(&self, image: &str) -> BoxStream<'static, DockerResult<String>> {
        let (name, tag) = split_reference(image);
        let options = CreateImageOptionsBuilder::new()
            .from_image(name)
            .tag(tag)
            .build();
        self.client
            .create_image(Some(options), None, None)
            .map(|item| {
                let info = item.map_err(map_err)?;
                if let Some(detail) = info.error_detail {
                    return Err(DockerError::Other(detail.message.unwrap_or_default()));
                }
                let status = info.status.unwrap_or_default();
                Ok(match info.id {
                    Some(layer) => format!("{layer}: {status}"),
                    None => status,
                })
            })
            .boxed()
    }

    async fn remove_image(&self, id: &str, force: bool) -> DockerResult<()> {
        let options = RemoveImageOptionsBuilder::new().force(force).build();
        self.client
            .remove_image(id, Some(options), None)
            .await
            .map(drop)
            .map_err(map_err)
    }

    async fn prune(&self, exclude_label: &str) -> DockerResult<PruneReport> {
        let filters = HashMap::from([("label!", vec![format!("{exclude_label}=true")])]);
        let containers = self
            .client
            .prune_containers(Some(
                PruneContainersOptionsBuilder::new()
                    .filters(&filters)
                    .build(),
            ))
            .await
            .map_err(map_err)?;
        let mut image_filters = filters.clone();
        image_filters.insert("dangling", vec!["true".to_owned()]);
        let images = self
            .client
            .prune_images(Some(
                PruneImagesOptionsBuilder::new()
                    .filters(&image_filters)
                    .build(),
            ))
            .await
            .map_err(map_err)?;
        let reclaimed = |n: Option<i64>| u64::try_from(n.unwrap_or(0)).unwrap_or(0);
        Ok(PruneReport {
            containers_deleted: containers.containers_deleted.map_or(0, |v| v.len()),
            images_deleted: images.images_deleted.map_or(0, |v| v.len()),
            space_reclaimed: reclaimed(containers.space_reclaimed)
                + reclaimed(images.space_reclaimed),
        })
    }

    async fn volumes(&self) -> DockerResult<Vec<VolumeItem>> {
        let list = self
            .client
            .list_volumes(None::<ListVolumesOptions>)
            .await
            .map_err(map_err)?;
        Ok(list
            .volumes
            .unwrap_or_default()
            .into_iter()
            .map(volume_item)
            .collect())
    }

    async fn create_volume(&self, name: &str) -> DockerResult<VolumeItem> {
        let request = VolumeCreateRequest {
            name: Some(name.to_owned()),
            ..Default::default()
        };
        self.client
            .create_volume(request)
            .await
            .map(volume_item)
            .map_err(map_err)
    }

    async fn remove_volume(&self, name: &str) -> DockerResult<()> {
        self.client
            .remove_volume(name, None::<RemoveVolumeOptions>)
            .await
            .map_err(map_err)
    }

    async fn networks(&self) -> DockerResult<Vec<NetworkItem>> {
        let list = self
            .client
            .list_networks(None::<ListNetworksOptions>)
            .await
            .map_err(map_err)?;
        Ok(list
            .into_iter()
            .map(|n| NetworkItem {
                id: n.id.unwrap_or_default(),
                name: n.name.unwrap_or_default(),
                driver: n.driver.unwrap_or_default(),
                scope: n.scope.unwrap_or_default(),
                labels: n.labels.unwrap_or_default(),
            })
            .collect())
    }

    async fn create_network(&self, name: &str, driver: &str) -> DockerResult<NetworkItem> {
        let request = NetworkCreateRequest {
            name: name.to_owned(),
            driver: Some(driver.to_owned()),
            attachable: Some(driver == "overlay"),
            ..Default::default()
        };
        let created = self.client.create_network(request).await.map_err(map_err)?;
        Ok(NetworkItem {
            id: created.id,
            name: name.to_owned(),
            driver: driver.to_owned(),
            scope: if driver == "overlay" {
                "swarm"
            } else {
                "local"
            }
            .to_owned(),
            labels: HashMap::new(),
        })
    }

    async fn remove_network(&self, id: &str) -> DockerResult<()> {
        self.client.remove_network(id).await.map_err(map_err)
    }
}

fn volume_item(v: ::bollard::models::Volume) -> VolumeItem {
    VolumeItem {
        name: v.name,
        driver: v.driver,
        mountpoint: v.mountpoint,
        labels: v.labels,
    }
}

/// `name[:tag]` → (name, tag); a `:` inside a registry host (`host:5000/app`)
/// is not a tag, and digests (`@sha256:…`) are kept whole.
fn split_reference(image: &str) -> (&str, &str) {
    if image.contains('@') {
        return (image, "");
    }
    match image.rsplit_once(':') {
        Some((name, tag)) if !tag.contains('/') => (name, tag),
        _ => (image, "latest"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_image_references() {
        assert_eq!(split_reference("nginx"), ("nginx", "latest"));
        assert_eq!(split_reference("nginx:1.27"), ("nginx", "1.27"));
        assert_eq!(
            split_reference("host:5000/app"),
            ("host:5000/app", "latest")
        );
        assert_eq!(split_reference("host:5000/app:v2"), ("host:5000/app", "v2"));
        assert_eq!(split_reference("app@sha256:abc"), ("app@sha256:abc", ""));
    }

    #[test]
    fn computes_cpu_like_docker_stats() {
        let stats: ContainerStatsResponse = serde_json::from_value(serde_json::json!({
            "cpu_stats": {"cpu_usage": {"total_usage": 400}, "system_cpu_usage": 2000, "online_cpus": 2},
            "precpu_stats": {"cpu_usage": {"total_usage": 200}, "system_cpu_usage": 1000},
            "memory_stats": {"usage": 1000, "limit": 4000, "stats": {"inactive_file": 200}},
            "networks": {"eth0": {"rx_bytes": 10, "tx_bytes": 20}, "eth1": {"rx_bytes": 1, "tx_bytes": 2}},
            "blkio_stats": {"io_service_bytes_recursive": [
                {"op": "read", "value": 5}, {"op": "Write", "value": 7}, {"op": "total", "value": 12}
            ]}
        }))
        .unwrap();
        let s = container_stats(&stats);
        assert!((s.cpu_percent - 40.0).abs() < 1e-9);
        assert_eq!((s.mem_used, s.mem_limit), (800, 4000));
        assert_eq!(
            (s.net_rx, s.net_tx, s.blk_read, s.blk_write),
            (11, 22, 5, 7)
        );
    }
}
