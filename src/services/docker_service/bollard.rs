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

use ::bollard::exec::{CreateExecOptions, StartExecOptions, StartExecResults};
use ::bollard::query_parameters::ResizeExecOptionsBuilder;

use super::{
    BuildLine, ContainerAction, ContainerItem, ContainerStats, DockerApi, DockerError, DockerInfo,
    DockerResult, ExecChunk, ExecSession, ImageDetails, ImageItem, LogLine, LogQuery, LogStream,
    NetworkItem, NetworkRequest, PruneReport, RegistryAuth, SESSION_ENV, ServiceSpec, SwarmRole,
    SwarmService, SwarmTask, TermSize, VolumeItem,
};

/// bash when the image has it, else sh.
const SHELL: [&str; 3] = [
    "/bin/sh",
    "-c",
    "if command -v bash >/dev/null 2>&1; then exec bash; else exec sh; fi",
];

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
        // Swarm bumps a service's version on its own (endpoints, tasks): an
        // update at a version read earlier is a conflict to retry, not a failure.
        Error::DockerResponseServerError { message, .. } if message.contains("out of sequence") => {
            DockerError::Conflict(message)
        }
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
        image_id: c.image_id.unwrap_or_default(),
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

    fn pull_image(
        &self,
        image: &str,
        auth: Option<RegistryAuth>,
    ) -> BoxStream<'static, DockerResult<String>> {
        let (name, tag) = split_reference(image);
        let options = CreateImageOptionsBuilder::new()
            .from_image(name)
            .tag(tag)
            .build();
        self.client
            .create_image(Some(options), None, auth.map(credentials))
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

    async fn exec(
        &self,
        container: &str,
        size: TermSize,
        token: &str,
    ) -> DockerResult<ExecSession> {
        let config = CreateExecOptions {
            attach_stdin: Some(true),
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            tty: Some(true),
            env: Some(vec![
                "TERM=xterm-256color".to_owned(),
                format!("{SESSION_ENV}={token}"),
            ]),
            cmd: Some(SHELL.iter().map(|part| (*part).to_owned()).collect()),
            ..Default::default()
        };
        let id = self
            .client
            .create_exec(container, config)
            .await
            .map_err(map_err)?
            .id;
        let start = StartExecOptions {
            detach: false,
            tty: true,
            output_capacity: None,
        };
        let StartExecResults::Attached { output, input } = self
            .client
            .start_exec(&id, Some(start))
            .await
            .map_err(map_err)?
        else {
            return Err(DockerError::Other("the shell started detached".to_owned()));
        };
        // Sized once it runs (Docker refuses to resize an exec not started).
        let _ = self.resize_exec(&id, size).await;
        let output = output
            .map(|chunk| chunk.map(|out| out.into_bytes().to_vec()).map_err(map_err))
            .boxed();
        Ok(ExecSession { id, output, input })
    }

    async fn run_in_container(
        &self,
        container: &str,
        cmd: Vec<String>,
        input: Option<BoxStream<'static, Vec<u8>>>,
    ) -> DockerResult<BoxStream<'static, DockerResult<ExecChunk>>> {
        // Its processes carry a mark: killed if the caller goes away.
        let token = crate::services::database_service::random_string(24);
        let config = CreateExecOptions {
            attach_stdin: Some(input.is_some()),
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            tty: Some(false),
            env: Some(vec![format!("{SESSION_ENV}={token}")]),
            cmd: Some(cmd),
            ..Default::default()
        };
        let id = self
            .client
            .create_exec(container, config)
            .await
            .map_err(map_err)?
            .id;
        let start = StartExecOptions {
            detach: false,
            tty: false,
            output_capacity: None,
        };
        let StartExecResults::Attached {
            mut output,
            input: mut stdin,
        } = self
            .client
            .start_exec(&id, Some(start))
            .await
            .map_err(map_err)?
        else {
            return Err(DockerError::Other(
                "the command started detached".to_owned(),
            ));
        };
        let (tx, rx) = tokio::sync::mpsc::channel(16);
        let client = self.client.clone();
        let container = container.to_owned();
        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            // Fed while its output is read (a full pipe would block both).
            let feed = async move {
                if let Some(mut input) = input {
                    while let Some(chunk) = input.next().await {
                        if stdin.write_all(&chunk).await.is_err() {
                            return;
                        }
                    }
                }
                let _ = stdin.shutdown().await;
            };
            // Whether the output was read to its end (else the caller left).
            let read = async {
                loop {
                    let item = tokio::select! {
                        item = output.next() => item,
                        () = tx.closed() => return false,
                    };
                    let chunk = match item {
                        None => return true,
                        Some(Ok(LogOutput::StdOut { message })) => {
                            Ok(ExecChunk::Out(message.to_vec()))
                        }
                        Some(Ok(LogOutput::StdErr { message })) => {
                            Ok(ExecChunk::Err(message.to_vec()))
                        }
                        Some(Ok(_)) => continue,
                        Some(Err(e)) => Err(map_err(e)),
                    };
                    let failed = chunk.is_err();
                    if tx.send(chunk).await.is_err() || failed {
                        return false;
                    }
                }
            };
            tokio::pin!(feed);
            tokio::pin!(read);
            let mut fed = false;
            let finished = loop {
                tokio::select! {
                    () = &mut feed, if !fed => fed = true,
                    finished = &mut read => break finished,
                }
            };
            if finished {
                let code = client
                    .inspect_exec(&id)
                    .await
                    .map_err(map_err)
                    .map(|e| e.exit_code.unwrap_or(-1));
                let _ = tx.send(code.map(ExecChunk::Exit)).await;
            } else {
                // Never left running (a restore half fed): its processes killed.
                let _ = kill_marked(&client, &container, &token).await;
            }
        });
        Ok(futures_util::stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|chunk| (chunk, rx))
        })
        .boxed())
    }
    async fn ensure_network(&self, request: NetworkRequest) -> DockerResult<NetworkItem> {
        // By its exact name, inspected (lists may leave out the labels).
        match self
            .client
            .inspect_network(
                &request.name,
                None::<::bollard::query_parameters::InspectNetworkOptions>,
            )
            .await
        {
            Ok(found) if found.name.as_deref() == Some(request.name.as_str()) => {
                return Ok(NetworkItem {
                    id: found.id.unwrap_or_default(),
                    name: request.name,
                    driver: found.driver.unwrap_or_default(),
                    scope: found.scope.unwrap_or_default(),
                    labels: found.labels.unwrap_or_default(),
                });
            }
            Ok(_)
            | Err(Error::DockerResponseServerError {
                status_code: 404, ..
            }) => {}
            Err(err) => return Err(map_err(err)),
        }
        let create = NetworkCreateRequest {
            name: request.name.clone(),
            driver: Some(request.driver.clone()),
            attachable: Some(request.attachable),
            internal: Some(request.internal),
            labels: Some(request.labels.clone()),
            ..Default::default()
        };
        let created = self.client.create_network(create).await.map_err(map_err)?;
        Ok(NetworkItem {
            id: created.id,
            name: request.name,
            driver: request.driver.clone(),
            scope: if request.driver == "overlay" {
                "swarm"
            } else {
                "local"
            }
            .to_owned(),
            labels: request.labels,
        })
    }

    async fn inspect_image(&self, reference: &str) -> DockerResult<ImageDetails> {
        let image = self
            .client
            .inspect_image(reference)
            .await
            .map_err(map_err)?;
        Ok(ImageDetails {
            id: image.id.unwrap_or_default(),
            repo_digests: image.repo_digests.unwrap_or_default(),
        })
    }

    async fn inspect_service(&self, name: &str) -> DockerResult<Option<SwarmService>> {
        match self
            .client
            .inspect_service(
                name,
                None::<::bollard::query_parameters::InspectServiceOptions>,
            )
            .await
        {
            Ok(service) => Ok(Some(service)),
            Err(Error::DockerResponseServerError {
                status_code: 404, ..
            }) => Ok(None),
            Err(err) => Err(map_err(err)),
        }
    }

    async fn create_service(
        &self,
        spec: ServiceSpec,
        auth: Option<RegistryAuth>,
    ) -> DockerResult<String> {
        let created = self
            .client
            .create_service(spec, auth.map(credentials))
            .await
            .map_err(map_err)?;
        Ok(created.id.unwrap_or_default())
    }

    async fn update_service(
        &self,
        name: &str,
        version: u64,
        spec: ServiceSpec,
        auth: Option<RegistryAuth>,
    ) -> DockerResult<Vec<String>> {
        let version = i32::try_from(version)
            .map_err(|_| DockerError::Other("service version out of range".to_owned()))?;
        let mut options =
            ::bollard::query_parameters::UpdateServiceOptionsBuilder::new().version(version);
        if auth.is_none() {
            // Keep the credentials of the current spec ("previous-spec"
            // fails on a service updated for the first time).
            options = options.registry_auth_from("spec");
        }
        let updated = self
            .client
            .update_service(name, spec, options.build(), auth.map(credentials))
            .await
            .map_err(map_err)?;
        Ok(updated.warnings.unwrap_or_default())
    }

    async fn delete_service(&self, name: &str) -> DockerResult<()> {
        self.client.delete_service(name).await.map_err(map_err)
    }

    fn build_image(
        &self,
        context: Vec<u8>,
        dockerfile: &str,
        tag: &str,
    ) -> BoxStream<'static, DockerResult<BuildLine>> {
        let options = ::bollard::query_parameters::BuildImageOptionsBuilder::new()
            .dockerfile(dockerfile)
            .t(tag)
            .rm(true)
            .forcerm(true)
            .pull("true")
            // BuildKit (what `docker build` uses): `RUN --mount`, as
            // Nixpacks writes, needs it; its session sends the context.
            .version(::bollard::query_parameters::BuilderVersion::BuilderBuildKit)
            .session(&format!(
                "rdp-{}",
                crate::services::database_service::random_string(20)
            ))
            .build();
        build_stream(self.client.clone(), options, context)
    }

    async fn create_config(
        &self,
        name: &str,
        data: &[u8],
        labels: HashMap<String, String>,
    ) -> DockerResult<String> {
        use base64::Engine;
        let spec = ::bollard::models::ConfigSpec {
            name: Some(name.to_owned()),
            labels: Some(labels),
            data: Some(base64::engine::general_purpose::STANDARD.encode(data)),
            ..Default::default()
        };
        let created = self.client.create_config(spec).await.map_err(map_err)?;
        Ok(created.id)
    }

    async fn list_configs(&self, key: &str, value: &str) -> DockerResult<Vec<(String, String)>> {
        let label = format!("{key}={value}");
        let filters = HashMap::from([("label", vec![label.as_str()])]);
        let options = ::bollard::query_parameters::ListConfigsOptionsBuilder::new()
            .filters(&filters)
            .build();
        let configs = self
            .client
            .list_configs(Some(options))
            .await
            .map_err(map_err)?;
        Ok(configs
            .into_iter()
            .filter_map(|c| Some((c.id?, c.spec?.name?)))
            .collect())
    }

    async fn delete_config(&self, id: &str) -> DockerResult<()> {
        self.client.delete_config(id).await.map_err(map_err)
    }

    async fn list_tasks(&self, service: &str) -> DockerResult<Vec<SwarmTask>> {
        let filters = HashMap::from([("service", vec![service.to_owned()])]);
        let options = ::bollard::query_parameters::ListTasksOptionsBuilder::new()
            .filters(&filters)
            .build();
        self.client.list_tasks(Some(options)).await.map_err(map_err)
    }

    fn service_logs(
        &self,
        service: &str,
        query: LogQuery,
    ) -> BoxStream<'static, DockerResult<LogLine>> {
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
            .service_logs(service, Some(options.build()))
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

    async fn end_exec(&self, container: &str, token: &str) -> DockerResult<()> {
        // The sessions (TTY sessions: the shell and everything it started,
        // even with a cleared environment) of the processes carrying the
        // mark: hung up, then killed 2 s later.
        let script = format!(
            r#"m='{SESSION_ENV}={token}'; sids=''
for p in /proc/[0-9]*; do
  tr '\0' '\n' < "$p/environ" 2>/dev/null | grep -qx "$m" || continue
  s=$(cat "$p/stat" 2>/dev/null) || continue; set -- ${{s##*) }}; sids="$sids $4"
done
[ -n "$sids" ] || exit 0
end() {{
  sig=$1
  for p in /proc/[0-9]*; do
    s=$(cat "$p/stat" 2>/dev/null) || continue; set -- ${{s##*) }}
    for sid in $sids; do [ "$4" = "$sid" ] && kill -$sig "${{p#/proc/}}" 2>/dev/null; done
  done
}}
end HUP; sleep 2; end KILL; true"#
        );
        let config = CreateExecOptions {
            cmd: Some(vec!["/bin/sh".to_owned(), "-c".to_owned(), script]),
            ..Default::default()
        };
        let id = self
            .client
            .create_exec(container, config)
            .await
            .map_err(map_err)?
            .id;
        let start = StartExecOptions {
            detach: true,
            tty: false,
            output_capacity: None,
        };
        self.client
            .start_exec(&id, Some(start))
            .await
            .map(drop)
            .map_err(map_err)
    }

    async fn resize_exec(&self, exec_id: &str, size: TermSize) -> DockerResult<()> {
        let options = ResizeExecOptionsBuilder::new()
            .h(i32::from(size.rows))
            .w(i32::from(size.cols))
            .build();
        self.client
            .resize_exec(exec_id, options)
            .await
            .map_err(map_err)
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

/// Registry credentials as Docker takes them (`X-Registry-Auth`).
fn credentials(auth: RegistryAuth) -> ::bollard::auth::DockerCredentials {
    ::bollard::auth::DockerCredentials {
        username: Some(auth.username),
        password: Some(auth.password),
        serveraddress: Some(auth.server),
        ..Default::default()
    }
}

/// Kills the container's processes carrying `SESSION_ENV=token` (a command
/// run by [`DockerApi::run_in_container`] and what it started).
async fn kill_marked(client: &Client, container: &str, token: &str) -> DockerResult<()> {
    let script = format!(
        r#"m='{SESSION_ENV}={token}'
for p in /proc/[0-9]*; do
  tr '\0' '\n' < "$p/environ" 2>/dev/null | grep -qx "$m" && kill -KILL "${{p#/proc/}}" 2>/dev/null
done; true"#
    );
    let config = CreateExecOptions {
        cmd: Some(vec!["/bin/sh".to_owned(), "-c".to_owned(), script]),
        ..Default::default()
    };
    let id = client
        .create_exec(container, config)
        .await
        .map_err(map_err)?
        .id;
    let start = StartExecOptions {
        detach: true,
        tty: false,
        output_capacity: None,
    };
    client
        .start_exec(&id, Some(start))
        .await
        .map(drop)
        .map_err(map_err)
}

/// The build's lines, from a task owning its client (the stream outlives
/// the request); dropping the stream stops the task at its next line.
fn build_stream(
    client: Client,
    options: ::bollard::query_parameters::BuildImageOptions,
    context: Vec<u8>,
) -> BoxStream<'static, DockerResult<BuildLine>> {
    let (tx, rx) = tokio::sync::mpsc::channel(64);
    tokio::spawn(async move {
        use ::bollard::models::BuildInfoAux;
        let mut build =
            client.build_image(options, None, Some(::bollard::body_full(context.into())));
        // BuildKit repeats a step while it runs: its name once.
        let mut named = std::collections::HashSet::new();
        loop {
            // A dropped stream (a lost lock, a timeout) stops the build now,
            // not at its next line.
            let item = tokio::select! {
                item = build.next() => item,
                () = tx.closed() => return,
            };
            let Some(item) = item else { break };
            let lines = item.map_err(map_err).and_then(|info| {
                if let Some(detail) = info.error_detail {
                    return Err(DockerError::Other(detail.message.unwrap_or_default()));
                }
                match info.aux {
                    Some(BuildInfoAux::Default(image)) if image.id.is_some() => {
                        return Ok(vec![BuildLine::Image(image.id.unwrap_or_default())]);
                    }
                    Some(BuildInfoAux::BuildKit(status)) => {
                        let mut lines = vec![];
                        for vertex in status.vertexes {
                            if !vertex.error.is_empty() {
                                return Err(DockerError::Other(format!(
                                    "{}: {}",
                                    vertex.name, vertex.error
                                )));
                            }
                            if vertex.started.is_some() && named.insert(vertex.digest) {
                                let cached = if vertex.cached { " (cached)" } else { "" };
                                lines.push(BuildLine::Log(format!("{}{cached}", vertex.name)));
                            }
                        }
                        for log in status.logs {
                            lines.push(BuildLine::Log(
                                String::from_utf8_lossy(&log.msg).into_owned(),
                            ));
                        }
                        return Ok(lines);
                    }
                    _ => {}
                }
                Ok(vec![BuildLine::Log(
                    info.stream.or(info.status).unwrap_or_default(),
                )])
            });
            let lines = match lines {
                Ok(lines) => lines.into_iter().map(Ok).collect(),
                Err(e) => vec![Err(e)],
            };
            for line in lines {
                if tx.send(line).await.is_err() {
                    return;
                }
            }
        }
    });
    futures_util::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|line| (line, rx))
    })
    .boxed()
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
