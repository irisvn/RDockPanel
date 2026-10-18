//! Projects and services in the panel: what may be configured, and what is
//! locked once a service was deployed.

use chrono::Utc;
use rdockpanel::database::migrations::Migrator;
use rdockpanel::models::service;
use rustcms::axum::http::{Method, StatusCode};
use rustcms_testing::{TestApp, TestClient, TestDb, TestResponse, db_test};
use sea_orm::{ActiveModelTrait, EntityTrait, IntoActiveModel, Set};
use serde_json::{Value, json};

use crate::support::{self, fake_docker::FakeDocker};

fn records(app: &TestApp, slug: &str) -> String {
    app.admin_api(&format!("resources/{slug}/records"))
}

fn record(app: &TestApp, slug: &str, key: &str) -> String {
    format!("{}/{key}", records(app, slug))
}

fn key(response: &TestResponse) -> String {
    response.assert_status(StatusCode::CREATED);
    response.json()["data"]["key"].as_str().unwrap().to_owned()
}

/// The field errors of a 422, as text.
fn errors(response: &TestResponse) -> String {
    response.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    let body = response.json();
    format!("{} {}", body["errors"], body["formErrors"])
}

async fn create(app: &TestApp, client: &TestClient<'_>, slug: &str, body: Value) -> TestResponse {
    client.post_json(&records(app, slug), &body).await
}

async fn admin(app: &TestApp) -> TestClient<'_> {
    let user = support::user_with(app, "admin", &[]).await;
    app.make_super_admin(&user).await;
    app.acting_as(&user).await
}

/// A project `shop` with a service `web` (nginx); their keys.
async fn shop(app: &TestApp, client: &TestClient<'_>) -> (String, String) {
    let project = key(&create(app, client, "projects", json!({"name": "shop"})).await);
    let service = key(
        &create(
            app,
            client,
            "services",
            json!({"project_id": project, "name": "web", "image": "nginx:1.27", "replicas": 1, "zero_downtime": true}),
        )
        .await,
    );
    (project, service)
}

db_test!(
    names_images_and_resources_are_checked,
    migrate = Migrator,
    |db: TestDb| async move {
        let app = support::app_with_docker(db, FakeDocker::default()).await;
        let admin = admin(&app).await;

        for bad in ["Shop", "1shop", "shop_", "shop-", ""] {
            assert!(
                errors(&create(&app, &admin, "projects", json!({"name": bad})).await)
                    .contains("name"),
                "{bad}"
            );
        }
        let (project, _) = shop(&app, &admin).await;
        assert!(
            errors(&create(&app, &admin, "projects", json!({"name": "shop"})).await)
                .contains("already taken")
        );

        let service = |body: Value| {
            let mut base = json!({"project_id": project, "name": "api", "image": "ghcr.io/org/api:1", "replicas": 2});
            base.as_object_mut()
                .unwrap()
                .extend(body.as_object().unwrap().clone());
            base
        };
        let refused = [
            (json!({"name": "API"}), "name"),
            (json!({"name": "web"}), "already taken in this project"),
            (json!({"image": "bad image"}), "image"),
            (json!({"replicas": 0}), "replicas"),
            (json!({"replicas": 51}), "replicas"),
            (
                json!({"cpu_limit_millis": 500, "cpu_reservation_millis": 1000}),
                "cannot exceed the limit",
            ),
            (json!({"mem_limit_mb": 2}), "mem_limit_mb"),
            (json!({"healthcheck_retries": 99}), "healthcheck_retries"),
        ];
        for (body, expected) in refused {
            let message = errors(&create(&app, &admin, "services", service(body.clone())).await);
            assert!(message.contains(expected), "{body}: {message}");
        }
        key(&create(
            &app,
            &admin,
            "services",
            service(json!({"cpu_limit_millis": 1000, "mem_limit_mb": 256})),
        )
        .await);
    }
);

db_test!(
    deployed_services_keep_their_names_and_need_destroy,
    migrate = Migrator,
    |db: TestDb| async move {
        let app = support::app_with_docker(db, FakeDocker::default()).await;
        let admin = admin(&app).await;
        let (project, service) = shop(&app, &admin).await;

        // Before any deployment, anything can change; a project with services stays.
        let renamed = admin
            .send(
                Method::PATCH,
                &record(&app, "services", &service),
                Some(&json!({"name": "www"})),
            )
            .await;
        renamed.assert_status(StatusCode::OK);
        admin
            .send(Method::DELETE, &record(&app, "projects", &project), None)
            .await
            .assert_status(StatusCode::FORBIDDEN);

        // Deployed: the Docker objects carry the names.
        let row = service::Entity::find_by_id(service.parse::<i64>().unwrap())
            .one(app.state().db())
            .await
            .unwrap()
            .unwrap();
        let mut active = row.into_active_model();
        active.deployed_at = Set(Some(Utc::now()));
        active.update(app.state().db()).await.unwrap();

        let url = record(&app, "services", &service);
        let patch = |body: Value| {
            let (admin, url) = (&admin, url.clone());
            async move { admin.send(Method::PATCH, &url, Some(&body)).await }
        };
        assert!(
            errors(&patch(json!({"name": "web2"})).await).contains("cannot change once deployed")
        );
        patch(json!({"image": "nginx:1.28", "replicas": 3}))
            .await
            .assert_status(StatusCode::OK);
        let project_patch = json!({"name": "store"});
        let message = errors(
            &admin
                .send(
                    Method::PATCH,
                    &record(&app, "projects", &project),
                    Some(&project_patch),
                )
                .await,
        );
        assert!(
            message.contains("cannot change once a service was deployed"),
            "{message}"
        );
        admin
            .send(Method::DELETE, &record(&app, "services", &service), None)
            .await
            .assert_status(StatusCode::FORBIDDEN);
    }
);

db_test!(
    domains_ports_and_mounts_are_checked,
    migrate = Migrator,
    |db: TestDb| async move {
        let app = support::app_with_docker(db, FakeDocker::default()).await;
        let admin = admin(&app).await;
        let (_, service) = shop(&app, &admin).await;
        let with = |body: Value| {
            let mut base = json!({"service_id": service});
            base.as_object_mut()
                .unwrap()
                .extend(body.as_object().unwrap().clone());
            base
        };

        // Domains: stored lowercase, one service per host and path.
        let domain = create(
            &app,
            &admin,
            "service_domains",
            with(json!({"host": "Shop.Example.COM", "path": "/", "port": 80, "https": true})),
        )
        .await;
        let domain = key(&domain);
        let stored = admin.get(&record(&app, "service_domains", &domain)).await;
        assert_eq!(stored.json()["data"]["data"]["host"], "shop.example.com");
        for (body, expected) in [
            (
                json!({"host": "shop.example.com", "path": "/", "port": 80}),
                "already routed",
            ),
            (
                json!({"host": "localhost", "path": "/", "port": 80}),
                "host",
            ),
            (
                json!({"host": "a.example.com", "path": "/a`) || Host(`x", "port": 80}),
                "path",
            ),
            (
                json!({"host": "a.example.com", "path": "/", "port": 70000}),
                "port",
            ),
        ] {
            let message =
                errors(&create(&app, &admin, "service_domains", with(body.clone())).await);
            assert!(message.contains(expected), "{body}: {message}");
        }
        key(&create(
            &app,
            &admin,
            "service_domains",
            with(json!({"host": "shop.example.com", "path": "/api", "port": 8080})),
        )
        .await);

        // Ports: the proxy's and the panel's are reserved; one per port and protocol.
        let port = |published: i64, protocol: &str| {
            with(
                json!({"published": published, "target": 5432, "protocol": protocol, "mode": "ingress"}),
            )
        };
        assert!(
            errors(&create(&app, &admin, "service_ports", port(443, "tcp")).await)
                .contains("reserved")
        );
        key(&create(&app, &admin, "service_ports", port(5432, "tcp")).await);
        assert!(
            errors(&create(&app, &admin, "service_ports", port(5432, "tcp")).await)
                .contains("already published")
        );
        key(&create(&app, &admin, "service_ports", port(5432, "udp")).await);
        assert!(
            errors(&create(&app, &admin, "service_ports", port(5433, "sctp")).await)
                .contains("protocol")
        );

        // Mounts: volumes by name; host directories need system.manage and an allowed root.
        key(&create(
            &app,
            &admin,
            "service_mounts",
            with(json!({"kind": "volume", "name": "data", "target": "/var/lib/data"})),
        )
        .await);
        for (body, expected) in [
            (
                json!({"kind": "volume", "name": "../x", "target": "/x"}),
                "name",
            ),
            (
                json!({"kind": "volume", "name": "logs", "target": "/"}),
                "target",
            ),
            (
                json!({"kind": "volume", "name": "logs", "target": "/var/lib/data"}),
                "already mounted",
            ),
            (
                json!({"kind": "bind", "source": "/var/run/docker.sock", "target": "/var/run/docker.sock"}),
                "not enabled",
            ),
        ] {
            let message = errors(&create(&app, &admin, "service_mounts", with(body.clone())).await);
            assert!(message.contains(expected), "{body}: {message}");
        }
        let operator = support::user_with(
            &app,
            "operator",
            &[
                "services.view",
                "services.view_any",
                "service_mounts.view_any",
                "service_mounts.view",
                "service_mounts.create",
            ],
        )
        .await;
        let operator = app.acting_as(&operator).await;
        let bind = with(json!({"kind": "bind", "source": "/srv/app", "target": "/srv"}));
        assert!(
            errors(&create(&app, &operator, "service_mounts", bind).await)
                .contains("system.manage")
        );
    }
);

db_test!(
    deployments_are_written_by_the_panel_only,
    migrate = Migrator,
    |db: TestDb| async move {
        let app = support::app_with_docker(db, FakeDocker::default()).await;
        let admin = admin(&app).await;
        let (_, service) = shop(&app, &admin).await;
        let body =
            json!({"service_id": service, "trigger": "deploy", "status": "succeeded", "spec": {}});
        create(&app, &admin, "deployments", body)
            .await
            .assert_status(StatusCode::FORBIDDEN);
        admin
            .get(&records(&app, "deployments"))
            .await
            .assert_status(StatusCode::OK);

        // Someone without the resources' permissions sees none of it.
        let nobody = support::user_with(&app, "nobody", &["docker.view"]).await;
        let nobody = app.acting_as(&nobody).await;
        for slug in ["projects", "services", "service_domains", "deployments"] {
            nobody
                .get(&records(&app, slug))
                .await
                .assert_status(StatusCode::FORBIDDEN);
        }
    }
);
