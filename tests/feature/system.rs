//! Settings → Panel's controls (`/admin/api/x/apps/system`): the panel's
//! restart and update check, the server's reboot, Docker's prunes — each
//! logged as an action and audited, the disruptive ones before they start.

use rdockpanel::database::migrations::Migrator;
use rdockpanel::models::action;
use rdockpanel::services::action_service::{self, Initiator};
use rdockpanel::services::docker_service::{Docker, SYSTEM_LABEL, SwarmService, VolumeItem};
use rdockpanel::services::system_service;
use rustcms::audit::AuditContext;
use rustcms::axum::http::StatusCode;
use rustcms_testing::{TestDb, db_test};
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};
use serde_json::{Value, json};
use std::collections::HashMap;

use crate::support::{self, fake_docker::FakeDocker};

const IMAGE: &str = "ghcr.io/irisvn/rdockpanel:main";

fn volume(name: &str, label: Option<&str>) -> VolumeItem {
    VolumeItem {
        name: name.to_owned(),
        driver: "local".to_owned(),
        mountpoint: String::new(),
        labels: label
            .map(|l| HashMap::from([(l.to_owned(), "x".to_owned())]))
            .unwrap_or_default(),
    }
}

/// The installer's panel service, running `IMAGE` at digest `aaa`.
fn installed(docker: &FakeDocker) {
    let spec = json!({
        "Name": "rdockpanel",
        "Labels": {SYSTEM_LABEL: "true"},
        "TaskTemplate": {"ContainerSpec": {"Image": format!("{IMAGE}@sha256:aaa")}},
    });
    docker.services.lock().unwrap().insert(
        "rdockpanel".to_owned(),
        SwarmService {
            spec: Some(serde_json::from_value(spec).unwrap()),
            ..Default::default()
        },
    );
}

async fn rows(app: &rustcms_testing::TestApp, kind: &str) -> Vec<action::Model> {
    action::Entity::find()
        .filter(action::Column::Kind.eq(kind))
        .all(app.state().db())
        .await
        .unwrap()
}

async fn audited(app: &rustcms_testing::TestApp, event: &str) -> u64 {
    rustcms::models::audit_log::Entity::find()
        .filter(rustcms::models::audit_log::Column::Event.eq(event))
        .count(app.state().db())
        .await
        .unwrap()
}

db_test!(
    from_source_and_docker_prunes,
    migrate = Migrator,
    |db: TestDb| async move {
        let docker = FakeDocker::default();
        docker.volumes.lock().unwrap().extend([
            volume("scratch", None),
            volume("rdp_shop_db_data", None),
            volume("shop_web_cache", Some("com.docker.compose.project")),
            volume("panel-data", Some(SYSTEM_LABEL)),
            volume("busy", None),
        ]);
        docker.mounted.lock().unwrap().push("busy".to_owned());
        let calls = docker.calls.clone();
        let app = support::app_with_docker(db, docker).await;
        let user = support::user_with(&app, "admin", &[]).await;
        app.make_super_admin(&user).await;
        let admin = app.acting_as(&user).await;
        let api = |p: &str| app.admin_api(&format!("x/apps/system/{p}"));

        // Run from source: no service to restart, no server to reboot.
        let status = admin.get(&api("status")).await;
        status.assert_status(StatusCode::OK);
        let status = status.json();
        assert_eq!(status["installed"], false);
        assert_eq!(status["service"], false);
        assert_eq!(status["can_reboot"], false);
        assert!(status["version"].is_string());
        admin
            .post_json(&api("restart"), &json!({}))
            .await
            .assert_validation_error("panel");
        admin
            .post_json(&api("reboot"), &json!({"confirm": status["hostname"]}))
            .await
            .assert_validation_error("server");
        assert_eq!(
            admin.post_json(&api("update"), &json!({})).await.json()["installed"],
            false
        );
        assert!(
            calls
                .lock()
                .unwrap()
                .iter()
                .all(|c| !c.starts_with("host "))
        );

        // Volumes: only the unused one nobody keeps.
        let pruned = admin.post_json(&api("prune/volumes"), &json!({})).await;
        pruned.assert_status(StatusCode::OK);
        assert_eq!(pruned.json()["report"]["volumes_deleted"], 1);
        let removed: Vec<String> = calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| c.starts_with("remove-volume"))
            .cloned()
            .collect();
        assert_eq!(removed, ["remove-volume scratch"]);
        admin
            .post_json(&api("prune/images"), &json!({}))
            .await
            .assert_status(StatusCode::OK);
        admin
            .post_json(&api("prune/build_cache"), &json!({}))
            .await
            .assert_status(StatusCode::OK);
        let calls_now = calls.lock().unwrap().clone();
        assert!(
            calls_now.contains(&format!("prune Images {SYSTEM_LABEL}")),
            "{calls_now:?}"
        );
        assert!(calls_now.contains(&format!("prune BuildCache {SYSTEM_LABEL}")));
        admin
            .post_json(&api("prune/everything"), &json!({}))
            .await
            .assert_status(StatusCode::NOT_FOUND);
        assert_eq!(
            rows(&app, "system.prune.volumes").await[0].status,
            action::OK
        );
        assert_eq!(audited(&app, "docker.system.prune.volumes").await, 1);
    }
);

db_test!(
    installed_restart_reboot_and_updates,
    migrate = Migrator,
    |db: TestDb| async move {
        let docker = FakeDocker::default();
        installed(&docker);
        let calls = docker.calls.clone();
        let registry = docker.registry.clone();
        let app = support::app_with_docker(db, docker).await;
        let state = app.state();
        let docker = state.service::<Docker>().unwrap();
        let by = Initiator {
            user_id: None,
            audit: AuditContext::system(None),
        };

        let status = system_service::status(state, Some(&docker), true).await;
        assert_eq!(status["service"], true);
        assert_eq!(status["image"], format!("{IMAGE}@sha256:aaa"));

        // Restart: logged and audited, then the service replaced.
        system_service::restart(state, &docker, &by, true)
            .await
            .unwrap();
        assert!(
            calls
                .lock()
                .unwrap()
                .contains(&"update service rdockpanel".to_owned())
        );
        let spec = docker
            .engine()
            .inspect_service("rdockpanel")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            spec.spec.unwrap().task_template.unwrap().force_update,
            Some(1)
        );
        let restart = rows(&app, "panel.restart").await;
        assert_eq!(restart[0].status, action::RUNNING);
        assert_eq!(audited(&app, "system.panel.restart").await, 1);
        // The panel came back: done.
        assert_eq!(
            action_service::came_back(state.db(), &system_service::DISRUPTIVE)
                .await
                .unwrap(),
            1
        );
        assert_eq!(rows(&app, "panel.restart").await[0].status, action::OK);

        // Reboot: the hostname typed, then a host command of the panel's image.
        let host = system_service::hostname(state);
        assert!(!host.is_empty());
        let wrong = system_service::reboot(state, &docker, &by, "not-it", true).await;
        assert!(wrong.is_err());
        assert!(rows(&app, "server.reboot").await.is_empty());
        system_service::reboot(state, &docker, &by, &host, true)
            .await
            .unwrap();
        let host_call = calls
            .lock()
            .unwrap()
            .iter()
            .find(|c| c.starts_with("host "))
            .cloned()
            .unwrap();
        assert!(
            host_call.starts_with(&format!("host {IMAGE}@sha256:aaa nsenter -t 1 ")),
            "{host_call}"
        );
        assert_eq!(rows(&app, "server.reboot").await[0].status, action::RUNNING);
        assert_eq!(audited(&app, "system.server.reboot").await, 1);

        // Updates: as the registry serves the tag now; unknown when it does not say.
        let update = system_service::check_update(&docker, true).await.unwrap();
        assert_eq!(update["update"], Value::Null);
        assert_eq!(update["image"], IMAGE);
        registry
            .lock()
            .unwrap()
            .insert(IMAGE.to_owned(), "sha256:aaa".to_owned());
        assert_eq!(
            system_service::check_update(&docker, true).await.unwrap()["update"],
            false
        );
        registry
            .lock()
            .unwrap()
            .insert(IMAGE.to_owned(), "sha256:bbb".to_owned());
        let update = system_service::check_update(&docker, true).await.unwrap();
        assert_eq!(update["update"], true);
        assert!(update["command"].as_str().unwrap().contains("install.sh"));
    }
);
