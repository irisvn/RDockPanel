//! Settings → Panel's controls (`/admin/api/x/apps/system`): the panel's
//! restart, the server's reboot, Docker's prunes; Settings → Version (its
//! history, the Dashboard's notice, old versions cleaned up) — each
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
        // From source: its version shown, no check asked (no registry).
        let version = admin.get(&api("version")).await.json();
        assert_eq!(version["rdockpanel"]["installed"], false);
        assert_eq!(version["rdockpanel"]["channel"], "source");
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
    installed_restart_and_reboot,
    migrate = Migrator,
    |db: TestDb| async move {
        let docker = FakeDocker::default();
        installed(&docker);
        let calls = docker.calls.clone();
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
        let host = system_service::hostname(state).await;
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
    }
);

fn container(
    id: &str,
    service: Option<&str>,
    image: &str,
    state: &str,
    created: i64,
) -> rdockpanel::services::docker_service::ContainerItem {
    let mut labels = HashMap::new();
    if let Some(service) = service {
        labels.insert(SYSTEM_LABEL.to_owned(), "true".to_owned());
        labels.insert(
            "com.docker.swarm.service.name".to_owned(),
            service.to_owned(),
        );
    }
    rdockpanel::services::docker_service::ContainerItem {
        id: id.to_owned(),
        name: id.to_owned(),
        image: image.to_owned(),
        image_id: image.to_owned(),
        state: state.to_owned(),
        status: String::new(),
        created,
        ports: vec![],
        labels,
    }
}

fn image(id: &str, tag: &str) -> rdockpanel::services::docker_service::ImageItem {
    rdockpanel::services::docker_service::ImageItem {
        id: id.to_owned(),
        tags: vec![tag.to_owned()],
        size: 300_000_000,
        created: 0,
        containers: -1,
        labels: HashMap::new(),
    }
}

db_test!(
    old_versions_go_but_the_running_and_previous_ones,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::default();
        *fake.containers.lock().unwrap() = vec![
            container("run", Some("rdockpanel"), "sha256:new", "running", 300),
            container("restarted", Some("rdockpanel"), "sha256:new", "exited", 250),
            container("previous", Some("rdockpanel"), "sha256:prev", "exited", 200),
            container("old", Some("rdockpanel"), "sha256:old", "exited", 100),
            container("theirs", None, "sha256:theirs", "exited", 50),
            // Nothing runs (failing): its newest version and the one before kept.
            container("t-new", Some("rdp-traefik"), "sha256:t-new", "exited", 300),
            container(
                "t-prev",
                Some("rdp-traefik"),
                "sha256:t-prev",
                "exited",
                200,
            ),
            container("t-old", Some("rdp-traefik"), "sha256:t-old", "exited", 100),
        ];
        *fake.images.lock().unwrap() = vec![
            image("sha256:new", "ghcr.io/irisvn/rdockpanel:0.5.0"),
            image("sha256:prev", "ghcr.io/irisvn/rdockpanel:0.4.2"),
            image("sha256:old", "ghcr.io/irisvn/rdockpanel:0.4.1"),
            image("sha256:theirs", "nginx:1.27"),
            image("sha256:t-new", "traefik:v3.7"),
            image("sha256:t-prev", "traefik:v3.5"),
            image("sha256:t-old", "traefik:v3.4"),
        ];
        let app = support::app_with_docker(db, fake.clone()).await;
        let docker = Docker::new(fake.clone());

        // Each image's containers by state: running, or kept by stopped ones.
        let usage: HashMap<String, Option<(usize, usize)>> = docker
            .images_with_use()
            .await
            .unwrap()
            .into_iter()
            .map(|(i, u)| (i.id, u))
            .collect();
        assert_eq!(usage["sha256:new"], Some((1, 1)));
        assert_eq!(usage["sha256:prev"], Some((0, 1)));
        assert_eq!(usage["sha256:theirs"], Some((0, 1)));

        // Said first: only the old tasks' containers and their images.
        let said = docker.system_cleanup(false).await.unwrap();
        assert_eq!(said.containers, 2);
        let mut ids: Vec<_> = said
            .images
            .iter()
            .map(|i| i["id"].as_str().unwrap().to_owned())
            .collect();
        ids.sort();
        assert_eq!(ids, ["sha256:old", "sha256:t-old"]);
        assert!(
            fake.calls
                .lock()
                .unwrap()
                .iter()
                .all(|c| !c.starts_with("remove"))
        );

        // By an administrator only, then removed (never forced).
        let user = support::user_with(&app, "operator", &["server.view"]).await;
        app.acting_as(&user)
            .await
            .post_json(&app.admin_api("x/apps/system/cleanup"), &json!({}))
            .await
            .assert_status(StatusCode::FORBIDDEN);
        let root = support::user_with(&app, "root", &[]).await;
        app.make_super_admin(&root).await;
        let admin = app.acting_as(&root).await;
        let done = admin
            .post_json(&app.admin_api("x/apps/system/cleanup"), &json!({}))
            .await;
        done.assert_status(StatusCode::OK);
        let calls = fake.calls.lock().unwrap().clone();
        assert!(calls.contains(&"remove old".to_owned()), "{calls:?}");
        assert!(
            calls.contains(&"remove-image sha256:old force=false".to_owned()),
            "{calls:?}"
        );
        for kept in [
            "remove run",
            "remove restarted",
            "remove previous",
            "remove theirs",
            "remove t-new",
            "remove t-prev",
        ] {
            assert!(!calls.contains(&kept.to_owned()), "{kept}: {calls:?}");
        }
        assert!(calls.contains(&"remove t-old".to_owned()), "{calls:?}");
        assert!(
            !calls
                .iter()
                .any(|c| c.contains("sha256:prev") || c.contains("sha256:new"))
        );
        assert_eq!(rows(&app, "system.cleanup").await[0].status, action::OK);
    }
);

db_test!(
    version_changes_are_the_history_and_notices_are_dismissed,
    migrate = Migrator,
    |db: TestDb| async move {
        use rdockpanel::services::version_service;
        let app = support::app_with_docker(db, FakeDocker::default()).await;
        let state = app.state();
        let db = state.db();
        // The first start: nothing to say; another version: from → to.
        version_service::record_start(db).await.unwrap();
        assert!(rows(&app, "panel.version").await.is_empty());
        rdockpanel::models::panel_setting::set(db, "version.seen", "0.0.1")
            .await
            .unwrap();
        version_service::record_start(db).await.unwrap();
        let changed = rows(&app, "panel.version").await;
        assert_eq!(changed.len(), 1);
        assert!(
            changed[0].target.starts_with("0.0.1 → "),
            "{}",
            changed[0].target
        );
        version_service::record_start(db).await.unwrap();
        assert_eq!(rows(&app, "panel.version").await.len(), 1);

        // Settings → Version: its history; an administrator's only.
        let root = support::user_with(&app, "root", &[]).await;
        app.make_super_admin(&root).await;
        let admin = app.acting_as(&root).await;
        let page = admin.get(&app.admin_api("x/apps/system/version")).await;
        page.assert_status(StatusCode::OK);
        let page = page.json();
        assert_eq!(page["rdockpanel"]["channel"], "source");
        assert_eq!(page["history"][0]["kind"], "panel.version");
        assert_eq!(page["update"], Value::Null);

        // A notice dismissed: that version, by that user.
        admin
            .post_json(
                &app.admin_api("x/apps/system/version/dismiss"),
                &json!({"version": "main"}),
            )
            .await
            .assert_validation_error("version");
        admin
            .post_json(
                &app.admin_api("x/apps/system/version/dismiss"),
                &json!({"version": "9.0.0"}),
            )
            .await
            .assert_status(StatusCode::OK);
        let key = format!("version.dismissed.{}", root.id);
        assert_eq!(
            rdockpanel::models::panel_setting::get(db, &key)
                .await
                .unwrap()
                .as_deref(),
            Some("9.0.0")
        );
        // From source: no notice.
        let notice = admin
            .get(&app.admin_api("x/apps/system/version/notice"))
            .await;
        notice.assert_status(StatusCode::OK);
        assert_eq!(notice.json()["notice"], Value::Null);
    }
);

db_test!(
    nothing_new_starts_while_the_panel_updates,
    migrate = Migrator,
    |db: TestDb| async move {
        use crate::deployments::{action, create};
        use rdockpanel::services::panel_update_service;
        let fake = FakeDocker::default();
        let app = support::app_with_docker(db, fake.clone()).await;
        let root = support::user_with(&app, "root", &[]).await;
        app.make_super_admin(&root).await;
        let admin = app.acting_as(&root).await;
        let project = create(&app, &admin, "projects", json!({"name": "shop"})).await;
        let service = create(
            &app,
            &admin,
            "services",
            json!({"project_id": project, "name": "web", "image": "nginx:1.27", "replicas": 1}),
        )
        .await;
        let db = app.state().db();

        // The gate up: a deployment is refused (it would be cut by the update).
        let until = (chrono::Utc::now() + chrono::Duration::minutes(5)).to_rfc3339();
        rdockpanel::models::panel_setting::set(db, "panel.update.gate", &until)
            .await
            .unwrap();
        assert!(
            panel_update_service::refuse_while_updating(db)
                .await
                .is_err()
        );
        let refused = action(&app, &admin, "services", "deploy", &service).await;
        assert_eq!(refused.status, StatusCode::CONFLICT, "{}", refused.text());
        // Expired (a panel that never came back) or lifted: as before.
        let past = (chrono::Utc::now() - chrono::Duration::minutes(1)).to_rfc3339();
        rdockpanel::models::panel_setting::set(db, "panel.update.gate", &past)
            .await
            .unwrap();
        assert!(
            panel_update_service::refuse_while_updating(db)
                .await
                .is_ok()
        );
        action(&app, &admin, "services", "deploy", &service)
            .await
            .assert_status(StatusCode::OK);

        // The button: an administrator's, a release's panel only (here from
        // source: refused, nothing recorded).
        let operator = support::user_with(&app, "operator", &["server.view"]).await;
        let url = app.admin_api("x/apps/system/version/update");
        app.acting_as(&operator)
            .await
            .post_json(&url, &json!({"version": "9.9.9"}))
            .await
            .assert_status(StatusCode::FORBIDDEN);
        admin
            .post_json(&url, &json!({"version": "9.9.9"}))
            .await
            .assert_validation_error("version");
        assert!(rows(&app, panel_update_service::KIND).await.is_empty());

        // The gate's row: made once, a closed gate kept.
        panel_update_service::ensure_gate(db).await.unwrap();
        panel_update_service::ensure_gate(db).await.unwrap();
        assert_eq!(
            rdockpanel::models::panel_setting::get(db, "panel.update.gate")
                .await
                .unwrap()
                .as_deref(),
            Some(past.as_str())
        );

        // An update's row is the reconciler's: never reaped as abandoned.
        let row = action_service::record(db, panel_update_service::KIND, "0.5.0 → 0.5.1")
            .await
            .unwrap();
        let mut active: action::ActiveModel = row.into();
        active.status = sea_orm::Set(action::RUNNING.to_owned());
        active.updated_at = sea_orm::Set(chrono::Utc::now() - chrono::Duration::hours(3));
        sea_orm::ActiveModelTrait::update(active, db).await.unwrap();
        action_service::reap_stale(db).await.unwrap();
        assert_eq!(
            rows(&app, panel_update_service::KIND).await[0].status,
            action::RUNNING
        );
        // While it runs: no restart nor reboot (they would cut it).
        let by = Initiator {
            user_id: None,
            audit: AuditContext::new(
                rustcms::audit::AuditActor::system(None),
                rustcms::audit::RequestMeta::default(),
            ),
        };
        assert!(
            panel_update_service::announce_disruptive(db, &by, "panel.restart", "rdockpanel")
                .await
                .is_err()
        );
    }
);

/// With `PANEL_TEST_SWARM=1`: the Update's service spec on a real Swarm — a
/// new image that never becomes healthy is rolled back to the previous one.
#[tokio::test]
async fn an_update_that_never_becomes_healthy_is_rolled_back() {
    if std::env::var_os("PANEL_TEST_SWARM").is_none() {
        return;
    }
    let _swarm = crate::support::swarm_lock().await;
    use rdockpanel::services::docker_service::BollardDocker;
    use rdockpanel::services::panel_update_service::{Timing, update_spec_with};
    let docker = Docker::new(BollardDocker::connect().expect("docker client"));
    let api = docker.engine();
    let name = format!("t{}_update", std::process::id());
    let good = "nginx:1.27-alpine";
    let spec = json!({
        "Name": name,
        "Labels": {"rdockpanel.managed": "true"},
        "TaskTemplate": {"ContainerSpec": {
            "Image": good,
            "HealthCheck": {
                "Test": ["CMD-SHELL", "wget -qO- http://127.0.0.1/ >/dev/null || exit 1"],
                "Interval": 2_000_000_000i64, "Retries": 3, "StartPeriod": 2_000_000_000i64,
            },
        }},
        "Mode": {"Replicated": {"Replicas": 1}},
    });
    api.create_service(serde_json::from_value(spec).unwrap(), None)
        .await
        .unwrap();
    let checks = std::panic::AssertUnwindSafe(async {
        let state_and_image = || async {
            let service = api.inspect_service(&name).await.unwrap().unwrap();
            let state = service
                .update_status
                .and_then(|u| u.state)
                .map(|s| s.to_string());
            let image = service
                .spec
                .and_then(|s| s.task_template)
                .and_then(|t| t.container_spec)
                .and_then(|c| c.image)
                .unwrap_or_default();
            (state, image)
        };
        // Healthy first.
        let mut up = false;
        for _ in 0..90 {
            let containers = api.containers().await.unwrap();
            if containers.iter().any(|c| {
                c.labels.get("com.docker.swarm.service.name") == Some(&name)
                    && c.status.contains("(healthy)")
            }) {
                up = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
        assert!(up, "the first version became healthy");
        // Moved to an image whose health check never passes (no web server).
        let service = api.inspect_service(&name).await.unwrap().unwrap();
        let version = service.version.and_then(|v| v.index).unwrap();
        let current = serde_json::to_value(service.spec.unwrap()).unwrap();
        let timing = Timing {
            interval: 2,
            retries: 3,
            start_period: 2,
            monitor: 20,
        };
        let mut next = update_spec_with(current, "busybox:1.36", &timing);
        next["TaskTemplate"]["ContainerSpec"]["Args"] = json!(["sleep", "600"]);
        api.update_service(&name, version, serde_json::from_value(next).unwrap(), None)
            .await
            .unwrap();
        let mut ended = (None, String::new());
        for _ in 0..180 {
            ended = state_and_image().await;
            if ended.0.as_deref() == Some("rollback_completed") {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
        assert_eq!(ended.0.as_deref(), Some("rollback_completed"), "{ended:?}");
        assert!(ended.1.starts_with(good), "back on {}", ended.1);
        assert_eq!(
            rdockpanel::services::panel_update_service::outcome(
                ended.0.as_deref(),
                "0.5.0",
                "0.5.1"
            ),
            Some(Err(
                "the new version did not start: rolled back to 0.5.0".to_owned()
            ))
        );
    })
    .catch_unwind();
    use futures_util::FutureExt;
    let result = checks.await;
    let _ = api.delete_service(&name).await;
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}
