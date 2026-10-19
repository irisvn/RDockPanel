//! Notification channels: written through their resource (the secret sealed,
//! never shown), "Send test notification", and a deployment said to a
//! webhook (a small HTTP server of the test receives it).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use rdockpanel::database::migrations::Migrator;
use rdockpanel::services::deployment_service::{self, Timings};
use rdockpanel::services::docker_service::TaskState;
use rustcms::axum::http::StatusCode;
use rustcms_testing::{TestDb, db_test};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::deployments::{action, create, last_deployment, records};
use crate::support::{self, fake_docker::FakeDocker};

const FAST: Timings = Timings {
    poll: Duration::from_millis(5),
    settle: Duration::from_millis(20),
    timeout: Duration::from_secs(2),
};

/// An HTTP server on 127.0.0.1 keeping each request's JSON body.
async fn receiver() -> (String, Arc<Mutex<Vec<Value>>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/hook", listener.local_addr().unwrap());
    let got = Arc::new(Mutex::new(vec![]));
    let keep = got.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let keep = keep.clone();
            tokio::spawn(async move {
                let mut buf = vec![];
                let mut chunk = [0u8; 4096];
                loop {
                    let n = socket.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    let text = String::from_utf8_lossy(&buf).into_owned();
                    if let Some((head, body)) = text.split_once("\r\n\r\n") {
                        let length = head
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                            })
                            .unwrap_or(0);
                        if body.len() >= length {
                            if let Ok(json) = serde_json::from_str(body) {
                                keep.lock().unwrap().push(json);
                            }
                            break;
                        }
                    }
                }
                let _ = socket
                    .write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")
                    .await;
            });
        }
    });
    (url, got)
}

async fn wait_for(got: &Arc<Mutex<Vec<Value>>>, event: &str) -> Value {
    for _ in 0..200 {
        if let Some(found) = got.lock().unwrap().iter().find(|v| v["event"] == event) {
            return found.clone();
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("no {event} notification: {:?}", got.lock().unwrap());
}

db_test!(
    channels_are_told_what_happened,
    migrate = Migrator,
    |db: TestDb| async move {
        let app = support::app_with_docker(db, FakeDocker::default()).await;
        let user = support::user_with(&app, "admin", &[]).await;
        app.make_super_admin(&user).await;
        let admin = app.acting_as(&user).await;
        let (url, got) = receiver().await;

        // A webhook needs its URL; Telegram its chat; email its server.
        admin
            .post_json(
                &records(&app, "notification_channels"),
                &json!({"name": "d", "kind": "discord"}),
            )
            .await
            .assert_validation_error("secret");
        admin
            .post_json(
                &records(&app, "notification_channels"),
                &json!({"name": "t", "kind": "telegram", "secret": {"replace": "1:abc"}}),
            )
            .await
            .assert_validation_error("chat_id");
        let mail = admin
            .post_json(
                &records(&app, "notification_channels"),
                &json!({"name": "m", "kind": "email"}),
            )
            .await;
        mail.assert_validation_error("smtp_host");
        mail.assert_validation_error("email_to");
        let id = create(
            &app,
            &admin,
            "notification_channels",
            json!({"name": "ops", "kind": "webhook", "secret": {"replace": url}, "on_backup": false}),
        )
        .await;

        // Listed without its URL.
        let list = admin
            .get(&app.admin_api("x/apps/notification-channels"))
            .await;
        list.assert_status(StatusCode::OK);
        assert!(!list.text().contains("/hook"), "{}", list.text());
        assert_eq!(list.json()["channels"][0]["secret_set"], true);

        // The test button sends now.
        let tested = admin
            .post_json(
                &app.admin_api(&format!("x/apps/notification-channels/{id}/test")),
                &json!({}),
            )
            .await;
        tested.assert_status(StatusCode::OK);
        assert_eq!(tested.json()["ok"], true, "{}", tested.text());
        assert_eq!(wait_for(&got, "test").await["ok"], true);

        // A deployment: said once it finished.
        let project = create(&app, &admin, "projects", json!({"name": "shop"})).await;
        let service = create(
            &app,
            &admin,
            "services",
            json!({"project_id": project, "name": "web", "image": "nginx:1.27", "replicas": 1}),
        )
        .await;
        action(&app, &admin, "services", "deploy", &service)
            .await
            .assert_status(StatusCode::OK);
        let queued = last_deployment(&app, service.parse().unwrap()).await;
        deployment_service::run(app.state(), queued.id, FAST)
            .await
            .unwrap();
        let said = wait_for(&got, "deploy").await;
        assert_eq!(said["title"], "deploy of shop/web succeeded");
        assert_eq!(said["data"]["status"], "succeeded");

        // Its secret never goes elsewhere: another kind needs it again.
        admin
            .send(
                rustcms::axum::http::Method::PATCH,
                &format!("{}/{id}", records(&app, "notification_channels")),
                Some(&json!({"kind": "email", "smtp_host": "evil.example", "smtp_from": "a@b.c", "email_to": "d@e.f"})),
            )
            .await
            .assert_validation_error("secret");

        // A channel that cannot be reached says why (never its URL).
        let dead = create(
            &app,
            &admin,
            "notification_channels",
            json!({"name": "dead", "kind": "slack", "secret": {"replace": "http://127.0.0.1:9/secret-path"}}),
        )
        .await;
        let failed = admin
            .post_json(
                &app.admin_api(&format!("x/apps/notification-channels/{dead}/test")),
                &json!({}),
            )
            .await
            .json();
        assert_eq!(failed["ok"], false);
        assert!(
            !failed["error"].as_str().unwrap().contains("secret-path"),
            "{failed}"
        );
    }
);

async fn list_id(
    app: &rustcms_testing::TestApp,
    admin: &rustcms_testing::TestClient<'_>,
    name: &str,
) -> i64 {
    let list = admin
        .get(&app.admin_api("x/apps/notification-channels"))
        .await
        .json();
    list["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == name)
        .unwrap()["id"]
        .as_i64()
        .unwrap()
}

db_test!(
    the_panels_alerts_reach_the_channels_that_hear_them,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::default();
        let app = support::app_with_docker(db, fake.clone()).await;
        let user = support::user_with(&app, "admin", &[]).await;
        app.make_super_admin(&user).await;
        let admin = app.acting_as(&user).await;
        let (url, got) = receiver().await;
        let channels = records(&app, "notification_channels");

        // Unknown events refused; a channel set before `events` hears its
        // choices and the panel's alerts.
        admin
            .post_json(&channels, &json!({"name": "x", "kind": "webhook", "secret": {"replace": url}, "events": "[\"everything\"]"}))
            .await
            .assert_validation_error("events");
        create(&app, &admin, "notification_channels", json!({"name": "legacy", "kind": "webhook", "secret": {"replace": format!("{url}?legacy")}, "on_backup": false})).await;
        create(&app, &admin, "notification_channels", json!({"name": "ops", "kind": "webhook", "secret": {"replace": url}, "events": "[\"service\",\"prune\",\"login\"]"})).await;
        // An older client's flag still counts, in step with the events.
        let ops = list_id(&app, &admin, "ops").await;
        let on = admin
            .send(
                rustcms::axum::http::Method::PATCH,
                &format!("{channels}/{ops}"),
                Some(&json!({"on_deploy": true})),
            )
            .await;
        assert!(on.status.is_success(), "{}", on.text());
        let seen = admin
            .get(&app.admin_api("x/apps/notification-channels"))
            .await
            .json();
        let ops_events = seen["channels"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == "ops")
            .unwrap()["events"]
            .clone();
        assert_eq!(ops_events, json!(["service", "prune", "login", "deploy"]));
        admin
            .send(
                rustcms::axum::http::Method::PATCH,
                &format!("{channels}/{ops}"),
                Some(&json!({"events": "[\"service\",\"prune\",\"login\"]"})),
            )
            .await;
        let list = admin
            .get(&app.admin_api("x/apps/notification-channels"))
            .await
            .json();
        let legacy = list["channels"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == "legacy")
            .unwrap();
        assert_eq!(
            legacy["events"],
            json!([
                "deploy",
                "disk",
                "certificate",
                "service",
                "traefik",
                "update"
            ])
        );

        // A running service whose tasks fail: said once, then its return.
        let project = create(&app, &admin, "projects", json!({"name": "shop"})).await;
        let web: i64 = create(
            &app,
            &admin,
            "services",
            json!({"project_id": project, "name": "web", "image": "nginx:1.27", "replicas": 1}),
        )
        .await
        .parse()
        .unwrap();
        let deployed = crate::deployments::run(&app, &admin, "deploy", web).await;
        assert_eq!(deployed.status, "succeeded", "{:?}", deployed.error);
        let alerts = |got: &Arc<Mutex<Vec<Value>>>, event: &str| {
            got.lock()
                .unwrap()
                .iter()
                .filter(|v| v["event"] == event)
                .cloned()
                .collect::<Vec<_>>()
        };
        rdockpanel::services::alert_service::check_health(app.state())
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(alerts(&got, "service").is_empty());
        *fake.task_state.lock().unwrap() = Some(TaskState::FAILED);
        rdockpanel::services::alert_service::check_health(app.state())
            .await
            .unwrap();
        // Failing at one check only (a task being replaced): not yet.
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(alerts(&got, "service").is_empty());
        rdockpanel::services::alert_service::check_health(app.state())
            .await
            .unwrap();
        let down = wait_for(&got, "service").await;
        assert_eq!(down["data"]["down"], true, "{down}");
        assert_eq!(down["data"]["service"], "web");
        rdockpanel::services::alert_service::check_health(app.state())
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        // Both channels hear it once (the legacy one too): no repeat.
        assert_eq!(alerts(&got, "service").len(), 2);
        *fake.task_state.lock().unwrap() = None;
        rdockpanel::services::alert_service::check_health(app.state())
            .await
            .unwrap();
        for _ in 0..100 {
            if alerts(&got, "service").len() == 4 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let back = alerts(&got, "service");
        assert_eq!(back.len(), 4, "{back:?}");
        assert_eq!(back[3]["data"]["down"], false);

        // A prune: only the channel that asked.
        admin
            .post_json(&app.admin_api("x/apps/system/prune/images"), &json!({}))
            .await
            .assert_status(StatusCode::OK);
        wait_for(&got, "prune").await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(alerts(&got, "prune").len(), 1);

        // Sign-ins: from an address new for its user (the first read only starts).
        use rustcms::models::audit_log;
        use sea_orm::{ActiveModelTrait, Set};
        rdockpanel::services::alert_service::check_logins(app.state())
            .await
            .unwrap();
        let mut n = 0u128;
        let mut signed_in = |ip: &str, at: chrono::DateTime<Utc>| {
            n += 1;
            audit_log::ActiveModel {
                id: Set(sea_orm::prelude::Uuid::from_u128(n)),
                at: Set(at),
                event: Set("auth.signed_in".to_owned()),
                subject_type: Set("session".to_owned()),
                actor_kind: Set("user".to_owned()),
                actor_user_id: Set(Some(user.id)),
                ip: Set(Some(ip.to_owned())),
                ..Default::default()
            }
        };
        let now = Utc::now();
        for row in [
            signed_in("10.0.0.1", now - chrono::TimeDelta::hours(1)),
            signed_in("10.0.0.1", now + chrono::TimeDelta::seconds(1)),
            signed_in("203.0.114.9", now + chrono::TimeDelta::seconds(2)),
        ] {
            row.insert(app.state().db()).await.unwrap();
        }
        rdockpanel::services::alert_service::check_logins(app.state())
            .await
            .unwrap();
        let login = wait_for(&got, "login").await;
        assert_eq!(login["data"]["ip"], "203.0.114.9", "{login}");
        rdockpanel::services::alert_service::check_logins(app.state())
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(alerts(&got, "login").len(), 1);
    }
);
