//! Docker → Nodes (`/admin/api/x/docker/nodes`): the Swarm's servers listed
//! (whitelisted, labels' secrets hidden), the join command (an audited read),
//! pause/drain/activate (the whole spec kept), removal (a worker that left),
//! and what draining a node does.

use rdockpanel::database::migrations::Migrator;
use rdockpanel::services::docker_service::{SwarmTask, TaskState, TaskStatus};
use rustcms::axum::http::{Method, StatusCode};
use rustcms_testing::{TestApp, TestClient, TestDb, db_test};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter};
use serde_json::{Value, json};

use crate::support::{self, fake_docker::FakeDocker};

/// A node as Docker answers it.
fn node(id: &str, host: &str, role: &str, state: &str, addr: &str) -> Value {
    let mut n = json!({
        "ID": id,
        "Version": { "Index": 10 },
        "Spec": {
            "Role": role,
            "Availability": "active",
            "Labels": { "rdockpanel.main": "true", "db.password": "hunter2" }
        },
        "Description": {
            "Hostname": host,
            "Platform": { "OS": "linux", "Architecture": "x86_64" },
            "Resources": { "NanoCPUs": 4_000_000_000_i64, "MemoryBytes": 8_589_934_592_i64 },
            "Engine": { "EngineVersion": "29.0.0" }
        },
        "Status": { "State": state, "Addr": "203.0.113.10" }
    });
    if role == "manager" {
        n["ManagerStatus"] = json!({ "Leader": true, "Reachability": "reachable", "Addr": addr });
    }
    n
}

/// `node-1` (this panel's, a manager), `node-2` (a worker, ready), `node-3`
/// (a worker that left: down).
fn swarm() -> FakeDocker {
    let fake = FakeDocker::default();
    *fake.nodes.lock().unwrap() = vec![
        node("node-1", "panel", "manager", "ready", "192.168.0.245:2377"),
        node("node-2", "worker-a", "worker", "ready", ""),
        node("node-3", "worker-b", "worker", "down", ""),
    ];
    *fake.join_token.lock().unwrap() = "SWMTKN-1-worker-secret".to_owned();
    fake
}

async fn admin(app: &TestApp) -> TestClient<'_> {
    let user = support::user_with(app, "root", &[]).await;
    app.make_super_admin(&user).await;
    app.acting_as(&user).await
}

fn url(app: &TestApp, path: &str) -> String {
    app.admin_api(&format!("x/docker/nodes{path}"))
}

async fn audited(app: &TestApp, event: &str) -> u64 {
    rustcms::models::audit_log::Entity::find()
        .filter(rustcms::models::audit_log::Column::Event.eq(event))
        .count(app.state().db())
        .await
        .unwrap()
}

db_test!(
    the_nodes_are_listed_with_secret_labels_hidden,
    migrate = Migrator,
    |db: TestDb| async move {
        let app = support::app_with_docker(db, swarm()).await;
        let viewer = support::user_with(&app, "viewer", &["docker.view"]).await;
        let viewer = app.acting_as(&viewer).await;
        let list = viewer.get(&url(&app, "")).await;
        list.assert_status(StatusCode::OK);
        let body = list.json();
        let nodes = body["nodes"].as_array().unwrap();
        assert_eq!(nodes.len(), 3);
        // The panel's node first.
        assert_eq!(nodes[0]["id"], "node-1");
        assert_eq!(nodes[0]["panel"], true);
        assert_eq!(nodes[0]["leader"], true);
        assert_eq!(nodes[1]["role"], "worker");
        assert_eq!(nodes[0]["labels"]["rdockpanel.main"], "true");
        assert!(!list.text().contains("hunter2"), "{}", list.text());
        assert_eq!(body["can_manage"], false);
    }
);

db_test!(
    the_join_command_is_an_audited_read_never_cached,
    migrate = Migrator,
    |db: TestDb| async move {
        let app = support::app_with_docker(db, swarm()).await;
        let admin = admin(&app).await;
        // Without its audit entry, no token.
        let db = app.state().db();
        let rename = |from: &str, to: &str| match db.get_database_backend() {
            sea_orm::DatabaseBackend::MySql => format!("RENAME TABLE {from} TO {to}"),
            _ => format!("ALTER TABLE {from} RENAME TO {to}"),
        };
        db.execute_unprepared(&rename("audit_logs", "audit_logs_off"))
            .await
            .unwrap();
        let refused = admin
            .post_json(&url(&app, "/join-command"), &json!({}))
            .await;
        db.execute_unprepared(&rename("audit_logs_off", "audit_logs"))
            .await
            .unwrap();
        assert!(!refused.status.is_success(), "{}", refused.text());
        assert!(!refused.text().contains("SWMTKN"), "{}", refused.text());
        let read = admin
            .post_json(&url(&app, "/join-command"), &json!({}))
            .await;
        read.assert_status(StatusCode::OK);
        assert_eq!(
            read.json()["command"],
            "docker swarm join --token SWMTKN-1-worker-secret 192.168.0.245:2377"
        );
        let cache = read
            .headers
            .get("cache-control")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        assert!(cache.contains("no-store"), "{cache}");
        assert_eq!(audited(&app, "swarm.join_token.read").await, 1);
        // The audit entry names no token.
        let entries = rustcms::models::audit_log::Entity::find()
            .all(app.state().db())
            .await
            .unwrap();
        assert!(!format!("{entries:?}").contains("SWMTKN"));
    }
);

db_test!(
    a_swarm_on_this_machine_only_gives_no_join_command,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = swarm();
        fake.nodes.lock().unwrap()[0]["ManagerStatus"]["Addr"] = json!("127.0.0.1:2377");
        let app = support::app_with_docker(db, fake).await;
        let admin = admin(&app).await;
        let read = admin
            .post_json(&url(&app, "/join-command"), &json!({}))
            .await;
        read.assert_status(StatusCode::OK);
        assert!(read.json()["command"].is_null(), "{}", read.text());
        assert!(read.json()["why"].as_str().unwrap().contains("127.0.0.1"));
        assert!(!read.text().contains("SWMTKN"));
    }
);

db_test!(
    a_rotated_token_is_recorded_and_never_answered,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = swarm();
        let app = support::app_with_docker(db, fake.clone()).await;
        let admin = admin(&app).await;
        let rotated = admin
            .post_json(&url(&app, "/rotate-token"), &json!({}))
            .await;
        rotated.assert_status(StatusCode::OK);
        assert!(!rotated.text().contains("SWMTKN"), "{}", rotated.text());
        assert!(fake.calls().iter().any(|c| c == "rotate_worker_token"));
        assert_eq!(
            *fake.join_token.lock().unwrap(),
            "SWMTKN-1-worker-secret-rotated"
        );
        assert_eq!(audited(&app, "docker.swarm.rotate_token").await, 1);
    }
);

db_test!(
    draining_a_node_keeps_its_labels_and_role,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = swarm();
        let app = support::app_with_docker(db, fake.clone()).await;
        let admin = admin(&app).await;
        admin
            .send(
                Method::PATCH,
                &url(&app, "/node-2"),
                Some(&json!({"availability": "drain"})),
            )
            .await
            .assert_status(StatusCode::OK);
        let n = fake.nodes.lock().unwrap()[1].clone();
        assert_eq!(n["Spec"]["Availability"], "drain");
        assert_eq!(n["Spec"]["Role"], "worker");
        assert_eq!(n["Spec"]["Labels"]["rdockpanel.main"], "true");
        assert_eq!(n["Version"]["Index"], 11);
        assert!(fake.calls().iter().any(|c| c == "update_node node-2 drain"));
        assert_eq!(audited(&app, "docker.node.drain").await, 1);
        // Back to active.
        admin
            .send(
                Method::PATCH,
                &url(&app, "/node-2"),
                Some(&json!({"availability": "active"})),
            )
            .await
            .assert_status(StatusCode::OK);
        assert_eq!(
            fake.nodes.lock().unwrap()[1]["Spec"]["Availability"],
            "active"
        );
        // Not a word Swarm knows.
        admin
            .send(
                Method::PATCH,
                &url(&app, "/node-2"),
                Some(&json!({"availability": "off"})),
            )
            .await
            .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    }
);

db_test!(
    the_panels_own_node_is_never_paused_drained_or_removed,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = swarm();
        let app = support::app_with_docker(db, fake.clone()).await;
        let admin = admin(&app).await;
        for availability in ["drain", "pause"] {
            let refused = admin
                .send(
                    Method::PATCH,
                    &url(&app, "/node-1"),
                    Some(&json!({"availability": availability})),
                )
                .await;
            assert_eq!(refused.status, StatusCode::CONFLICT, "{}", refused.text());
        }
        let refused = admin
            .send(Method::DELETE, &url(&app, "/node-1"), None)
            .await;
        assert_eq!(refused.status, StatusCode::CONFLICT, "{}", refused.text());
        assert!(
            !fake
                .calls()
                .iter()
                .any(|c| c.starts_with("update_node") || c.starts_with("remove_node")),
            "{:?}",
            fake.calls()
        );
        assert_eq!(
            fake.nodes.lock().unwrap()[0]["Spec"]["Availability"],
            "active"
        );
    }
);

db_test!(
    only_a_worker_that_left_is_removed,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = swarm();
        fake.nodes.lock().unwrap().push(node(
            "node-4",
            "manager-b",
            "manager",
            "down",
            "198.51.100.4:2377",
        ));
        let app = support::app_with_docker(db, fake.clone()).await;
        let admin = admin(&app).await;
        // Still in the Swarm.
        let ready = admin
            .send(Method::DELETE, &url(&app, "/node-2"), None)
            .await;
        assert_eq!(ready.status, StatusCode::CONFLICT, "{}", ready.text());
        assert!(ready.text().contains("swarm leave"), "{}", ready.text());
        // A manager.
        let manager = admin
            .send(Method::DELETE, &url(&app, "/node-4"), None)
            .await;
        assert_eq!(manager.status, StatusCode::CONFLICT, "{}", manager.text());
        assert!(!fake.calls().iter().any(|c| c.starts_with("remove_node")));
        // Gone.
        admin
            .send(Method::DELETE, &url(&app, "/nowhere"), None)
            .await
            .assert_status(StatusCode::NOT_FOUND);
        // A worker that left.
        admin
            .send(Method::DELETE, &url(&app, "/node-3"), None)
            .await
            .assert_status(StatusCode::OK);
        assert!(fake.calls().iter().any(|c| c == "remove_node node-3"));
        assert_eq!(fake.nodes.lock().unwrap().len(), 3);
        assert_eq!(audited(&app, "docker.node.remove").await, 4);
    }
);

db_test!(
    draining_says_what_is_tied_to_the_node_and_keeps_data_there,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = swarm();
        *fake.swarm_services.lock().unwrap() = vec![
            json!({"ID": "svc-web", "Spec": {"Name": "shop_web", "Labels": {"rdockpanel.managed": "true"},
                "TaskTemplate": {"ContainerSpec": {"Mounts": [{"Type": "volume", "Source": "rdp_shop_web_data"}]}}}}),
            json!({"ID": "svc-cache", "Spec": {"Name": "shop_cache", "Labels": {"rdockpanel.managed": "true"},
                "TaskTemplate": {"Placement": {"Constraints": ["node.id == node-2"]}, "ContainerSpec": {}}}}),
            json!({"ID": "svc-other", "Spec": {"Name": "elsewhere_app", "Labels": {}, "TaskTemplate": {"ContainerSpec": {}}}}),
        ];
        let task = |service: &str| SwarmTask {
            id: Some(format!("t-{service}")),
            service_id: Some(service.to_owned()),
            node_id: Some("node-2".to_owned()),
            desired_state: Some(TaskState::RUNNING),
            status: Some(TaskStatus {
                state: Some(TaskState::RUNNING),
                ..Default::default()
            }),
            ..Default::default()
        };
        *fake.node_task_list.lock().unwrap() =
            vec![task("svc-web"), task("svc-cache"), task("svc-other")];
        let app = support::app_with_docker(db, fake.clone()).await;
        let admin = admin(&app).await;
        let impact = admin.get(&url(&app, "/node-2/impact")).await;
        impact.assert_status(StatusCode::OK);
        let services = impact.json()["services"].as_array().unwrap().clone();
        let of = |name: &str| services.iter().find(|s| s["name"] == name).unwrap().clone();
        assert_eq!(of("shop_web")["volumes"], true);
        assert_eq!(of("shop_web")["tied"], false);
        assert_eq!(of("shop_cache")["tied"], true);
        assert_eq!(of("elsewhere_app")["panel"], false);
        // The list counts the tasks there.
        let list = admin.get(&url(&app, "")).await.json();
        let worker = list["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["id"] == "node-2")
            .unwrap()
            .clone();
        assert_eq!(worker["tasks"], 3);
        // A viewer may not see what runs where.
        let viewer = support::user_with(&app, "viewer", &["docker.view"]).await;
        let viewer = app.acting_as(&viewer).await;
        viewer
            .get(&url(&app, "/node-2/impact"))
            .await
            .assert_status(StatusCode::FORBIDDEN);
    }
);

db_test!(
    a_node_changed_meanwhile_or_an_unknown_panel_node_changes_nothing,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = swarm();
        let app = support::app_with_docker(db, fake.clone()).await;
        let admin = admin(&app).await;
        // Another change won: refused, said.
        *fake.node_update_conflict.lock().unwrap() = true;
        let stale = admin
            .send(
                Method::PATCH,
                &url(&app, "/node-2"),
                Some(&json!({"availability": "pause"})),
            )
            .await;
        assert_eq!(stale.status, StatusCode::CONFLICT, "{}", stale.text());
        assert!(stale.text().contains("node_changed"), "{}", stale.text());
        assert_eq!(
            fake.nodes.lock().unwrap()[1]["Spec"]["Availability"],
            "active"
        );
        // Docker does not say which node the panel is on: nothing is asked of it.
        *fake.node_id.lock().unwrap() = Some(String::new());
        let before = fake.calls().len();
        for (method, path, body) in [
            (
                Method::PATCH,
                "/node-2",
                Some(json!({"availability": "drain"})),
            ),
            (Method::DELETE, "/node-3", None),
        ] {
            let refused = admin.send(method, &url(&app, path), body.as_ref()).await;
            assert!(!refused.status.is_success(), "{}", refused.text());
        }
        assert_eq!(fake.calls().len(), before, "{:?}", fake.calls());
    }
);

db_test!(
    a_node_that_left_with_tasks_still_assigned_is_kept,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = swarm();
        *fake.node_task_list.lock().unwrap() = vec![SwarmTask {
            id: Some("t1".to_owned()),
            service_id: Some("svc".to_owned()),
            node_id: Some("node-3".to_owned()),
            desired_state: Some(TaskState::RUNNING),
            ..Default::default()
        }];
        let app = support::app_with_docker(db, fake.clone()).await;
        let admin = admin(&app).await;
        let refused = admin
            .send(Method::DELETE, &url(&app, "/node-3"), None)
            .await;
        assert_eq!(refused.status, StatusCode::CONFLICT, "{}", refused.text());
        assert!(!fake.calls().iter().any(|c| c.starts_with("remove_node")));
    }
);

db_test!(
    a_viewer_does_not_count_the_panels_own_tasks_and_no_token_is_kept,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = swarm();
        *fake.swarm_services.lock().unwrap() = vec![
            json!({"ID": "svc-panel", "Spec": {"Name": "rdockpanel", "Labels": {"rdockpanel.system": "true"}}}),
            json!({"ID": "svc-web", "Spec": {"Name": "shop_web", "Labels": {"rdockpanel.managed": "true"}}}),
        ];
        let running = |service: &str| SwarmTask {
            id: Some(format!("t-{service}")),
            service_id: Some(service.to_owned()),
            node_id: Some("node-1".to_owned()),
            desired_state: Some(TaskState::RUNNING),
            status: Some(TaskStatus {
                state: Some(TaskState::RUNNING),
                ..Default::default()
            }),
            ..Default::default()
        };
        *fake.node_task_list.lock().unwrap() = vec![
            running("svc-panel"),
            running("svc-web"),
            running("svc-unknown"),
        ];
        let app = support::app_with_docker(db, fake.clone()).await;
        let viewer = support::user_with(&app, "viewer", &["docker.view"]).await;
        let viewer = app.acting_as(&viewer).await;
        let seen = viewer.get(&url(&app, "")).await.json();
        assert_eq!(seen["nodes"][0]["tasks"], 1, "{seen}");
        let admin = admin(&app).await;
        let all = admin.get(&url(&app, "")).await.json();
        assert_eq!(all["nodes"][0]["tasks"], 3, "{all}");
        // Joining and rotating leave no token in the actions or the audit log.
        admin
            .post_json(&url(&app, "/join-command"), &json!({}))
            .await
            .assert_status(StatusCode::OK);
        admin
            .post_json(&url(&app, "/rotate-token"), &json!({}))
            .await
            .assert_status(StatusCode::OK);
        let actions = rdockpanel::models::action::Entity::find()
            .all(app.state().db())
            .await
            .unwrap();
        let audit = rustcms::models::audit_log::Entity::find()
            .all(app.state().db())
            .await
            .unwrap();
        assert!(!format!("{actions:?}{audit:?}").contains("SWMTKN"));
    }
);
