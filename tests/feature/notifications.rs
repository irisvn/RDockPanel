//! Notification channels: written through their resource (the secret sealed,
//! never shown), "Send test notification", and a deployment said to a
//! webhook (a small HTTP server of the test receives it).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use rdockpanel::database::migrations::Migrator;
use rdockpanel::services::deployment_service::{self, Timings};
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
