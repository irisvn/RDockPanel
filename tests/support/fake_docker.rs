//! An in-memory [`DockerApi`]: containers, volumes and networks the test
//! declares, and a record of what the panel asked Docker to do.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use futures_util::StreamExt;
use futures_util::stream::{self, BoxStream};
use rdockpanel::services::docker_service::{
    ContainerAction, ContainerItem, ContainerStats, DockerApi, DockerError, DockerInfo,
    DockerResult, ExecSession, ImageDetails, ImageItem, LogLine, LogQuery, LogStream, NetworkItem,
    NetworkRequest, PruneReport, RegistryAuth, SYSTEM_LABEL, ServiceSpec, ServiceUpdateStatus,
    ServiceUpdateStatusStateEnum, SwarmRole, SwarmService, SwarmTask, TaskState, TermSize,
    VolumeItem,
};
use rustcms::async_trait;
use serde_json::{Value, json};

#[derive(Clone, Default)]
pub struct FakeDocker {
    pub containers: Arc<Mutex<Vec<ContainerItem>>>,
    pub volumes: Arc<Mutex<Vec<VolumeItem>>>,
    pub networks: Arc<Mutex<Vec<NetworkItem>>>,
    pub images: Arc<Mutex<Vec<ImageItem>>>,
    /// Swarm services by name.
    pub services: Arc<Mutex<HashMap<String, SwarmService>>>,
    /// The state every task reports (default: running).
    pub task_state: Arc<Mutex<Option<TaskState>>>,
    /// The update status an update leaves (default: none, like a create).
    pub update_state: Arc<Mutex<Option<ServiceUpdateStatusStateEnum>>>,
    /// A stopped service's task that Swarm wants shut down but still runs.
    pub lingering: Arc<Mutex<bool>>,
    /// `"restart web"`, `"remove web"`, `"prune rdockpanel.system"`, …
    pub calls: Arc<Mutex<Vec<String>>>,
}

pub fn labels(system: bool) -> HashMap<String, String> {
    if system {
        HashMap::from([(SYSTEM_LABEL.to_owned(), "true".to_owned())])
    } else {
        HashMap::new()
    }
}

impl FakeDocker {
    /// `web` (an app) and `panel` (the panel itself, system-labelled), each
    /// with an image, a volume and a network of the same kind. Ids differ
    /// from names as on a real engine: container `id-web`, image
    /// `sha256:img-web` (tags `web:latest`; the panel's also `panel:old`).
    pub fn with_app_and_panel() -> Self {
        let fake = Self::default();
        for (name, system) in [("web", false), ("panel", true)] {
            fake.containers.lock().unwrap().push(ContainerItem {
                id: format!("id-{name}"),
                name: name.to_owned(),
                image: format!("{name}:latest"),
                image_id: format!("sha256:img-{name}"),
                state: "running".to_owned(),
                status: "Up".to_owned(),
                created: 0,
                ports: vec![],
                labels: labels(system),
            });
            let mut tags = vec![format!("{name}:latest")];
            if system {
                tags.push(format!("{name}:old"));
            }
            fake.images.lock().unwrap().push(ImageItem {
                id: format!("sha256:img-{name}"),
                tags,
                size: 1,
                created: 0,
                containers: 1,
                labels: HashMap::new(),
            });
            fake.volumes.lock().unwrap().push(VolumeItem {
                name: format!("{name}-data"),
                driver: "local".to_owned(),
                mountpoint: String::new(),
                labels: labels(system),
            });
            fake.networks.lock().unwrap().push(NetworkItem {
                id: format!("netid-{name}"),
                name: format!("{name}-net"),
                driver: "overlay".to_owned(),
                scope: "swarm".to_owned(),
                labels: labels(system),
            });
        }
        fake
    }

    pub fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn record(&self, call: String) {
        self.calls.lock().unwrap().push(call);
    }

    /// By id or name, as Docker resolves them.
    fn container(&self, id: &str) -> DockerResult<ContainerItem> {
        let containers = self.containers.lock().unwrap();
        containers
            .iter()
            .find(|c| c.id == id || c.name == id)
            .cloned()
            .ok_or(DockerError::NotFound)
    }

    /// Only full ids reach mutations: the panel resolves names first.
    fn by_id(&self, id: &str) -> DockerResult<ContainerItem> {
        self.containers
            .lock()
            .unwrap()
            .iter()
            .find(|c| c.id == id)
            .cloned()
            .ok_or(DockerError::NotFound)
    }
}

#[async_trait]
impl DockerApi for FakeDocker {
    async fn info(&self) -> DockerResult<DockerInfo> {
        Ok(DockerInfo {
            name: "fake".to_owned(),
            server_version: "29.0.0".to_owned(),
            operating_system: "Fake OS".to_owned(),
            kernel_version: "6.0".to_owned(),
            architecture: "x86_64".to_owned(),
            cpus: 2,
            mem_total: 1 << 30,
            containers: self.containers.lock().unwrap().len() as i64,
            containers_running: 0,
            images: 0,
            swarm: SwarmRole::Manager,
        })
    }
    async fn containers(&self) -> DockerResult<Vec<ContainerItem>> {
        Ok(self.containers.lock().unwrap().clone())
    }
    async fn inspect_container(&self, id: &str) -> DockerResult<Value> {
        let c = self.container(id)?;
        Ok(
            json!({"Id": c.id, "Config": {"Labels": c.labels, "Env": ["PATH=/bin", "DB_PASSWORD=hunter2"]}}),
        )
    }
    async fn container_action(&self, id: &str, action: ContainerAction) -> DockerResult<()> {
        self.by_id(id)?;
        self.record(format!(
            "{} {id}",
            serde_json::to_value(action).unwrap().as_str().unwrap()
        ));
        Ok(())
    }
    async fn remove_container(&self, id: &str, _force: bool) -> DockerResult<()> {
        self.by_id(id)?;
        self.record(format!("remove {id}"));
        Ok(())
    }
    fn logs(&self, id: &str, _query: LogQuery) -> BoxStream<'static, DockerResult<LogLine>> {
        let line = |stream, text: &str| {
            Ok(LogLine {
                stream,
                text: text.to_owned(),
            })
        };
        stream::iter([
            line(LogStream::Stdout, &format!("{id} started\n")),
            line(LogStream::Stderr, "warn\n"),
        ])
        .boxed()
    }
    fn stats(&self, _id: &str) -> BoxStream<'static, DockerResult<ContainerStats>> {
        stream::iter([Ok(ContainerStats {
            cpu_percent: 12.5,
            ..Default::default()
        })])
        .boxed()
    }
    async fn images(&self) -> DockerResult<Vec<ImageItem>> {
        Ok(self.images.lock().unwrap().clone())
    }
    fn pull_image(&self, image: &str) -> BoxStream<'static, DockerResult<String>> {
        stream::iter([Ok(format!("Pulling {image}")), Ok("Done".to_owned())]).boxed()
    }
    async fn remove_image(&self, id: &str, force: bool) -> DockerResult<()> {
        self.record(format!("remove-image {id} force={force}"));
        Ok(())
    }
    async fn prune(&self, exclude_label: &str) -> DockerResult<PruneReport> {
        self.record(format!("prune {exclude_label}"));
        Ok(PruneReport::default())
    }
    async fn volumes(&self) -> DockerResult<Vec<VolumeItem>> {
        Ok(self.volumes.lock().unwrap().clone())
    }
    async fn create_volume(&self, name: &str) -> DockerResult<VolumeItem> {
        let volume = VolumeItem {
            name: name.to_owned(),
            driver: "local".to_owned(),
            mountpoint: String::new(),
            labels: HashMap::new(),
        };
        self.volumes.lock().unwrap().push(volume.clone());
        Ok(volume)
    }
    async fn remove_volume(&self, name: &str) -> DockerResult<()> {
        self.record(format!("remove-volume {name}"));
        Ok(())
    }
    async fn networks(&self) -> DockerResult<Vec<NetworkItem>> {
        Ok(self.networks.lock().unwrap().clone())
    }
    async fn create_network(&self, name: &str, driver: &str) -> DockerResult<NetworkItem> {
        Ok(NetworkItem {
            id: name.to_owned(),
            name: name.to_owned(),
            driver: driver.to_owned(),
            scope: "local".to_owned(),
            labels: HashMap::new(),
        })
    }
    async fn remove_network(&self, id: &str) -> DockerResult<()> {
        self.record(format!("remove-network {id}"));
        Ok(())
    }
    /// A shell that echoes what it is sent, after a prompt.
    async fn exec(
        &self,
        container: &str,
        size: TermSize,
        token: &str,
    ) -> DockerResult<ExecSession> {
        let c = self.by_id(container)?;
        self.record(format!("exec {} {}x{}", c.id, size.cols, size.rows));
        self.record(format!("token {token}"));
        let (input, shell) = tokio::io::duplex(64 * 1024);
        let prompt = stream::iter([Ok(format!("{}$ ", c.name).into_bytes())]);
        let echo = stream::unfold(shell, |mut shell| async move {
            use tokio::io::AsyncReadExt;
            let mut buffer = vec![0; 4096];
            match shell.read(&mut buffer).await {
                Ok(0) | Err(_) => None,
                Ok(n) => {
                    buffer.truncate(n);
                    Some((Ok(buffer), shell))
                }
            }
        });
        Ok(ExecSession {
            id: format!("exec-{}", c.name),
            output: prompt.chain(echo).boxed(),
            input: Box::pin(input),
        })
    }
    async fn end_exec(&self, container: &str, token: &str) -> DockerResult<()> {
        self.record(format!("end exec {container} {token}"));
        Ok(())
    }
    async fn resize_exec(&self, exec_id: &str, size: TermSize) -> DockerResult<()> {
        self.record(format!("resize {exec_id} {}x{}", size.cols, size.rows));
        Ok(())
    }
    async fn ensure_network(&self, request: NetworkRequest) -> DockerResult<NetworkItem> {
        let mut networks = self.networks.lock().unwrap();
        if let Some(found) = networks.iter().find(|n| n.name == request.name) {
            return Ok(found.clone());
        }
        self.record(format!(
            "create network {} {}",
            request.name, request.driver
        ));
        let network = NetworkItem {
            id: format!("netid-{}", request.name),
            name: request.name,
            driver: request.driver,
            scope: "swarm".to_owned(),
            labels: request.labels,
        };
        networks.push(network.clone());
        Ok(network)
    }
    async fn inspect_image(&self, reference: &str) -> DockerResult<ImageDetails> {
        let repo = reference.split(['@', ':']).next().unwrap_or(reference);
        Ok(ImageDetails {
            id: format!("sha256:id-{repo}"),
            repo_digests: vec![format!("{repo}@sha256:fake")],
        })
    }
    async fn inspect_service(&self, name: &str) -> DockerResult<Option<SwarmService>> {
        Ok(self.services.lock().unwrap().get(name).cloned())
    }
    async fn create_service(
        &self,
        spec: ServiceSpec,
        _auth: Option<RegistryAuth>,
    ) -> DockerResult<String> {
        let name = spec.name.clone().unwrap_or_default();
        let mut services = self.services.lock().unwrap();
        if services.contains_key(&name) {
            return Err(DockerError::Conflict(format!("service {name} exists")));
        }
        self.record(format!("create service {name}"));
        let id = format!("svc-{name}");
        services.insert(
            name,
            SwarmService {
                id: Some(id.clone()),
                version: Some(rdockpanel::services::docker_service::ObjectVersion {
                    index: Some(1),
                }),
                spec: Some(spec),
                ..Default::default()
            },
        );
        Ok(id)
    }
    async fn update_service(
        &self,
        name: &str,
        version: u64,
        spec: ServiceSpec,
        _auth: Option<RegistryAuth>,
    ) -> DockerResult<Vec<String>> {
        let mut services = self.services.lock().unwrap();
        let service = services.get_mut(name).ok_or(DockerError::NotFound)?;
        let current = service.version.as_ref().and_then(|v| v.index).unwrap_or(0);
        if current != version {
            return Err(DockerError::Conflict("update out of sequence".to_owned()));
        }
        self.record(format!("update service {name}"));
        service.version = Some(rdockpanel::services::docker_service::ObjectVersion {
            index: Some(current + 1),
        });
        service.spec = Some(spec);
        if let Some(state) = *self.update_state.lock().unwrap() {
            service.update_status = Some(ServiceUpdateStatus {
                state: Some(state),
                started_at: serde_json::from_value(serde_json::json!(
                    chrono::Utc::now().to_rfc3339()
                ))
                .ok(),
                message: Some(format!("update {state}")),
                ..Default::default()
            });
        }
        Ok(vec![])
    }
    async fn delete_service(&self, name: &str) -> DockerResult<()> {
        self.services
            .lock()
            .unwrap()
            .remove(name)
            .ok_or(DockerError::NotFound)?;
        self.record(format!("delete service {name}"));
        Ok(())
    }
    /// One running task per replica.
    /// One task per replica, running the service's current task template,
    /// in the state set by `task_state`.
    async fn list_tasks(&self, service: &str) -> DockerResult<Vec<SwarmTask>> {
        let services = self.services.lock().unwrap();
        let Some(found) = services.get(service) else {
            return Ok(vec![]);
        };
        let spec = found.spec.clone().unwrap_or_default();
        let replicas = spec
            .mode
            .as_ref()
            .and_then(|m| m.replicated.as_ref())
            .and_then(|r| r.replicas)
            .unwrap_or(1);
        let state = self
            .task_state
            .lock()
            .unwrap()
            .unwrap_or(TaskState::RUNNING);
        let now = serde_json::from_value(serde_json::json!(chrono::Utc::now().to_rfc3339())).ok();
        if replicas == 0 && *self.lingering.lock().unwrap() {
            return Ok(vec![SwarmTask {
                id: Some(format!("task-{service}-1")),
                slot: Some(1),
                spec: spec.task_template.clone(),
                desired_state: Some(TaskState::SHUTDOWN),
                status: Some(rdockpanel::services::docker_service::TaskStatus {
                    state: Some(TaskState::RUNNING),
                    timestamp: now,
                    ..Default::default()
                }),
                ..Default::default()
            }]);
        }
        // A failing task is restarted by Swarm: three tries per slot.
        let tries = if state == TaskState::FAILED { 3 } else { 1 };
        Ok((1..=replicas)
            .flat_map(|slot| std::iter::repeat_n(slot, tries))
            .map(|slot| SwarmTask {
                id: Some(format!("task-{service}-{slot}")),
                slot: Some(slot),
                spec: spec.task_template.clone(),
                desired_state: Some(TaskState::RUNNING),
                status: Some(rdockpanel::services::docker_service::TaskStatus {
                    state: Some(state),
                    timestamp: now.clone(),
                    err: (state == TaskState::FAILED).then(|| "exit code 1".to_owned()),
                    ..Default::default()
                }),
                ..Default::default()
            })
            .collect())
    }
    fn service_logs(
        &self,
        service: &str,
        _query: LogQuery,
    ) -> BoxStream<'static, DockerResult<LogLine>> {
        let line = LogLine {
            stream: LogStream::Stdout,
            text: format!("{service} ready\n"),
        };
        stream::iter([Ok(line)]).boxed()
    }
}
