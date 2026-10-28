//! Changes recorded with their audit entry: when the entry cannot be
//! written, the change is not made (firewall, certificates, API tokens)
//! and no secret is answered (the API's environment read).

use rdockpanel::database::migrations::Migrator;
use rdockpanel::models::{certificate, firewall_blocked, firewall_opening};
use rustcms::auth::entities::personal_access_token as token;
use rustcms::axum::http::{Method, StatusCode};
use rustcms_testing::{TestApp, TestDb, db_test};
use sea_orm::{ConnectionTrait, EntityTrait, PaginatorTrait};
use serde_json::json;

use crate::certificates::issued;
use crate::deployments::create;
use crate::support::{self, fake_docker::FakeDocker};

async fn audit_log(app: &TestApp, on: bool) {
    let (from, to) = if on {
        ("audit_logs_off", "audit_logs")
    } else {
        ("audit_logs", "audit_logs_off")
    };
    let db = app.state().db();
    let sql = match db.get_database_backend() {
        sea_orm::DatabaseBackend::MySql => format!("RENAME TABLE {from} TO {to}"),
        _ => format!("ALTER TABLE {from} RENAME TO {to}"),
    };
    db.execute_unprepared(&sql).await.unwrap();
}

db_test!(
    no_change_without_its_audit_entry,
    migrate = Migrator,
    |db: TestDb| async move {
        let app = support::app_with_docker(db, FakeDocker::default()).await;
        let root = support::user_with(&app, "root", &[]).await;
        app.make_super_admin(&root).await;
        let admin = app.acting_as(&root).await;
        let project = create(&app, &admin, "projects", json!({"name": "shop"})).await;
        let web = create(
            &app,
            &admin,
            "services",
            json!({"project_id": project, "name": "web", "image": "nginx:1.27", "replicas": 1}),
        )
        .await;
        admin
            .send(
                Method::PUT,
                &app.admin_api(&format!("x/apps/services/{web}/env")),
                Some(&json!({"text": "TOKEN=s3cr3t-value\n"})),
            )
            .await
            .assert_status(StatusCode::OK);
        let reader = app
            .with_token(&root, &["services.view", "services.secrets"])
            .await;
        let blocked = admin
            .post_json(
                &app.admin_api("x/apps/firewall/blocked"),
                &json!({"cidr": "198.51.100.7"}),
            )
            .await;
        blocked.assert_status(StatusCode::OK);
        let blocked = blocked.json()["id"].as_i64().unwrap();
        let (chain, key) = issued(&["shop.example.com"]);
        let cert = admin
            .post_json(
                &app.admin_api("x/apps/certificates"),
                &json!({"chain": chain, "key": key}),
            )
            .await;
        cert.assert_status(StatusCode::OK);
        let cert = cert.json()["id"].as_i64().unwrap();
        let db = app.state().db();
        let tokens = token::Entity::find().count(db).await.unwrap();

        audit_log(&app, false).await;
        let failed = |status: StatusCode| assert!(status.is_server_error(), "{status}");
        failed(
            admin
                .post_json(
                    &app.admin_api("x/apps/firewall/openings"),
                    &json!({"ports": "8080", "protocol": "tcp", "target": "published", "anywhere": true}),
                )
                .await
                .status,
        );
        failed(
            admin
                .send(
                    Method::DELETE,
                    &app.admin_api(&format!("x/apps/firewall/blocked/{blocked}")),
                    None,
                )
                .await
                .status,
        );
        failed(
            admin
                .send(
                    Method::DELETE,
                    &app.admin_api(&format!("x/apps/certificates/{cert}")),
                    None,
                )
                .await
                .status,
        );
        failed(
            admin
                .post_json(
                    &app.admin_api("x/apps/tokens"),
                    &json!({"name": "ci", "scopes": ["read"], "days": 30}),
                )
                .await
                .status,
        );
        let read = reader.get(&format!("/api/v1/services/{web}/env")).await;
        failed(read.status);
        assert!(!read.text().contains("s3cr3t-value"));
        audit_log(&app, true).await;

        // Nothing changed.
        assert_eq!(firewall_opening::Entity::find().count(db).await.unwrap(), 0);
        assert!(
            firewall_blocked::Entity::find_by_id(blocked)
                .one(db)
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            certificate::Entity::find_by_id(cert)
                .one(db)
                .await
                .unwrap()
                .is_some()
        );
        assert_eq!(token::Entity::find().count(db).await.unwrap(), tokens);

        // Recorded again: the same changes go through, each with its entry.
        admin
            .post_json(
                &app.admin_api("x/apps/tokens"),
                &json!({"name": "ci", "scopes": ["read"], "days": 30}),
            )
            .await
            .assert_status(StatusCode::OK);
        let events: Vec<String> = rustcms::models::audit_log::Entity::find()
            .all(db)
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.event)
            .collect();
        assert!(
            events.iter().any(|e| e == "api_tokens.create"),
            "{events:?}"
        );
    }
);
