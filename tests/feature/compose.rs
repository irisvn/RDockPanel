//! Compose services: made by the panel's administrators only, their file
//! checked, joined to the project's network (aliases its domains lead to),
//! run by `docker compose` (a FakeDocker records it).

use std::time::Duration;

use rdockpanel::database::migrations::Migrator;
use rdockpanel::services::deployment_service::{self, Timings};
use rustcms::axum::http::StatusCode;
use rustcms_testing::{TestApp, TestClient, TestDb, db_test};
use serde_json::json;

use crate::deployments::{action, create, last_deployment, records};
use crate::support::{self, fake_docker::FakeDocker};

const FAST: Timings = Timings {
    poll: Duration::from_millis(5),
    settle: Duration::from_millis(20),
    timeout: Duration::from_secs(2),
};

const FILE: &str = "services:\n  web:\n    image: nginx:1.27\n    environment:\n      GREETING: ${GREETING}\n  worker:\n    image: busybox\n    command: sleep 3600\n";

async fn run(app: &TestApp, admin: &TestClient<'_>, name: &str, id: i64) -> String {
    action(app, admin, "services", name, &id.to_string())
        .await
        .assert_status(StatusCode::OK);
    let queued = last_deployment(app, id).await;
    deployment_service::run(app.state(), queued.id, FAST)
        .await
        .unwrap();
    let done = last_deployment(app, id).await;
    assert_eq!(done.status, "succeeded", "{:?} {:?}", done.error, done.log);
    done.log.unwrap_or_default()
}

db_test!(
    a_compose_file_runs_in_its_project,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::default();
        let app = support::app_with_docker(db, fake.clone()).await;
        let user = support::user_with(&app, "admin", &[]).await;
        app.make_super_admin(&user).await;
        let admin = app.acting_as(&user).await;
        let project = create(&app, &admin, "projects", json!({"name": "shop"})).await;
        let url = app.admin_api(&format!("x/apps/projects/{project}/services"));

        // Only the panel's administrators (a Compose file runs anything).
        let maker = support::user_with(&app, "maker", &["projects.view", "services.create"]).await;
        let maker = app.acting_as(&maker).await;
        maker
            .post_json(
                &url,
                &json!({"name": "stack", "kind": "compose", "compose": FILE}),
            )
            .await
            .assert_status(StatusCode::FORBIDDEN);
        admin
            .post_json(
                &url,
                &json!({"name": "bad", "kind": "compose", "compose": "services: []"}),
            )
            .await
            .assert_validation_error("compose");
        let made = admin
            .post_json(
                &url,
                &json!({"name": "stack", "kind": "compose", "compose": FILE}),
            )
            .await;
        made.assert_status(StatusCode::OK);
        let id = made.json()["id"].as_i64().unwrap();

        // A domain names the container it leads to.
        let domain = json!({"service_id": id.to_string(), "host": "stack.localhost", "path": "/", "port": 80, "https": false});
        admin
            .post_json(&records(&app, "service_domains"), &domain)
            .await
            .assert_validation_error("compose_service");
        let mut domain = domain;
        domain["compose_service"] = json!("web");
        create(&app, &admin, "service_domains", domain).await;

        // Its environment: the Compose project's `.env` (placeholders resolved).
        let env = app.admin_api(&format!("x/apps/services/{id}/env"));
        admin
            .send(
                rustcms::axum::http::Method::PUT,
                &env,
                Some(&json!({"text": "GREETING=hello $(PROJECT_NAME)\n"})),
            )
            .await
            .assert_status(StatusCode::OK);

        // Only its administrators change what the file reads.
        let editor = support::user_with(
            &app,
            "editor",
            &["projects.view", "services.view", "services.update"],
        )
        .await;
        let editor = app.acting_as(&editor).await;
        editor
            .send(
                rustcms::axum::http::Method::PUT,
                &env,
                Some(&json!({"text": "GREETING=x\n"})),
            )
            .await
            .assert_status(StatusCode::FORBIDDEN);
        editor
            .send(
                rustcms::axum::http::Method::PATCH,
                &format!("{}/{id}", records(&app, "services")),
                Some(&json!({"compose": FILE})),
            )
            .await
            .assert_status(StatusCode::FORBIDDEN);

        run(&app, &admin, "deploy", id).await;
        let calls = fake.calls();
        assert!(
            calls.contains(&"compose shop_stack up -d --build --remove-orphans".to_owned()),
            "{calls:?}"
        );
        // Force rebuild: built without the cache, pulled, every container made again.
        run(&app, &admin, "rebuild", id).await;
        let calls = fake.calls();
        assert!(
            calls.contains(&"compose shop_stack build --no-cache --pull".to_owned()),
            "{calls:?}"
        );
        assert!(
            calls.contains(
                &"compose shop_stack up -d --build --pull always --force-recreate --remove-orphans"
                    .to_owned()
            )
        );
        let composed = fake.composed.lock().unwrap().clone().unwrap();
        assert!(
            composed.contains("shop_stack_web") && composed.contains("rdp_shop"),
            "{composed}"
        );
        let dir = std::env::current_dir()
            .unwrap()
            .join("storage/compose/shop_stack");
        let dotenv = std::fs::read_to_string(dir.join(".env")).unwrap();
        assert_eq!(dotenv, "GREETING=\"hello shop\"\n");
        // Traefik leads the domain to the container's alias.
        let routes =
            std::fs::read_to_string(support::traefik_dir(&app).join("shop_stack.yml")).unwrap();
        assert!(routes.contains("http://shop_stack_web:80"), "{routes}");

        run(&app, &admin, "stop", id).await;
        assert!(fake.calls().contains(&"compose shop_stack stop".to_owned()));
        run(&app, &admin, "destroy", id).await;
        assert!(
            fake.calls()
                .contains(&"compose shop_stack down --remove-orphans".to_owned())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
);
