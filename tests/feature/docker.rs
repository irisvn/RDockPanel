//! The panel's rules over Docker: its own objects (label
//! `rdockpanel.system=true`) are hidden and untouchable without the system
//! scope, prune skips them, inspect hides secrets. With `PANEL_TEST_DOCKER=1`
//! the last test runs against the real engine.

use futures_util::StreamExt;
use rdockpanel::services::docker_service::{
    BollardDocker, ContainerAction, Docker, DockerError, LogQuery, SYSTEM_LABEL, Scope, TermSize,
};

use crate::support::fake_docker::FakeDocker;

fn docker() -> (Docker, FakeDocker) {
    let fake = FakeDocker::with_app_and_panel();
    (Docker::new(fake.clone()), fake)
}

#[tokio::test]
async fn system_objects_are_hidden_from_users() {
    let (docker, _) = docker();
    let names = |items: Vec<String>| items.join(",");
    let users = docker
        .containers(Scope::User)
        .await
        .unwrap()
        .into_iter()
        .map(|c| c.name)
        .collect();
    assert_eq!(names(users), "web");
    let system = docker
        .containers(Scope::System)
        .await
        .unwrap()
        .into_iter()
        .map(|c| c.name)
        .collect();
    assert_eq!(names(system), "web,panel");
    let volumes = docker
        .volumes(Scope::User)
        .await
        .unwrap()
        .into_iter()
        .map(|v| v.name)
        .collect();
    assert_eq!(names(volumes), "web-data");
    let networks = docker
        .networks(Scope::User)
        .await
        .unwrap()
        .into_iter()
        .map(|n| n.name)
        .collect();
    assert_eq!(names(networks), "web-net");
}

#[tokio::test]
async fn system_objects_cannot_be_touched_by_users() {
    let (docker, fake) = docker();
    let protected = |r: Result<(), DockerError>| matches!(r, Err(DockerError::Protected));

    assert!(protected(
        docker
            .container_action("panel", ContainerAction::Stop, Scope::User)
            .await
    ));
    assert!(protected(
        docker.remove_container("panel", true, Scope::User).await
    ));
    assert!(protected(
        docker
            .inspect_container("panel", Scope::User)
            .await
            .map(drop)
    ));
    assert!(protected(
        docker
            .logs("panel", LogQuery::default(), Scope::User)
            .await
            .map(drop)
    ));
    assert!(protected(
        docker.stats("panel", Scope::User).await.map(drop)
    ));
    assert!(protected(
        docker.remove_volume("panel-data", Scope::User).await
    ));
    assert!(protected(
        docker.remove_network("panel-net", Scope::User).await
    ));
    assert!(protected(
        docker.remove_image("panel:latest", true, Scope::User).await
    ));
    assert!(
        fake.calls().is_empty(),
        "nothing reached Docker: {:?}",
        fake.calls()
    );

    docker
        .container_action("web", ContainerAction::Restart, Scope::User)
        .await
        .unwrap();
    docker
        .container_action("panel", ContainerAction::Restart, Scope::System)
        .await
        .unwrap();
    docker.remove_volume("web-data", Scope::User).await.unwrap();
    assert_eq!(
        fake.calls(),
        [
            "restart id-web",
            "restart id-panel",
            "remove-volume web-data"
        ]
    );
}

#[tokio::test]
async fn unknown_objects_are_not_found() {
    let (docker, _) = docker();
    assert!(matches!(
        docker
            .container_action("nope", ContainerAction::Start, Scope::User)
            .await,
        Err(DockerError::NotFound)
    ));
    assert!(matches!(
        docker.remove_volume("nope", Scope::User).await,
        Err(DockerError::NotFound)
    ));
}

#[tokio::test]
async fn prune_skips_system_objects_and_inspect_hides_secrets() {
    let (docker, fake) = docker();
    docker.prune().await.unwrap();
    assert_eq!(fake.calls(), [format!("prune {SYSTEM_LABEL}")]);

    let doc = docker.inspect_container("web", Scope::User).await.unwrap();
    assert_eq!(
        doc["Config"]["Env"],
        serde_json::json!(["PATH=/bin", "DB_PASSWORD=******"])
    );
}

#[tokio::test]
async fn identifiers_never_carry_paths_into_docker() {
    let (docker, fake) = docker();
    let not_found = |r: Result<(), DockerError>| matches!(r, Err(DockerError::NotFound));
    assert!(not_found(
        docker
            .remove_image("../containers/panel", false, Scope::User)
            .await
    ));
    assert!(not_found(
        docker
            .container_action("../containers/panel", ContainerAction::Stop, Scope::User)
            .await
    ));
    assert!(not_found(
        docker
            .remove_volume("../volumes/panel-data", Scope::User)
            .await
    ));
    assert!(not_found(docker.remove_network("a/b", Scope::User).await));
    assert!(fake.calls().is_empty(), "{:?}", fake.calls());
}

#[tokio::test]
async fn mutations_use_the_checked_objects_id() {
    let (docker, fake) = docker();
    docker
        .container_action("web", ContainerAction::Restart, Scope::User)
        .await
        .unwrap();
    // A user network named like the panel network's id: Docker resolves the id first, so do we.
    fake.networks
        .lock()
        .unwrap()
        .push(rdockpanel::services::docker_service::NetworkItem {
            id: "netid-decoy".to_owned(),
            name: "netid-panel".to_owned(),
            driver: "bridge".to_owned(),
            scope: "local".to_owned(),
            labels: Default::default(),
        });
    assert!(matches!(
        docker.remove_network("netid-panel", Scope::User).await,
        Err(DockerError::Protected)
    ));
    docker.remove_network("web-net", Scope::User).await.unwrap();
    assert_eq!(fake.calls(), ["restart id-web", "remove-network netid-web"]);
}

#[tokio::test]
async fn images_of_the_panel_are_protected() {
    let (docker, fake) = docker();
    for reference in ["panel:old", "panel:latest", "sha256:img-panel", "img-panel"] {
        let refused = docker.remove_image(reference, false, Scope::User).await;
        assert!(
            matches!(refused, Err(DockerError::Protected)),
            "{reference}"
        );
    }
    assert!(matches!(
        docker.remove_image("web:latest", true, Scope::User).await,
        Err(DockerError::Protected)
    ));
    docker
        .remove_image("web:latest", false, Scope::User)
        .await
        .unwrap();
    docker
        .remove_image("panel:old", false, Scope::System)
        .await
        .unwrap();
    assert!(matches!(
        docker.remove_image("nope:1", false, Scope::User).await,
        Err(DockerError::NotFound)
    ));
    assert_eq!(
        fake.calls(),
        [
            "remove-image web:latest force=false",
            "remove-image panel:old force=false"
        ]
    );
}

#[tokio::test]
async fn existing_volumes_are_not_handed_back() {
    let (docker, _) = docker();
    assert!(matches!(
        docker.create_volume("panel-data").await,
        Err(DockerError::Conflict(_))
    ));
    assert_eq!(docker.create_volume("fresh").await.unwrap().name, "fresh");
}

/// Runs a throwaway alpine container: logs, stats, actions, removal.
#[tokio::test]
async fn against_the_real_engine() {
    if std::env::var_os("PANEL_TEST_DOCKER").is_none() {
        return;
    }
    let docker = Docker::new(BollardDocker::connect().expect("docker client"));
    let info = docker.info().await.expect("docker reachable");
    assert!(!info.server_version.is_empty());

    let pulled: Vec<_> = docker.pull_image("alpine:3.20").collect().await;
    assert!(pulled.iter().all(Result::is_ok), "{pulled:?}");

    let name = format!("rdockpanel-test-{}", std::process::id());
    let status = std::process::Command::new("docker")
        .args([
            "run",
            "-d",
            "--name",
            &name,
            "--label",
            "rdockpanel.test=1",
            "alpine:3.20",
            "sh",
            "-c",
        ])
        .arg("echo hello-out; echo hello-err >&2; sleep 300")
        .status()
        .expect("docker cli");
    assert!(status.success());

    // Remove the container even when an assertion fails.
    let checks = std::panic::AssertUnwindSafe(async {
        let listed = docker.containers(Scope::User).await.unwrap();
        assert!(
            listed
                .iter()
                .any(|c| c.name == name && c.state == "running")
        );

        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let query = LogQuery {
            tail: Some(10),
            ..Default::default()
        };
        let lines: Vec<_> = docker
            .logs(&name, query, Scope::User)
            .await
            .unwrap()
            .collect()
            .await;
        let text: String = lines.into_iter().map(|l| l.unwrap().text).collect();
        assert!(
            text.contains("hello-out") && text.contains("hello-err"),
            "{text}"
        );

        let mut stats = docker.stats(&name, Scope::User).await.unwrap();
        let reading = stats.next().await.unwrap().unwrap();
        assert!(reading.mem_used > 0, "{reading:?}");

        docker
            .container_action(&name, ContainerAction::Pause, Scope::User)
            .await
            .unwrap();
        docker
            .container_action(&name, ContainerAction::Unpause, Scope::User)
            .await
            .unwrap();

        // A shell: a command's output comes back, and resizing is accepted.
        use tokio::io::AsyncWriteExt;
        let size = TermSize {
            cols: 100,
            rows: 30,
        };
        let token = rdockpanel::services::docker_service::session_token();
        let mut shell = docker.exec(&name, size, Scope::User, &token).await.unwrap();
        docker
            .resize_exec(
                &shell.session.id,
                TermSize {
                    cols: 120,
                    rows: 40,
                },
            )
            .await
            .unwrap();
        shell
            .session
            .input
            .write_all(b"echo rdp-$((40+2))\n")
            .await
            .unwrap();
        let mut seen = String::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !seen.contains("rdp-42") && std::time::Instant::now() < deadline {
            let next = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                shell.session.output.next(),
            )
            .await;
            if let Ok(Some(Ok(bytes))) = next {
                seen.push_str(&String::from_utf8_lossy(&bytes));
            }
        }
        assert!(seen.contains("rdp-42"), "{seen:?}");

        // What ignores a hang-up, or clears its environment, ends too.
        shell
            .session
            .input
            .write_all(b"nohup sleep 3001 >/dev/null 2>&1 & env -i /bin/sleep 3002 &\n")
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        // Dropping it ends the shell: no process of the container keeps its mark.
        drop(shell);
        let marked = || {
            let out = std::process::Command::new("docker")
                .args([
                    "exec",
                    &name,
                    "sh",
                    "-c",
                    "grep -l RDOCKPANEL_SESSION /proc/[0-9]*/environ 2>/dev/null; true",
                ])
                .output()
                .expect("docker cli");
            String::from_utf8_lossy(&out.stdout).trim().to_owned()
        };
        let mut left = marked();
        for _ in 0..20 {
            if left.is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            left = marked();
        }
        assert!(left.is_empty(), "shell processes left: {left}");
        let sleeping = || {
            let out = std::process::Command::new("docker")
                .args([
                    "exec",
                    &name,
                    "sh",
                    "-c",
                    "ps -o args | grep -c '^/bin/sleep 3002\\|^sleep 3001'; true",
                ])
                .output()
                .expect("docker cli");
            String::from_utf8_lossy(&out.stdout).trim().to_owned()
        };
        let mut count = sleeping();
        for _ in 0..30 {
            if count == "0" {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            count = sleeping();
        }
        assert_eq!(count, "0", "background processes of the shell survived");
    });
    let outcome = futures_util::FutureExt::catch_unwind(checks).await;
    docker
        .remove_container(&name, true, Scope::User)
        .await
        .unwrap();
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}

fn managed_spec(
    name: &str,
    project: &str,
    service: &str,
    managed: bool,
) -> rdockpanel::services::docker_service::ServiceSpec {
    let mut labels = std::collections::HashMap::new();
    if managed {
        labels.insert("rdockpanel.managed".to_owned(), "true".to_owned());
        labels.insert("rdockpanel.project".to_owned(), project.to_owned());
        labels.insert("rdockpanel.service".to_owned(), service.to_owned());
    }
    serde_json::from_value(serde_json::json!({
        "Name": name,
        "Labels": labels,
        "TaskTemplate": {"ContainerSpec": {"Image": "alpine:3.20", "Command": ["sleep", "300"]}},
        "Mode": {"Replicated": {"Replicas": 1}}
    }))
    .unwrap()
}

#[tokio::test]
async fn services_the_panel_did_not_deploy_are_never_taken_over() {
    let (docker, _) = docker();
    assert!(
        docker
            .managed_service("shop_web", "shop", "web")
            .await
            .unwrap()
            .is_none()
    );
    docker
        .engine()
        .create_service(managed_spec("shop_web", "shop", "web", true), None)
        .await
        .unwrap();
    assert!(
        docker
            .managed_service("shop_web", "shop", "web")
            .await
            .unwrap()
            .is_some()
    );
    // Another project's, or nobody's: a conflict, whatever the name says.
    assert!(matches!(
        docker.managed_service("shop_web", "other", "web").await,
        Err(DockerError::Conflict(_))
    ));
    docker
        .engine()
        .create_service(managed_spec("shop_api", "", "", false), None)
        .await
        .unwrap();
    assert!(matches!(
        docker.managed_service("shop_api", "shop", "api").await,
        Err(DockerError::Conflict(_))
    ));
    assert!(matches!(
        docker.managed_service("../x", "shop", "x").await,
        Err(DockerError::NotFound)
    ));
}

#[tokio::test]
async fn foreign_networks_are_not_joined() {
    use rdockpanel::services::docker_service::NetworkRequest;
    let (docker, _) = docker();
    let ours = |name: &str| NetworkRequest {
        name: name.to_owned(),
        driver: "overlay".to_owned(),
        labels: [("rdockpanel.managed".to_owned(), "true".to_owned())].into(),
        attachable: true,
        internal: false,
    };
    // `web-net` exists, without the panel's labels.
    assert!(matches!(
        docker.ensure_network(ours("web-net")).await,
        Err(DockerError::Conflict(_))
    ));
    let created = docker.ensure_network(ours("rdp_shop")).await.unwrap();
    assert_eq!(
        docker.ensure_network(ours("rdp_shop")).await.unwrap().id,
        created.id
    );
    assert!(matches!(
        docker.ensure_network(ours("../x")).await,
        Err(DockerError::NotFound)
    ));
}

/// With `PANEL_TEST_SWARM=1` (a Swarm manager): an overlay network and a
/// service created, updated at its version, and removed.
#[tokio::test]
async fn against_a_real_swarm() {
    if std::env::var_os("PANEL_TEST_SWARM").is_none() {
        return;
    }
    let _swarm = crate::support::swarm_lock().await;
    use rdockpanel::services::docker_service::{NetworkRequest, SwarmRole};
    let docker = Docker::new(BollardDocker::connect().expect("docker client"));
    assert_eq!(docker.info().await.unwrap().swarm, SwarmRole::Manager);
    let suffix = std::process::id();
    let (project, network, name) = (
        format!("t{suffix}"),
        format!("rdp_t{suffix}"),
        format!("t{suffix}_web"),
    );
    let api = docker.engine();

    let request = NetworkRequest {
        name: network.clone(),
        driver: "overlay".to_owned(),
        labels: [("rdockpanel.managed".to_owned(), "true".to_owned())].into(),
        attachable: true,
        internal: false,
    };
    let created = docker.ensure_network(request.clone()).await.unwrap();
    assert_eq!(
        docker.ensure_network(request).await.unwrap().id,
        created.id,
        "an existing network is kept"
    );

    let checks = std::panic::AssertUnwindSafe(async {
        let mut spec = managed_spec(&name, &project, "web", true);
        spec.task_template.as_mut().unwrap().networks = Some(vec![
            rdockpanel::services::docker_service::NetworkAttachmentConfig {
                target: Some(network.clone()),
                ..Default::default()
            },
        ]);
        api.create_service(spec.clone(), None).await.unwrap();
        let service = docker
            .managed_service(&name, &project, "web")
            .await
            .unwrap()
            .expect("created");
        let version = service.version.and_then(|v| v.index).unwrap();

        // Running within a minute.
        let mut running = false;
        for _ in 0..60 {
            let tasks = api.list_tasks(&name).await.unwrap();
            if tasks.iter().any(|t| {
                t.status.as_ref().and_then(|s| s.state)
                    == Some(rdockpanel::services::docker_service::TaskState::RUNNING)
            }) {
                running = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
        assert!(running, "no running task");

        // An update needs the current version.
        spec.mode =
            serde_json::from_value(serde_json::json!({"Replicated": {"Replicas": 2}})).unwrap();
        assert!(matches!(
            api.update_service(&name, version + 99, spec.clone(), None)
                .await,
            Err(DockerError::Conflict(_))
        ));
        // At the version it has now (Swarm bumps it on its own), retried
        // when it moved meanwhile.
        let mut updated = false;
        for _ in 0..5 {
            let current = api
                .inspect_service(&name)
                .await
                .unwrap()
                .and_then(|s| s.version)
                .and_then(|v| v.index)
                .unwrap();
            match api.update_service(&name, current, spec.clone(), None).await {
                Ok(_) => {
                    updated = true;
                    break;
                }
                Err(DockerError::Conflict(_)) => continue,
                Err(err) => panic!("{err}"),
            }
        }
        assert!(updated);
        // Scaled to two running replicas.
        let mut running = 0;
        for _ in 0..60 {
            let tasks = api.list_tasks(&name).await.unwrap();
            running = tasks
                .iter()
                .filter(|t| {
                    t.desired_state
                        == Some(rdockpanel::services::docker_service::TaskState::RUNNING)
                })
                .filter(|t| {
                    t.status.as_ref().and_then(|s| s.state)
                        == Some(rdockpanel::services::docker_service::TaskState::RUNNING)
                })
                .count();
            if running == 2 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
        assert_eq!(running, 2);
        let details = api.inspect_image("alpine:3.20").await.unwrap();
        assert!(details.id.starts_with("sha256:"));
    });
    let outcome = futures_util::FutureExt::catch_unwind(checks).await;
    let _ = api.delete_service(&name).await;
    // The network goes once the service's tasks have left it.
    for _ in 0..30 {
        if api.remove_network(&network).await.is_ok() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
    assert!(api.inspect_service(&name).await.unwrap().is_none());
}

/// With `PANEL_TEST_SWARM=1`: what `desired` makes of a service is a spec
/// Docker runs (pinned image, health check, volume, command, labels).
#[tokio::test]
async fn desired_specs_run_on_a_real_swarm() {
    if std::env::var_os("PANEL_TEST_SWARM").is_none() {
        return;
    }
    let _swarm = crate::support::swarm_lock().await;
    use rdockpanel::services::deploy_service::{
        DeployOptions, Healthcheck, Mount, Snapshot, desired, network_name, pin_image, volume_name,
    };
    use rdockpanel::services::docker_service::{NetworkRequest, TaskState};
    let docker = Docker::new(BollardDocker::connect().expect("docker client"));
    let api = docker.engine();
    let project = format!("d{}", std::process::id());
    let snapshot = Snapshot {
        project: project.clone(),
        service: "app".to_owned(),
        image: "alpine:3.20".to_owned(),
        command: Some("sh -c 'echo ok > /data/ready; sleep 300'".to_owned()),
        replicas: 1,
        running: true,
        cpu_limit_millis: Some(250),
        mem_limit_mb: Some(64),
        cpu_reservation_millis: None,
        mem_reservation_mb: None,
        zero_downtime: true,
        healthcheck: Some(Healthcheck {
            // Healthy once its volume and its file (a Swarm config) are there.
            command: "test -f /data/ready && grep -q hi /etc/app/greeting.txt".to_owned(),
            interval_s: 1,
            retries: 3,
        }),
        domains: vec![],
        ports: vec![],
        mounts: vec![
            Mount {
                target: "/data".to_owned(),
                kind: "volume".to_owned(),
                source: "data".to_owned(),
                read_only: false,
                content_digest: None,
            },
            Mount {
                target: "/etc/app/greeting.txt".to_owned(),
                kind: "file".to_owned(),
                source: String::new(),
                read_only: true,
                content_digest: Some("digest-of-hi".to_owned()),
            },
        ],
        env_names: vec![],
        env_digest: None,
        registry_username: None,
        primary_domain: None,
        build: None,
        compose: None,
        cap_add: vec![],
        cap_drop: vec![],
        sysctls: Default::default(),
        groups: vec![],
        maintenance: false,
        redirects: vec![],
        basic_auth: vec![],
    };
    // An environment, as a deployment opens it.
    let env = std::collections::BTreeMap::from([("GREETING".to_owned(), "hello".to_owned())]);
    let snapshot = snapshot.with_env(&env, b"a key");
    let name = snapshot.service_name();
    let network = NetworkRequest {
        name: network_name(&project),
        driver: "overlay".to_owned(),
        labels: [("rdockpanel.managed".to_owned(), "true".to_owned())].into(),
        attachable: true,
        internal: false,
    };
    docker.ensure_network(network).await.unwrap();
    let pulled: Vec<String> = docker
        .pull_image("alpine:3.20")
        .map(|line| line.unwrap())
        .collect()
        .await;
    let digest =
        rdockpanel::services::deploy_service::pulled_digest(pulled.iter().map(String::as_str));
    let image = pin_image(
        &snapshot.image,
        digest.as_deref(),
        &api.inspect_image("alpine:3.20").await.unwrap(),
    );
    assert!(image.digest().is_some(), "{image:?}");
    // The file's config, as a deployment makes it.
    let file = &snapshot.mounts[1];
    let config_name = file.config_name(&snapshot).unwrap();
    let config_id = api
        .create_config(&config_name, b"hi from a config\n", Default::default())
        .await
        .unwrap();
    let runtime = rdockpanel::services::deploy_service::Runtime {
        env: env.clone(),
        env_digest: None,
        configs: [(
            file.target.clone(),
            rdockpanel::services::deploy_service::ConfigRef {
                id: config_id.clone(),
                name: config_name,
            },
        )]
        .into(),
    };
    let out = desired(&snapshot, &image, &runtime, &DeployOptions::default());

    let checks = std::panic::AssertUnwindSafe(async {
        api.create_service(out.spec.clone(), None).await.unwrap();
        let service = docker
            .managed_service(&name, &project, "app")
            .await
            .unwrap()
            .expect("created");
        let spec = service.spec.unwrap_or_default();
        let env = spec
            .task_template
            .as_ref()
            .and_then(|t| t.container_spec.as_ref())
            .and_then(|c| c.env.clone());
        assert_eq!(env, Some(vec!["GREETING=hello".to_owned()]));
        let labels = spec.labels.unwrap_or_default();
        assert_eq!(labels.get("rdockpanel.spec-hash"), Some(&out.hash));
        // Running, and healthy (the command wrote the file the check looks for).
        let mut healthy = false;
        for _ in 0..90 {
            let tasks = api.list_tasks(&name).await.unwrap();
            let running = tasks
                .iter()
                .find(|t| t.status.as_ref().and_then(|s| s.state) == Some(TaskState::RUNNING));
            if let Some(container) = running.and_then(|t| {
                t.status
                    .as_ref()?
                    .container_status
                    .as_ref()?
                    .container_id
                    .clone()
            }) {
                let doc = api.inspect_container(&container).await.unwrap();
                if doc
                    .pointer("/State/Health/Status")
                    .and_then(serde_json::Value::as_str)
                    == Some("healthy")
                {
                    healthy = true;
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
        assert!(healthy, "the service never became healthy");
    });
    let outcome = futures_util::FutureExt::catch_unwind(checks).await;
    let _ = api.delete_service(&name).await;
    for _ in 0..30 {
        if api.delete_config(&config_id).await.is_ok() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    for _ in 0..30 {
        let volume_gone = api
            .remove_volume(&volume_name(&project, "app", "data"))
            .await
            .is_ok();
        let network_gone = api.remove_network(&network_name(&project)).await.is_ok();
        if volume_gone && network_gone {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}

/// A build on the real engine goes through BuildKit (`RUN --mount`, as
/// Nixpacks writes).
#[tokio::test]
async fn a_real_build_uses_buildkit() {
    if std::env::var_os("PANEL_TEST_DOCKER").is_none() {
        return;
    }
    use rdockpanel::services::docker_service::BuildLine;
    let docker = Docker::new(BollardDocker::connect().expect("docker client"));
    let dockerfile = "FROM busybox:1.37\nRUN --mount=type=cache,target=/cache echo built > /built\nCOPY hello.txt /hello.txt\n";
    let mut tar = tar::Builder::new(Vec::new());
    for (name, body) in [("Dockerfile", dockerfile), ("hello.txt", "hi\n")] {
        let mut header = tar::Header::new_gnu();
        header.set_size(body.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        tar.append_data(&mut header, name, body.as_bytes()).unwrap();
    }
    let tag = "rdp/test_build:d1";
    let mut lines = docker
        .engine()
        .build_image(tar.into_inner().unwrap(), "Dockerfile", tag);
    let (mut image, mut log) = (None, vec![]);
    while let Some(line) = lines.next().await {
        match line.expect("the build") {
            BuildLine::Image(id) => image = Some(id),
            BuildLine::Log(text) => log.push(text),
        }
    }
    let log = log.join("\n");
    let id = image.unwrap_or_else(|| panic!("no image: {log}"));
    assert!(id.starts_with("sha256:"), "{id}");
    assert!(log.contains("RUN --mount"), "{log}");
    docker.engine().remove_image(tag, true).await.unwrap();

    // A failing step fails the build, with its message.
    let mut tar = tar::Builder::new(Vec::new());
    let body = "FROM busybox:1.37\nRUN exit 3\n";
    let mut header = tar::Header::new_gnu();
    header.set_size(body.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    tar.append_data(&mut header, "Dockerfile", body.as_bytes())
        .unwrap();
    let mut lines =
        docker
            .engine()
            .build_image(tar.into_inner().unwrap(), "Dockerfile", "rdp/test_build:d2");
    let mut failed = None;
    while let Some(line) = lines.next().await {
        if let Err(e) = line {
            failed = Some(e.to_string());
            break;
        }
    }
    assert!(failed.is_some_and(|e| e.contains("exit 3") || e.contains("exit code")));
}
