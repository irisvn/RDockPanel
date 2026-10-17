//! The panel's rules over Docker: its own objects (label
//! `rust-panel.system=true`) are hidden and untouchable without the system
//! scope, prune skips them, inspect hides secrets. With `PANEL_TEST_DOCKER=1`
//! the last test runs against the real engine.

use futures_util::StreamExt;
use rust_panel::services::docker_service::{
    BollardDocker, ContainerAction, Docker, DockerError, LogQuery, SYSTEM_LABEL, Scope,
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
        .push(rust_panel::services::docker_service::NetworkItem {
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

    let name = format!("rust-panel-test-{}", std::process::id());
    let status = std::process::Command::new("docker")
        .args([
            "run",
            "-d",
            "--name",
            &name,
            "--label",
            "rust-panel.test=1",
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
