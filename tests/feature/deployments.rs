//! Deployments: asked for by a service's actions (one at a time), run
//! against Docker (here a FakeDocker), and how they end.

use std::time::Duration;

use chrono::Utc;
use rdockpanel::database::migrations::Migrator;
use rdockpanel::models::{deployment, service};
use rdockpanel::services::deployment_service::{self, Timings};
use rdockpanel::services::docker_service::{ServiceUpdateStatusStateEnum, TaskState};
use rustcms::axum::http::StatusCode;
use rustcms_testing::{TestApp, TestClient, TestDb, TestResponse, db_test};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, PaginatorTrait, QueryFilter,
    QueryOrder, Set,
};
use serde_json::{Value, json};

use crate::support::{self, fake_docker::FakeDocker};

pub(crate) const FAST: Timings = Timings {
    poll: Duration::from_millis(5),
    settle: Duration::from_millis(20),
    timeout: Duration::from_secs(2),
};

/// What the `deploy.publish` job a save queues does (no worker runs in
/// tests): the service's routes written again, as saved.
pub(crate) async fn publish_routes(app: &TestApp, id: i64) {
    let db = app.state().db();
    let jobs = rustcms::models::job::Entity::find()
        .filter(rustcms::models::job::Column::Kind.eq("deploy.publish"))
        .count(db)
        .await
        .unwrap();
    assert!(jobs > 0, "a save queues the routes' publication");
    let traefik = app
        .state()
        .service::<rdockpanel::services::traefik_service::Traefik>()
        .unwrap();
    let row = service::Entity::find_by_id(id)
        .one(db)
        .await
        .unwrap()
        .unwrap();
    rdockpanel::services::traefik_service::republish(
        db,
        &traefik,
        &row,
        &deployment_service::deploy_options(0),
    )
    .await
    .unwrap();
}

pub(crate) fn records(app: &TestApp, slug: &str) -> String {
    app.admin_api(&format!("resources/{slug}/records"))
}

pub(crate) async fn create(
    app: &TestApp,
    client: &TestClient<'_>,
    slug: &str,
    body: Value,
) -> String {
    let response = client.post_json(&records(app, slug), &body).await;
    response.assert_status(StatusCode::CREATED);
    response.json()["data"]["key"].as_str().unwrap().to_owned()
}

pub(crate) async fn action(
    app: &TestApp,
    client: &TestClient<'_>,
    slug: &str,
    name: &str,
    key: &str,
) -> TestResponse {
    let url = app.admin_api(&format!("resources/{slug}/actions/{name}"));
    client.post_json(&url, &json!({"keys": [key]})).await
}

/// A super admin, a project `shop` and its service `web` (nginx, a domain).
async fn shop(app: &TestApp) -> (TestClient<'_>, i64) {
    let user = support::user_with(app, "admin", &[]).await;
    app.make_super_admin(&user).await;
    let admin = app.acting_as(&user).await;
    let project = create(app, &admin, "projects", json!({"name": "shop"})).await;
    let service = create(
        app,
        &admin,
        "services",
        json!({"project_id": project, "name": "web", "image": "nginx:1.27", "replicas": 2, "zero_downtime": true}),
    )
    .await;
    create(app, &admin, "service_domains", json!({"service_id": service, "host": "shop.localhost", "path": "/", "port": 80, "https": false})).await;
    (admin, service.parse().unwrap())
}

async fn service_row(app: &TestApp, id: i64) -> service::Model {
    service::Entity::find_by_id(id)
        .one(app.state().db())
        .await
        .unwrap()
        .unwrap()
}

pub(crate) async fn last_deployment(app: &TestApp, service: i64) -> deployment::Model {
    deployment::Entity::find()
        .filter(deployment::Column::ServiceId.eq(service))
        .order_by_desc(deployment::Column::Id)
        .one(app.state().db())
        .await
        .unwrap()
        .unwrap()
}

/// Asks for `name` on the service, then runs the deployment it queued.
pub(crate) async fn run(
    app: &TestApp,
    admin: &TestClient<'_>,
    name: &str,
    service: i64,
) -> deployment::Model {
    action(app, admin, "services", name, &service.to_string())
        .await
        .assert_status(StatusCode::OK);
    let queued = last_deployment(app, service).await;
    assert_eq!(queued.status, "queued");
    deployment_service::run(app.state(), queued.id, FAST)
        .await
        .unwrap();
    deployment::Entity::find_by_id(queued.id)
        .one(app.state().db())
        .await
        .unwrap()
        .unwrap()
}

/// What the service page's aside (`ServiceLiveSlot`) gets.
async fn aside(app: &TestApp, client: &TestClient<'_>, service: i64) -> Value {
    let url = app.admin_api(&format!("slots?resource=services&key={service}"));
    let response = client.get(&url).await;
    response.assert_status(StatusCode::OK);
    response.json()["data"][0]["content"]["data"].clone()
}

async fn deployment_by_id(app: &TestApp, id: i64) -> deployment::Model {
    deployment::Entity::find_by_id(id)
        .one(app.state().db())
        .await
        .unwrap()
        .unwrap()
}

fn replicas(fake: &FakeDocker, name: &str) -> Option<i64> {
    let services = fake.services.lock().unwrap();
    services
        .get(name)?
        .spec
        .as_ref()?
        .mode
        .as_ref()?
        .replicated
        .as_ref()?
        .replicas
}

db_test!(
    a_service_deploys_stops_starts_restarts_and_is_destroyed,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::default();
        let app = support::app_with_docker(db, fake.clone()).await;
        let (admin, id) = shop(&app).await;

        let deployed = run(&app, &admin, "deploy", id).await;
        assert_eq!(deployed.status, "succeeded", "{:?}", deployed.error);
        assert_eq!(
            deployed.image_digest.as_deref(),
            Some("nginx:1.27@sha256:fake")
        );
        assert!(
            deployed
                .log
                .as_deref()
                .unwrap_or_default()
                .contains("creating shop_web"),
            "{:?}",
            deployed.log
        );
        let row = service_row(&app, id).await;
        assert_eq!(
            (row.status.as_str(), row.current_deployment_id),
            ("running", None)
        );
        assert!(row.deployed_at.is_some() && row.deployed_spec_hash == deployed.spec_hash);
        assert!(
            !deployment_service::has_pending_changes(app.state().db(), &row, app.state().keyring())
                .await
                .unwrap()
        );
        // On the project's network, labelled, at the pinned image.
        {
            let services = fake.services.lock().unwrap();
            let spec = services["shop_web"].spec.clone().unwrap();
            assert_eq!(spec.labels.unwrap()["rdockpanel.project"], "shop");
            let template = spec.task_template.unwrap();
            assert_eq!(
                template.container_spec.unwrap().image.as_deref(),
                Some("nginx:1.27@sha256:fake")
            );
            assert_eq!(
                template.networks.unwrap()[0].target.as_deref(),
                Some("rdp_shop")
            );
        }
        assert!(
            fake.calls()
                .contains(&"create network rdp_shop overlay".to_owned())
        );

        // A change is pending until deployed again.
        let patch = json!({"replicas": 3});
        admin
            .send(
                rustcms::axum::http::Method::PATCH,
                &format!("{}/{id}", records(&app, "services")),
                Some(&patch),
            )
            .await
            .assert_status(StatusCode::OK);
        assert!(
            deployment_service::has_pending_changes(
                app.state().db(),
                &service_row(&app, id).await,
                app.state().keyring()
            )
            .await
            .unwrap()
        );
        assert_eq!(run(&app, &admin, "deploy", id).await.status, "succeeded");
        assert_eq!(replicas(&fake, "shop_web"), Some(3));

        assert_eq!(run(&app, &admin, "stop", id).await.status, "succeeded");
        assert_eq!(replicas(&fake, "shop_web"), Some(0));
        assert_eq!(service_row(&app, id).await.status, "stopped");
        assert_eq!(run(&app, &admin, "start", id).await.status, "succeeded");
        assert_eq!(replicas(&fake, "shop_web"), Some(3));

        let force = |fake: &FakeDocker| {
            fake.services.lock().unwrap()["shop_web"]
                .spec
                .clone()
                .unwrap()
                .task_template
                .unwrap()
                .force_update
                .unwrap_or(0)
        };
        let before = force(&fake);
        assert_eq!(run(&app, &admin, "restart", id).await.status, "succeeded");
        assert_eq!(force(&fake), before + 1);

        assert_eq!(run(&app, &admin, "destroy", id).await.status, "succeeded");
        assert!(!fake.services.lock().unwrap().contains_key("shop_web"));
        let row = service_row(&app, id).await;
        assert_eq!((row.status.as_str(), row.deployed_at), ("new", None));
    }
);

db_test!(
    one_deployment_at_a_time,
    migrate = Migrator,
    |db: TestDb| async move {
        let app = support::app_with_docker(db, FakeDocker::default()).await;
        let (admin, id) = shop(&app).await;
        action(&app, &admin, "services", "deploy", &id.to_string())
            .await
            .assert_status(StatusCode::OK);
        let second = action(&app, &admin, "services", "deploy", &id.to_string()).await;
        assert_eq!(second.status, StatusCode::CONFLICT, "{}", second.text());
        assert_eq!(second.json()["code"], "deployment_in_progress");
        // Only the first was recorded.
        let rows = deployment::Entity::find()
            .filter(deployment::Column::ServiceId.eq(id))
            .all(app.state().db())
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        // A run without the lock touches nothing.
        let first = rows[0].clone();
        let mut row = service_row(&app, id).await.into_active_model();
        row.current_deployment_id = Set(Some(first.id + 1000));
        row.update(app.state().db()).await.unwrap();
        deployment_service::run(app.state(), first.id, FAST)
            .await
            .unwrap();
        let after = last_deployment(&app, id).await;
        assert_eq!(
            (after.status.as_str(), after.error.as_deref()),
            (
                "failed",
                Some("another deployment took over, or the lock expired")
            )
        );
    }
);

db_test!(
    a_run_stops_once_its_lock_is_lost,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::default();
        let app = support::app_with_docker(db, fake.clone()).await;
        let (admin, id) = shop(&app).await;

        // Expired before the run started: it touches nothing.
        action(&app, &admin, "services", "deploy", &id.to_string())
            .await
            .assert_status(StatusCode::OK);
        let late = last_deployment(&app, id).await;
        let mut row = service_row(&app, id).await.into_active_model();
        row.lock_expires_at = Set(Some(Utc::now() - chrono::Duration::seconds(1)));
        row.update(app.state().db()).await.unwrap();
        deployment_service::run(app.state(), late.id, FAST)
            .await
            .unwrap();
        assert_eq!(last_deployment(&app, id).await.status, "failed");
        assert!(!fake.services.lock().unwrap().contains_key("shop_web"));

        // Expired, then taken over by a new request: interrupted at once, and
        // its late run does nothing.
        action(&app, &admin, "services", "deploy", &id.to_string())
            .await
            .assert_status(StatusCode::OK);
        let displaced = last_deployment(&app, id).await;
        let mut row = service_row(&app, id).await.into_active_model();
        row.lock_expires_at = Set(Some(Utc::now() - chrono::Duration::seconds(1)));
        row.update(app.state().db()).await.unwrap();
        let next = run(&app, &admin, "deploy", id).await;
        assert_eq!(next.status, "succeeded", "{:?}", next.error);
        assert_eq!(
            deployment_by_id(&app, displaced.id).await.status,
            "interrupted"
        );
        deployment_service::run(app.state(), displaced.id, FAST)
            .await
            .unwrap();
        assert_eq!(
            deployment_by_id(&app, displaced.id).await.status,
            "interrupted"
        );

        // Taken while it waits for Swarm (tasks never start): it stops there,
        // and leaves the new holder's lock alone.
        *fake.task_state.lock().unwrap() = Some(TaskState::STARTING);
        action(&app, &admin, "services", "deploy", &id.to_string())
            .await
            .assert_status(StatusCode::OK);
        let cut = last_deployment(&app, id).await;
        let steal = async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let mut row = service_row(&app, id).await.into_active_model();
            row.current_deployment_id = Set(Some(cut.id + 1000));
            row.update(app.state().db()).await.unwrap();
        };
        let (ran, ()) = tokio::join!(deployment_service::run(app.state(), cut.id, FAST), steal);
        ran.unwrap();
        let cut = deployment::Entity::find_by_id(cut.id)
            .one(app.state().db())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(cut.status, "failed");
        assert!(
            cut.error
                .as_deref()
                .unwrap_or_default()
                .contains("lock was lost"),
            "{:?}",
            cut.error
        );
        assert_eq!(
            service_row(&app, id).await.current_deployment_id,
            Some(cut.id + 1000)
        );
    }
);

db_test!(
    a_stop_waits_for_every_task_to_end,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::default();
        let app = support::app_with_docker(db, fake.clone()).await;
        let (admin, id) = shop(&app).await;
        assert_eq!(run(&app, &admin, "deploy", id).await.status, "succeeded");
        // A task Swarm wants shut down but that still runs: not stopped yet.
        *fake.lingering.lock().unwrap() = true;
        let stop = run(&app, &admin, "stop", id).await;
        assert_eq!(
            (stop.status.as_str(), stop.error.as_deref()),
            ("failed", Some("timed out waiting for the service"))
        );
        *fake.lingering.lock().unwrap() = false;
        assert_eq!(run(&app, &admin, "stop", id).await.status, "succeeded");
    }
);

db_test!(
    a_service_keeps_its_names_while_deploying,
    migrate = Migrator,
    |db: TestDb| async move {
        let app = support::app_with_docker(db, FakeDocker::default()).await;
        let (admin, id) = shop(&app).await;
        // Its first deployment is queued: not deployed yet, but Docker may
        // get it any moment.
        action(&app, &admin, "services", "deploy", &id.to_string())
            .await
            .assert_status(StatusCode::OK);
        let patch = json!({"name": "api"});
        admin
            .send(
                rustcms::axum::http::Method::PATCH,
                &format!("{}/{id}", records(&app, "services")),
                Some(&patch),
            )
            .await
            .assert_validation_error("name");
        admin
            .send(
                rustcms::axum::http::Method::DELETE,
                &format!("{}/{id}", records(&app, "services")),
                None,
            )
            .await
            .assert_status(StatusCode::FORBIDDEN);
    }
);

db_test!(
    failures_and_rollbacks_end_the_deployment,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::default();
        let app = support::app_with_docker(db, fake.clone()).await;
        let (admin, id) = shop(&app).await;

        // Tasks that keep failing: the deployment fails, the lock is released.
        *fake.task_state.lock().unwrap() = Some(TaskState::FAILED);
        let failed = run(&app, &admin, "deploy", id).await;
        assert_eq!(failed.status, "failed");
        assert!(
            failed
                .error
                .as_deref()
                .unwrap_or_default()
                .contains("exit code 1"),
            "{:?}",
            failed.error
        );
        let row = service_row(&app, id).await;
        assert_eq!(
            (row.status.as_str(), row.current_deployment_id),
            ("failed", None)
        );

        // Healthy, deployed; then an update Swarm rolls back.
        *fake.task_state.lock().unwrap() = None;
        assert_eq!(run(&app, &admin, "deploy", id).await.status, "succeeded");
        *fake.update_state.lock().unwrap() = Some(ServiceUpdateStatusStateEnum::ROLLBACK_COMPLETED);
        let rolled = run(&app, &admin, "restart", id).await;
        assert_eq!(rolled.status, "rolled_back");
        assert_eq!(service_row(&app, id).await.status, "running");
        // Rolled back to a spec without tasks: stopped, whatever was asked.
        let rolled = run(&app, &admin, "stop", id).await;
        assert_eq!(rolled.status, "rolled_back");
        let row = service_row(&app, id).await;
        assert_eq!(
            (row.status.as_str(), row.desired_state.as_str()),
            ("stopped", "stopped")
        );
        // An update Swarm paused: failed.
        *fake.update_state.lock().unwrap() = Some(ServiceUpdateStatusStateEnum::PAUSED);
        assert_eq!(run(&app, &admin, "restart", id).await.status, "failed");
    }
);

db_test!(
    rollback_redeploys_an_earlier_configuration,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::default();
        let app = support::app_with_docker(db, fake.clone()).await;
        let (admin, id) = shop(&app).await;
        let first = run(&app, &admin, "deploy", id).await;
        let patch = json!({"image": "nginx:1.28", "replicas": 1});
        admin
            .send(
                rustcms::axum::http::Method::PATCH,
                &format!("{}/{id}", records(&app, "services")),
                Some(&patch),
            )
            .await
            .assert_status(StatusCode::OK);
        assert_eq!(run(&app, &admin, "deploy", id).await.status, "succeeded");
        assert_eq!(replicas(&fake, "shop_web"), Some(1));

        action(
            &app,
            &admin,
            "deployments",
            "rollback",
            &first.id.to_string(),
        )
        .await
        .assert_status(StatusCode::OK);
        let rollback = last_deployment(&app, id).await;
        assert_eq!(rollback.trigger, "rollback");
        deployment_service::run(app.state(), rollback.id, FAST)
            .await
            .unwrap();
        let image = fake.services.lock().unwrap()["shop_web"]
            .spec
            .clone()
            .unwrap()
            .task_template
            .unwrap()
            .container_spec
            .unwrap()
            .image;
        assert_eq!(image, first.image_digest);
        assert_eq!(replicas(&fake, "shop_web"), Some(2));
        // Only succeeded deployments can be returned to.
        let failed = deployment::ActiveModel {
            status: Set("failed".to_owned()),
            ..rollback.into_active_model()
        };
        let failed = failed.update(app.state().db()).await.unwrap();
        let refused = action(
            &app,
            &admin,
            "deployments",
            "rollback",
            &failed.id.to_string(),
        )
        .await;
        refused.assert_status(StatusCode::OK);
        assert_eq!(refused.json()["denied"], json!([failed.id.to_string()]));

        // Nor a stop (nothing to return to), nor one without a recorded image.
        let stop = run(&app, &admin, "stop", id).await;
        assert_eq!(stop.status, "succeeded");
        let refused = action(
            &app,
            &admin,
            "deployments",
            "rollback",
            &stop.id.to_string(),
        )
        .await;
        assert_eq!(refused.json()["denied"], json!([stop.id.to_string()]));
        let imageless = deployment::ActiveModel {
            image_digest: Set(None),
            ..first.clone().into_active_model()
        }
        .update(app.state().db())
        .await
        .unwrap();
        let refused = action(
            &app,
            &admin,
            "deployments",
            "rollback",
            &imageless.id.to_string(),
        )
        .await;
        assert_eq!(refused.json()["denied"], json!([imageless.id.to_string()]));
    }
);

db_test!(
    a_rollback_uses_the_services_names_now,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::default();
        let app = support::app_with_docker(db, fake.clone()).await;
        let (admin, id) = shop(&app).await;
        let first = run(&app, &admin, "deploy", id).await;
        assert_eq!(run(&app, &admin, "destroy", id).await.status, "succeeded");
        // Destroyed, then renamed: `shop_web` may now be another service's.
        let patch = json!({"name": "api"});
        admin
            .send(
                rustcms::axum::http::Method::PATCH,
                &format!("{}/{id}", records(&app, "services")),
                Some(&patch),
            )
            .await
            .assert_status(StatusCode::OK);
        action(
            &app,
            &admin,
            "deployments",
            "rollback",
            &first.id.to_string(),
        )
        .await
        .assert_status(StatusCode::OK);
        let rollback = last_deployment(&app, id).await;
        deployment_service::run(app.state(), rollback.id, FAST)
            .await
            .unwrap();
        let services = fake.services.lock().unwrap();
        assert!(services.contains_key("shop_api") && !services.contains_key("shop_web"));
    }
);

db_test!(
    runs_cut_short_are_interrupted_at_boot,
    migrate = Migrator,
    |db: TestDb| async move {
        let app = support::app_with_docker(db, FakeDocker::default()).await;
        let (admin, id) = shop(&app).await;
        action(&app, &admin, "services", "deploy", &id.to_string())
            .await
            .assert_status(StatusCode::OK);
        let queued = last_deployment(&app, id).await;
        // Still within its lease: left alone.
        assert_eq!(
            deployment_service::recover_interrupted(app.state().db())
                .await
                .unwrap(),
            0
        );
        let mut row = service_row(&app, id).await.into_active_model();
        row.lock_expires_at = Set(Some(Utc::now() - chrono::Duration::minutes(1)));
        row.update(app.state().db()).await.unwrap();
        assert_eq!(
            deployment_service::recover_interrupted(app.state().db())
                .await
                .unwrap(),
            1
        );
        let after = deployment::Entity::find_by_id(queued.id)
            .one(app.state().db())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after.status, "interrupted");
        let row = service_row(&app, id).await;
        assert_eq!(
            (row.status.as_str(), row.current_deployment_id),
            ("failed", None)
        );
        // A late run of it does nothing.
        deployment_service::run(app.state(), queued.id, FAST)
            .await
            .unwrap();
        assert_eq!(
            deployment::Entity::find_by_id(queued.id)
                .one(app.state().db())
                .await
                .unwrap()
                .unwrap()
                .status,
            "interrupted"
        );
    }
);

db_test!(
    deploying_needs_its_permissions,
    migrate = Migrator,
    |db: TestDb| async move {
        let app = support::app_with_docker(db, FakeDocker::default()).await;
        let (_, id) = shop(&app).await;
        let viewer = support::user_with(
            &app,
            "viewer",
            &["services.view_any", "services.view", "services.update"],
        )
        .await;
        let viewer = app.acting_as(&viewer).await;
        for name in ["deploy", "stop", "destroy"] {
            action(&app, &viewer, "services", name, &id.to_string())
                .await
                .assert_status(StatusCode::FORBIDDEN);
        }
        let deployer = support::user_with(
            &app,
            "deployer",
            &["services.view_any", "services.view", "services.deploy"],
        )
        .await;
        let deployer = app.acting_as(&deployer).await;
        action(&app, &deployer, "services", "deploy", &id.to_string())
            .await
            .assert_status(StatusCode::OK);
        action(&app, &deployer, "services", "destroy", &id.to_string())
            .await
            .assert_status(StatusCode::FORBIDDEN);
        action(&app, &viewer, "services", "republish", &id.to_string())
            .await
            .assert_status(StatusCode::FORBIDDEN);

        // The service's aside: its deployments (log, error) only with their
        // own permission.
        let data = aside(&app, &deployer, id).await;
        assert_eq!(
            (data["latest"].clone(), data["busy"].clone()),
            (Value::Null, json!(true)),
            "{data}"
        );
        let reader = support::user_with(
            &app,
            "reader",
            &["services.view_any", "services.view", "deployments.view"],
        )
        .await;
        let reader = app.acting_as(&reader).await;
        let data = aside(&app, &reader, id).await;
        assert_eq!(data["latest"]["status"], "queued", "{data}");
    }
);

db_test!(
    secrets_are_sealed_and_never_shown,
    migrate = Migrator,
    |db: TestDb| async move {
        const SECRET: &str = "s3cret-7f3a";
        const TOKEN: &str = "token-9b2c";
        let fake = FakeDocker::default();
        let app = support::app_with_docker(db, fake.clone()).await;
        let (admin, id) = shop(&app).await;
        let url = format!("{}/{id}", records(&app, "services"));
        let read = |body: Value| body["data"]["data"]["env"].clone();
        let env = read(admin.get(&url).await.json());
        let revision = env["revision"].clone();

        // A name no shell takes: refused.
        let bad = json!({"env": {"revision": revision, "set": {"1BAD": "x"}}});
        admin
            .send(rustcms::axum::http::Method::PATCH, &url, Some(&bad))
            .await
            .assert_validation_error("env");
        // A private image, its environment and credentials.
        let patch = json!({
            "image": "ghcr.io/acme/web:1",
            "env": {"revision": revision, "set": {"DB_PASSWORD": SECRET, "MODE": "prod"}},
            "registry_username": "bot",
            "registry_password": {"replace": TOKEN},
        });
        let saved = admin
            .send(rustcms::axum::http::Method::PATCH, &url, Some(&patch))
            .await;
        saved.assert_status(StatusCode::OK);
        assert!(!saved.text().contains(SECRET) && !saved.text().contains(TOKEN));
        let shown = admin.get(&url).await.text();
        assert!(!shown.contains(SECRET) && !shown.contains(TOKEN), "{shown}");
        assert!(shown.contains("DB_PASSWORD"), "names are shown: {shown}");

        let first = run(&app, &admin, "deploy", id).await;
        assert_eq!(first.status, "succeeded", "{:?}", first.error);
        // The containers get the values; the pull and the service the credentials.
        let container = |fake: &FakeDocker| {
            fake.services.lock().unwrap()["shop_web"]
                .spec
                .clone()
                .unwrap()
                .task_template
                .unwrap()
                .container_spec
                .unwrap()
        };
        let spec = container(&fake);
        assert_eq!(
            spec.env.clone().unwrap(),
            [format!("DB_PASSWORD={SECRET}"), "MODE=prod".to_owned()]
        );
        let label = &spec.labels.unwrap()["rdockpanel.template"];
        assert!(!label.contains(SECRET));
        let calls = fake.calls();
        assert!(
            calls.contains(&"pull ghcr.io/acme/web:1 as bot@ghcr.io".to_owned()),
            "{calls:?}"
        );
        assert!(
            calls.contains(&"create shop_web as bot@ghcr.io".to_owned()),
            "{calls:?}"
        );
        // The deployment keeps names, never values: not in its spec, its log,
        // what the panel sends, the audit log.
        let row = deployment_by_id(&app, first.id).await;
        let spec_json = row.spec.to_string();
        assert!(!spec_json.contains(SECRET) && spec_json.contains("DB_PASSWORD"));
        assert!(!row.log.clone().unwrap_or_default().contains(SECRET));
        assert!(row.env.is_some());
        let listed = admin.get(&records(&app, "deployments")).await.text();
        assert!(!listed.contains(SECRET) && !listed.contains(TOKEN));
        let audit = rustcms::models::audit_log::Entity::find()
            .all(app.state().db())
            .await
            .unwrap();
        let audit = format!("{audit:?}");
        assert!(!audit.contains(SECRET) && !audit.contains(TOKEN));

        // A new value: a change to deploy, then the containers get it.
        let service = service_row(&app, id).await;
        let keyring = app.state().keyring();
        assert!(
            !deployment_service::has_pending_changes(app.state().db(), &service, keyring)
                .await
                .unwrap()
        );
        let revision = read(admin.get(&url).await.json())["revision"].clone();
        let patch = json!({"env": {"revision": revision, "set": {"DB_PASSWORD": "rotated"}, "keep": ["MODE"]}});
        admin
            .send(rustcms::axum::http::Method::PATCH, &url, Some(&patch))
            .await
            .assert_status(StatusCode::OK);
        assert!(
            deployment_service::has_pending_changes(
                app.state().db(),
                &service_row(&app, id).await,
                keyring
            )
            .await
            .unwrap()
        );
        assert_eq!(run(&app, &admin, "deploy", id).await.status, "succeeded");
        assert!(
            container(&fake)
                .env
                .unwrap()
                .contains(&"DB_PASSWORD=rotated".to_owned())
        );

        // APP_KEY rotated (the old one kept to open values): nothing to deploy.
        let rotated = rustcms::crypto::Keyring::new(
            rustcms::key::AppKey::generate(),
            vec![keyring.current().clone()],
        )
        .unwrap();
        assert!(
            !deployment_service::has_pending_changes(
                app.state().db(),
                &service_row(&app, id).await,
                &rotated
            )
            .await
            .unwrap(),
            "the same environment under a new key is no change"
        );

        // A rollback applies the environment of then.
        action(
            &app,
            &admin,
            "deployments",
            "rollback",
            &first.id.to_string(),
        )
        .await
        .assert_status(StatusCode::OK);
        let rollback = last_deployment(&app, id).await;
        deployment_service::run(app.state(), rollback.id, FAST)
            .await
            .unwrap();
        assert!(
            container(&fake)
                .env
                .unwrap()
                .contains(&format!("DB_PASSWORD={SECRET}"))
        );

        // Credentials go to their registry only: an image moved elsewhere,
        // then rolled back to, is pulled without them.
        let patch =
            json!({"image": "registry.example.com/acme/web:2", "registry_username": "other"});
        admin
            .send(rustcms::axum::http::Method::PATCH, &url, Some(&patch))
            .await
            .assert_status(StatusCode::OK);
        assert_eq!(run(&app, &admin, "deploy", id).await.status, "succeeded");
        assert!(fake.calls().contains(
            &"pull registry.example.com/acme/web:2 as other@registry.example.com".to_owned()
        ));
        action(
            &app,
            &admin,
            "deployments",
            "rollback",
            &first.id.to_string(),
        )
        .await
        .assert_status(StatusCode::OK);
        let back = last_deployment(&app, id).await;
        deployment_service::run(app.state(), back.id, FAST)
            .await
            .unwrap();
        let back = deployment_by_id(&app, back.id).await;
        assert!(
            back.log
                .unwrap_or_default()
                .contains("credentials are for registry.example.com"),
        );
        assert!(
            !fake
                .calls()
                .iter()
                .any(|c| c.starts_with("pull ghcr.io") && c.contains("other")),
            "{:?}",
            fake.calls()
        );

        // Sealed with another key: no deployment, a clear error.
        let mut broken = service_row(&app, id).await.into_active_model();
        broken.env = Set(Some(rustcms::crypto::Encrypted::from_envelope(
            "v1.not-ours".to_owned(),
        )));
        broken.update(app.state().db()).await.unwrap();
        let refused = action(&app, &admin, "services", "deploy", &id.to_string()).await;
        assert_eq!(
            refused.status,
            StatusCode::BAD_REQUEST,
            "{}",
            refused.text()
        );
        assert!(refused.text().contains("APP_KEY"), "{}", refused.text());
        // Destroying needs none of it.
        assert_eq!(run(&app, &admin, "destroy", id).await.status, "succeeded");

        // bollard logs request bodies (the spec, its environment) at debug
        // through `log`: capped at info whatever RUST_LOG says.
        assert!(log::max_level() <= log::LevelFilter::Info);
    }
);

db_test!(
    placeholders_and_files_reach_the_containers,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::default();
        let app = support::app_with_docker(db, fake.clone()).await;
        let (admin, id) = shop(&app).await;
        let env = app.admin_api(&format!("x/apps/services/{id}/env"));
        admin
            .send(
                rustcms::axum::http::Method::PUT,
                &env,
                Some(&json!({"text": "URL=https://$(PRIMARY_DOMAIN)\nDB=$(PROJECT_NAME)_db\nKEEP=$(UNKNOWN)"})),
            )
            .await
            .assert_status(StatusCode::OK);
        create(&app, &admin, "service_mounts", json!({"service_id": id.to_string(), "kind": "file", "target": "/etc/app.ini", "sealed_content": {"replace": "a=1\n"}})).await;
        let deployed = run(&app, &admin, "deploy", id).await;
        assert_eq!(deployed.status, "succeeded", "{:?}", deployed.error);
        let container = |fake: &FakeDocker| {
            fake.services.lock().unwrap()["shop_web"]
                .spec
                .clone()
                .unwrap()
                .task_template
                .unwrap()
                .container_spec
                .unwrap()
        };
        let spec = container(&fake);
        let env = spec.env.clone().unwrap();
        assert!(
            env.contains(&"URL=https://shop.localhost".to_owned()),
            "{env:?}"
        );
        assert!(env.contains(&"DB=shop_db".to_owned()), "{env:?}");
        assert!(env.contains(&"KEEP=$(UNKNOWN)".to_owned()), "{env:?}");
        // The file: a config, mounted read-only at its path.
        let configs = spec.configs.clone().unwrap();
        assert_eq!(configs.len(), 1);
        assert_eq!(
            configs[0].file.as_ref().unwrap().name.as_deref(),
            Some("/etc/app.ini")
        );
        let first = configs[0].config_name.clone().unwrap();
        assert_eq!(fake.configs.lock().unwrap()[&first].1, b"a=1\n");
        // Sealed: never in the deployment's configuration.
        assert!(
            !deployed.spec.to_string().contains("a=1"),
            "{}",
            deployed.spec
        );
        assert!(deployed.files.is_some());

        // A new content: a new config; the old one is removed once unused.
        let mount = service_mount_id(&app, id).await;
        admin
            .send(
                rustcms::axum::http::Method::PATCH,
                &format!("{}/{mount}", records(&app, "service_mounts")),
                Some(&json!({"sealed_content": {"replace": "a=2\n"}})),
            )
            .await
            .assert_status(StatusCode::OK);
        assert_eq!(run(&app, &admin, "deploy", id).await.status, "succeeded");
        let second = container(&fake).configs.unwrap()[0]
            .config_name
            .clone()
            .unwrap();
        assert_ne!(first, second);
        {
            let configs = fake.configs.lock().unwrap();
            assert!(configs.contains_key(&second) && !configs.contains_key(&first));
        }
        // Destroyed: its configs go too.
        assert_eq!(run(&app, &admin, "destroy", id).await.status, "succeeded");
        assert!(fake.configs.lock().unwrap().is_empty());
    }
);

async fn service_mount_id(app: &TestApp, service: i64) -> i64 {
    rdockpanel::models::service_mount::Entity::find()
        .filter(rdockpanel::models::service_mount::Column::ServiceId.eq(service))
        .one(app.state().db())
        .await
        .unwrap()
        .unwrap()
        .id
}

db_test!(
    domains_need_traefik_on_the_network,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::default();
        let app = support::app_with_docker(db, fake.clone()).await;
        fake.services.lock().unwrap().remove("rdp-traefik");
        let (admin, id) = shop(&app).await;
        let failed = run(&app, &admin, "deploy", id).await;
        assert_eq!(failed.status, "failed");
        assert!(
            failed
                .error
                .as_deref()
                .unwrap_or_default()
                .contains("could not join rdp_shop"),
            "{:?}",
            failed.error
        );
        assert!(!fake.services.lock().unwrap().contains_key("shop_web"));
    }
);

db_test!(
    routes_follow_the_deployed_service,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::default();
        let app = support::app_with_docker(db, fake.clone()).await;
        let dir = support::traefik_dir(&app);
        let file = dir.join("shop_web.yml");
        let (admin, id) = shop(&app).await;
        let traefik_updates = |fake: &FakeDocker| {
            fake.calls()
                .iter()
                .filter(|c| *c == "update service rdp-traefik")
                .count()
        };

        let deployed = run(&app, &admin, "deploy", id).await;
        assert_eq!(deployed.status, "succeeded", "{:?}", deployed.error);
        assert!(deployment_by_id(&app, deployed.id).await.published);
        let routes = std::fs::read_to_string(&file).unwrap();
        assert!(routes.contains("Host(`shop.localhost`)"), "{routes}");
        assert!(routes.contains("http://shop_web:80"), "{routes}");
        // Traefik joined the project's network, once, the rest of it kept.
        let traefik = fake.services.lock().unwrap()["rdp-traefik"]
            .spec
            .clone()
            .unwrap();
        assert_eq!(traefik.labels.unwrap()["rdockpanel.system"], "true");
        let template = traefik.task_template.unwrap();
        let targets: Vec<_> = template
            .networks
            .unwrap()
            .into_iter()
            .filter_map(|n| n.target)
            .collect();
        assert_eq!(targets, ["ingress-extra", "rdp_shop"]);
        assert!(template.container_spec.unwrap().args.is_some());
        assert_eq!(run(&app, &admin, "restart", id).await.status, "succeeded");
        assert_eq!(traefik_updates(&fake), 1);

        // A domain added: routed when saved (its job), no deployment; the
        // service has nothing to deploy.
        create(&app, &admin, "service_domains", json!({"service_id": id.to_string(), "host": "www.shop.localhost", "path": "/", "port": 80, "https": true})).await;
        publish_routes(&app, id).await;
        let routes = std::fs::read_to_string(&file).unwrap();
        assert!(
            routes.contains("www.shop.localhost") && routes.contains("websecure"),
            "{routes}"
        );
        assert_eq!(aside(&app, &admin, id).await["pending"], json!(false));

        // A failed deployment (of a change of its image) leaves the routes,
        // as saved, to what still runs.
        create(&app, &admin, "service_domains", json!({"service_id": id.to_string(), "host": "new.shop.localhost", "path": "/", "port": 80, "https": false})).await;
        publish_routes(&app, id).await;
        let changed = admin
            .send(
                rustcms::axum::http::Method::PATCH,
                &format!("{}/{id}", records(&app, "services")),
                Some(&json!({"image": "nginx:1.28"})),
            )
            .await;
        assert!(changed.status.is_success(), "{}", changed.text());
        assert_eq!(aside(&app, &admin, id).await["pending"], json!(true));
        *fake.task_state.lock().unwrap() = Some(TaskState::FAILED);
        assert_eq!(run(&app, &admin, "deploy", id).await.status, "failed");
        let routes = std::fs::read_to_string(&file).unwrap();
        assert!(
            routes.contains("www.shop.localhost") && routes.contains("new.shop.localhost"),
            "{routes}"
        );
        assert!(routes.contains("http://shop_web:80"), "{routes}");
        *fake.task_state.lock().unwrap() = None;
        // Swarm rolled it back: the same.
        *fake.update_state.lock().unwrap() = Some(ServiceUpdateStatusStateEnum::ROLLBACK_COMPLETED);
        assert_eq!(run(&app, &admin, "deploy", id).await.status, "rolled_back");
        assert!(
            std::fs::read_to_string(&file)
                .unwrap()
                .contains("new.shop.localhost")
        );
        *fake.update_state.lock().unwrap() = None;

        // Stopped: not routed; started: routed again; destroyed: gone.
        assert_eq!(run(&app, &admin, "stop", id).await.status, "succeeded");
        assert!(!file.exists());
        assert_eq!(run(&app, &admin, "start", id).await.status, "succeeded");
        assert!(file.exists());

        // Routes that cannot be written: the deployment says so; Republish
        // (a job) writes them once they can be.
        std::fs::remove_dir_all(&dir).unwrap();
        std::fs::write(&dir, "not a directory").unwrap();
        let unpublished = run(&app, &admin, "restart", id).await;
        assert_eq!(unpublished.status, "succeeded");
        assert!(!deployment_by_id(&app, unpublished.id).await.published);
        let aside = aside(&app, &admin, id).await;
        assert_eq!(aside["latest"]["published"], json!(false), "{aside}");
        std::fs::remove_file(&dir).unwrap();
        let publications = || async {
            rustcms::models::job::Entity::find()
                .filter(rustcms::models::job::Column::Kind.eq("deploy.publish"))
                .count(app.state().db())
                .await
                .unwrap()
        };
        let before = publications().await;
        action(&app, &admin, "services", "republish", &id.to_string())
            .await
            .assert_status(StatusCode::OK);
        assert_eq!(publications().await, before + 1);
        let traefik = app
            .state()
            .service::<rdockpanel::services::traefik_service::Traefik>()
            .unwrap();
        // (The job's body.)
        assert!(
            deployment_service::publish(app.state().db(), None, &traefik, id, unpublished.id)
                .await
                .unwrap()
        );
        assert!(file.exists());
        assert!(deployment_by_id(&app, unpublished.id).await.published);

        // At boot: written again from the database, strays removed.
        std::fs::remove_file(&file).unwrap();
        std::fs::write(dir.join("gone_web.yml"), "http: {}").unwrap();
        std::fs::write(dir.join("custom.yml"), "http: {}").unwrap();
        let options = deployment_service::deploy_options(0);
        let written =
            rdockpanel::services::traefik_service::sync_all(app.state().db(), &traefik, &options)
                .await
                .unwrap();
        assert_eq!(written, 1);
        assert!(file.exists() && !dir.join("gone_web.yml").exists());
        assert!(dir.join("custom.yml").exists(), "not a service's: kept");

        assert_eq!(run(&app, &admin, "destroy", id).await.status, "succeeded");
        assert!(!file.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
);

// With `PANEL_TEST_SWARM=1`: the whole flow on a real Swarm, from the
// panel's actions: deploy (nginx, a health check), a rolling update to
// another image, stop, start, destroy.
db_test!(
    deployments_on_a_real_swarm,
    migrate = Migrator,
    |db: TestDb| async move {
        if std::env::var_os("PANEL_TEST_SWARM").is_none() {
            return;
        }
        let _swarm = support::swarm_lock().await;
        let app = support::app(db).await;
        let user = support::user_with(&app, "admin", &[]).await;
        app.make_super_admin(&user).await;
        let admin = app.acting_as(&user).await;
        let project = format!("e{}", std::process::id());
        let project_key = create(&app, &admin, "projects", json!({"name": project})).await;
        let service = create(
        &app,
        &admin,
        "services",
        json!({
            "project_id": project_key, "name": "web", "image": "nginx:1.27-alpine", "replicas": 2,
            "zero_downtime": true, "healthcheck_cmd": "wget -qO- http://127.0.0.1/ >/dev/null", "healthcheck_interval_s": 1,
        }),
    )
    .await;
        let id: i64 = service.parse().unwrap();
        let real = Timings {
            poll: Duration::from_secs(1),
            settle: Duration::from_secs(4),
            timeout: Duration::from_secs(180),
        };
        let name = format!("{project}_web");
        let docker = app
            .state()
            .service::<rdockpanel::services::docker_service::Docker>()
            .unwrap();
        let running = |docker: std::sync::Arc<rdockpanel::services::docker_service::Docker>,
                       name: String| async move {
            let tasks = docker.engine().list_tasks(&name).await.unwrap();
            tasks
                .iter()
                .filter(|t| t.desired_state == Some(TaskState::RUNNING))
                .filter(|t| t.status.as_ref().and_then(|s| s.state) == Some(TaskState::RUNNING))
                .map(|t| {
                    t.spec
                        .as_ref()
                        .unwrap()
                        .container_spec
                        .as_ref()
                        .unwrap()
                        .image
                        .clone()
                        .unwrap_or_default()
                })
                .collect::<Vec<_>>()
        };
        let step = |trigger: &'static str| {
            let (app, admin) = (&app, &admin);
            async move {
                action(app, admin, "services", trigger, &id.to_string())
                    .await
                    .assert_status(StatusCode::OK);
                let queued = last_deployment(app, id).await;
                deployment_service::run(app.state(), queued.id, real)
                    .await
                    .unwrap();
                let done = deployment::Entity::find_by_id(queued.id)
                    .one(app.state().db())
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    done.status,
                    "succeeded",
                    "{trigger}: {:?}\n{}",
                    done.error,
                    done.log.unwrap_or_default()
                );
                done
            }
        };

        let checks = std::panic::AssertUnwindSafe(async {
            let first = step("deploy").await;
            let images = running(docker.clone(), name.clone()).await;
            assert_eq!(images.len(), 2, "{images:?}");
            // Read back with its health check (Docker writes `Healthcheck`,
            // Bollard reads `HealthCheck`): an update from it keeps it.
            let read = docker
                .engine()
                .inspect_service(&name)
                .await
                .unwrap()
                .unwrap();
            let check = read
                .spec
                .and_then(|s| s.task_template)
                .and_then(|t| t.container_spec)
                .and_then(|c| c.health_check);
            assert!(
                check.is_some_and(|c| c
                    .test
                    .unwrap_or_default()
                    .iter()
                    .any(|t| t.contains("wget"))),
                "the health check is lost when read"
            );
            assert!(
                images
                    .iter()
                    .all(|i| i.starts_with("nginx:1.27-alpine@sha256:")),
                "{images:?}"
            );

            let patch = json!({"image": "nginx:1.28-alpine"});
            admin
                .send(
                    rustcms::axum::http::Method::PATCH,
                    &format!("{}/{id}", records(&app, "services")),
                    Some(&patch),
                )
                .await
                .assert_status(StatusCode::OK);
            let second = step("deploy").await;
            assert_ne!(second.image_digest, first.image_digest);
            let images = running(docker.clone(), name.clone()).await;
            assert!(
                images.len() == 2 && images.iter().all(|i| i.starts_with("nginx:1.28-alpine@")),
                "{images:?}"
            );

            step("stop").await;
            assert!(running(docker.clone(), name.clone()).await.is_empty());
            step("start").await;
            assert_eq!(running(docker.clone(), name.clone()).await.len(), 2);
        });
        let outcome = futures_util::FutureExt::catch_unwind(checks).await;
        // Destroy (also as cleanup), then the project's network.
        action(&app, &admin, "services", "destroy", &id.to_string()).await;
        let destroy = last_deployment(&app, id).await;
        let _ = deployment_service::run(app.state(), destroy.id, real).await;
        for _ in 0..30 {
            if docker
                .engine()
                .remove_network(&format!("rdp_{project}"))
                .await
                .is_ok()
            {
                break;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        if let Err(panic) = outcome {
            std::panic::resume_unwind(panic);
        }
        assert_eq!(destroy.trigger, "destroy");
        assert!(
            docker
                .engine()
                .inspect_service(&name)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(service_row(&app, id).await.status, "new");
    }
);

db_test!(
    a_dockerfile_is_built_then_deployed,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::default();
        let app = support::app_with_docker(db, fake.clone()).await;
        let (admin, _) = shop(&app).await;
        let project = service_row(&app, 1).await.project_id.to_string();
        // A Git source needs an https or ssh URL (never a local path).
        let local = admin
            .post_json(
                &records(&app, "services"),
                &json!({"project_id": project, "name": "api", "replicas": 1, "source": "git", "git_url": "/srv/repo"}),
            )
            .await;
        local.assert_validation_error("git_url");
        let id: i64 = create(
            &app,
            &admin,
            "services",
            json!({"project_id": project, "name": "site", "replicas": 1, "source": "dockerfile", "dockerfile": "FROM nginx:1.27\nCOPY . /usr/share/nginx/html\n"}),
        )
        .await
        .parse()
        .unwrap();

        let first = run(&app, &admin, "deploy", id).await;
        assert_eq!(first.status, "succeeded", "{:?}", first.error);
        let tag = format!("rdp/shop_site:d{}", first.id);
        let calls = fake.calls();
        assert!(
            calls.contains(&format!("build {tag} from Dockerfile")),
            "{calls:?}"
        );
        let image = |fake: &FakeDocker| {
            fake.services.lock().unwrap()["shop_site"]
                .spec
                .clone()
                .unwrap()
                .task_template
                .unwrap()
                .container_spec
                .unwrap()
                .image
                .unwrap()
        };
        assert_eq!(image(&fake), tag);
        // A restart keeps the image built (no new build).
        let restarted = run(&app, &admin, "restart", id).await;
        assert_eq!(restarted.status, "succeeded", "{:?}", restarted.error);
        assert_eq!(image(&fake), tag);
        let builds = fake
            .calls()
            .iter()
            .filter(|c| c.starts_with("build "))
            .count();
        assert_eq!(builds, 1);

        // The trigger URL deploys without a session; only its token does.
        let made = admin
            .post_json(
                &app.admin_api(&format!("x/apps/services/{id}/trigger")),
                &json!({}),
            )
            .await;
        made.assert_status(StatusCode::OK);
        let url = made.json()["url"].as_str().unwrap().to_owned();
        let path = &url[url.find("/api/v1/deploy/").unwrap()..];
        let token = path.rsplit_once("?token=").unwrap().1.to_owned();
        assert!(
            !service_row(&app, id)
                .await
                .trigger_hash
                .unwrap()
                .contains(&token)
        );
        for wrong in [
            format!("/api/v1/deploy/{id}?token=nope"),
            format!("/api/v1/deploy/{}?token={token}", id + 1),
            format!("/api/v1/deploy/{id}"),
            format!("/api/v1/deploy/x?token={token}"),
        ] {
            app.post_json(&wrong, &json!({}))
                .await
                .assert_status(StatusCode::NOT_FOUND);
        }
        let fired = app.post_json(path, &json!({})).await;
        fired.assert_status(StatusCode::ACCEPTED);
        let queued = last_deployment(&app, id).await;
        assert_eq!(queued.status, "queued");
        assert_eq!(fired.json()["deployment"], queued.id);
        // One at a time: busy while one waits.
        app.post_json(path, &json!({}))
            .await
            .assert_status(StatusCode::CONFLICT);
        // A new URL: the old one stops working.
        admin
            .post_json(
                &app.admin_api(&format!("x/apps/services/{id}/trigger")),
                &json!({}),
            )
            .await
            .assert_status(StatusCode::OK);
        deployment_service::run(app.state(), queued.id, FAST)
            .await
            .unwrap();
        app.post_json(path, &json!({}))
            .await
            .assert_status(StatusCode::NOT_FOUND);

        // A deploy key: its public half shown, its private half sealed.
        let key = admin
            .post_json(
                &app.admin_api(&format!("x/apps/services/{id}/deploy-key")),
                &json!({}),
            )
            .await;
        key.assert_status(StatusCode::OK);
        let public = key.json()["public_key"].as_str().unwrap().to_owned();
        assert!(public.starts_with("ssh-ed25519 "), "{public}");
        let row = service_row(&app, id).await;
        assert_eq!(row.deploy_key_public.as_deref(), Some(public.as_str()));
        let page = admin
            .get(&app.admin_api(&format!("x/apps/services/{id}")))
            .await
            .text();
        assert!(!page.contains("PRIVATE KEY") && page.contains(&public));
    }
);

db_test!(
    a_deleted_projects_network_goes,
    migrate = Migrator,
    |db: TestDb| async move {
        use rdockpanel::services::docker_service::Docker;
        use rdockpanel::services::traefik_service::{Traefik, remove_orphan_networks};
        let fake = FakeDocker::default();
        let app = support::app_with_docker(db, fake.clone()).await;
        let (admin, id) = shop(&app).await;
        assert_eq!(run(&app, &admin, "deploy", id).await.status, "succeeded");
        let traefik_networks = |fake: &FakeDocker| -> Vec<String> {
            fake.services.lock().unwrap()["rdp-traefik"]
                .spec
                .clone()
                .unwrap()
                .task_template
                .unwrap()
                .networks
                .unwrap_or_default()
                .into_iter()
                .filter_map(|n| n.target)
                .collect()
        };
        assert!(traefik_networks(&fake).contains(&"rdp_shop".to_owned()));
        let docker = app.state().service::<Docker>().unwrap();
        let traefik = app.state().service::<Traefik>();
        // The project still there: its network stays.
        assert_eq!(
            remove_orphan_networks(app.state().db(), &docker, traefik.as_deref())
                .await
                .unwrap(),
            0
        );
        // Another panel's (or a test's) network of a project unknown here: left.
        fake.networks
            .lock()
            .unwrap()
            .push(rdockpanel::services::docker_service::NetworkItem {
                id: "netid-rdp_ghost".to_owned(),
                name: "rdp_ghost".to_owned(),
                driver: "overlay".to_owned(),
                scope: "swarm".to_owned(),
                labels: std::collections::HashMap::from([
                    ("rdockpanel.managed".to_owned(), "true".to_owned()),
                    ("rdockpanel.project".to_owned(), "ghost".to_owned()),
                    ("rdockpanel.instance".to_owned(), "another".to_owned()),
                ]),
            });
        // Destroyed, deleted with its project: Traefik detached, network removed.
        assert_eq!(run(&app, &admin, "destroy", id).await.status, "succeeded");
        let project = service_row(&app, id).await.project_id;
        admin
            .send(
                rustcms::axum::http::Method::DELETE,
                &format!("{}/{id}", records(&app, "services")),
                None,
            )
            .await
            .assert_status(StatusCode::NO_CONTENT);
        admin
            .send(
                rustcms::axum::http::Method::DELETE,
                &format!("{}/{project}", records(&app, "projects")),
                None,
            )
            .await
            .assert_status(StatusCode::NO_CONTENT);
        assert_eq!(
            remove_orphan_networks(app.state().db(), &docker, traefik.as_deref())
                .await
                .unwrap(),
            1
        );
        assert!(!traefik_networks(&fake).contains(&"rdp_shop".to_owned()));
        assert!(
            fake.networks
                .lock()
                .unwrap()
                .iter()
                .any(|n| n.name == "rdp_ghost")
        );
        assert!(
            !fake
                .networks
                .lock()
                .unwrap()
                .iter()
                .any(|n| n.name == "rdp_shop")
        );
    }
);

db_test!(
    domains_carry_their_redirects_and_authentication,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::default();
        let app = support::app_with_docker(db, fake.clone()).await;
        let (admin, id) = shop(&app).await;
        assert_eq!(run(&app, &admin, "deploy", id).await.status, "succeeded");
        let auth = app.admin_api(&format!("x/apps/services/{id}/basic-auth"));
        admin
            .post_json(&auth, &json!({"username": "a b", "password": "short"}))
            .await
            .assert_validation_error("password");
        admin
            .post_json(
                &auth,
                &json!({"username": "ops", "password": "correct horse"}),
            )
            .await
            .assert_status(StatusCode::OK);
        admin
            .post_json(
                &records(&app, "service_redirects"),
                &json!({"service_id": id.to_string(), "regex": "([", "replacement": "x"}),
            )
            .await
            .assert_validation_error("regex");
        create(
            &app,
            &admin,
            "service_redirects",
            json!({"service_id": id.to_string(), "regex": "^http://shop.localhost/old/(.*)", "replacement": "http://shop.localhost/new/${1}", "permanent": true, "enabled": true}),
        )
        .await;
        // Applied when saved (its job): no deployment, nothing to deploy.
        publish_routes(&app, id).await;
        assert_eq!(aside(&app, &admin, id).await["pending"], json!(false));
        let routes =
            std::fs::read_to_string(support::traefik_dir(&app).join("shop_web.yml")).unwrap();
        assert!(
            routes.contains("basicAuth") && routes.contains("ops:$2"),
            "{routes}"
        );
        assert!(
            routes.contains("redirectRegex") && routes.contains("permanent: true"),
            "{routes}"
        );
        assert!(
            routes.contains("shop_web-redirect-0") && routes.contains("shop_web-auth"),
            "{routes}"
        );
        // Removed: gone at once.
        let page = admin
            .get(&app.admin_api(&format!("x/apps/services/{id}")))
            .await
            .json();
        let user = page["basic_auth"][0]["id"].as_i64().unwrap();
        admin
            .send(
                rustcms::axum::http::Method::DELETE,
                &format!("{auth}/{user}"),
                None,
            )
            .await
            .assert_status(StatusCode::NO_CONTENT);
        publish_routes(&app, id).await;
        let routes =
            std::fs::read_to_string(support::traefik_dir(&app).join("shop_web.yml")).unwrap();
        assert!(!routes.contains("basicAuth"), "{routes}");
    }
);

db_test!(
    a_rate_limit_applies_when_saved,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::default();
        let app = support::app_with_docker(db, fake.clone()).await;
        let (admin, id) = shop(&app).await;
        assert_eq!(run(&app, &admin, "deploy", id).await.status, "succeeded");
        let url = format!("{}/{id}", records(&app, "services"));
        let patch = |body: serde_json::Value| {
            let admin = &admin;
            let url = url.clone();
            async move {
                admin
                    .send(rustcms::axum::http::Method::PATCH, &url, Some(&body))
                    .await
            }
        };
        // Refused: out of bounds, another period, Cloudflare's or every address.
        for (body, field) in [
            (json!({"rate_limit_average": 0}), "rate_limit_average"),
            (json!({"rate_limit_burst": 100_001}), "rate_limit_burst"),
            (json!({"rate_limit_period": "1d"}), "rate_limit_period"),
            (
                json!({"rate_limit_exempt": "104.16.0.1"}),
                "rate_limit_exempt",
            ),
            (
                json!({"rate_limit_exempt": "0.0.0.0/0"}),
                "rate_limit_exempt",
            ),
            (json!({"rate_limit_exempt": "nope"}), "rate_limit_exempt"),
        ] {
            patch(body).await.assert_validation_error(field);
        }
        patch(json!({
            "rate_limit_average": 5,
            "rate_limit_period": "1m",
            "rate_limit_burst": 10,
            "rate_limit_exempt": "10.8.0.7/24\n10.8.0.9, 2001:db8::1",
        }))
        .await
        .assert_status(StatusCode::OK);
        // Kept canonical; shown on the service's page.
        let page = admin
            .get(&app.admin_api(&format!("x/apps/services/{id}")))
            .await
            .json();
        assert_eq!(page["service"]["rate_limit_average"], 5);
        assert_eq!(page["service"]["rate_limit_period"], "1m");
        assert_eq!(
            page["service"]["rate_limit_exempt"],
            "10.8.0.0/24\n10.8.0.9/32\n2001:db8::1/128"
        );
        // Applied when saved (its job): no deployment, nothing to deploy.
        publish_routes(&app, id).await;
        assert_eq!(aside(&app, &admin, id).await["pending"], json!(false));
        let read =
            || std::fs::read_to_string(support::traefik_dir(&app).join("shop_web.yml")).unwrap();
        let routes = read();
        assert!(
            routes.contains("shop_web-ratelimit:")
                && routes.contains("shop_web-ratelimit-proxied:"),
            "{routes}"
        );
        assert!(
            routes.contains("period: 1m") && routes.contains("burst: 10"),
            "{routes}"
        );
        assert!(
            routes.contains("ClientIP(`10.8.0.9/32`)")
                && routes.contains("ClientIP(`173.245.48.0/20`)"),
            "{routes}"
        );
        // Off: gone at once.
        patch(json!({"rate_limit_average": null, "rate_limit_exempt": null}))
            .await
            .assert_status(StatusCode::OK);
        publish_routes(&app, id).await;
        let routes = read();
        assert!(
            !routes.contains("ratelimit") && !routes.contains("ClientIP"),
            "{routes}"
        );
    }
);

db_test!(
    a_signed_github_push_deploys,
    migrate = Migrator,
    |db: TestDb| async move {
        use rdockpanel::http::controllers::trigger_controller::signed;
        use rustcms::axum::body::Body;
        use rustcms::axum::http::Request;
        let app = support::app_with_docker(db, FakeDocker::default()).await;
        let (admin, id) = shop(&app).await;
        let made = admin
            .post_json(
                &app.admin_api(&format!("x/apps/services/{id}/github-webhook")),
                &json!({}),
            )
            .await;
        made.assert_status(StatusCode::OK);
        let secret = made.json()["secret"].as_str().unwrap().to_owned();
        let url = format!("/api/v1/github/{id}");
        let sign = |body: &[u8]| {
            use hmac::{KeyInit, Mac};
            let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(secret.as_bytes()).unwrap();
            mac.update(body);
            let hex: String = mac
                .finalize()
                .into_bytes()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            format!("sha256={hex}")
        };
        let send = |event: &str, body: &str, signature: String| {
            Request::post(url.as_str())
                .header("content-type", "application/json")
                .header("x-github-event", event)
                .header("x-hub-signature-256", signature)
                .body(Body::from(body.to_owned()))
                .unwrap()
        };
        assert!(signed(&secret, b"x", &sign(b"x")) && !signed(&secret, b"x", "sha256=00"));
        // Unsigned or wrongly signed: as if it did not exist.
        let push = r#"{"ref": "refs/heads/main", "after": "abc"}"#;
        app.request(send("push", push, "sha256=00".into()))
            .await
            .assert_status(StatusCode::NOT_FOUND);
        app.request(send("ping", "{}", sign(b"{}")))
            .await
            .assert_status(StatusCode::OK);
        // Another branch: ignored.
        let other = r#"{"ref": "refs/heads/dev"}"#;
        let ignored = app
            .request(send("push", other, sign(other.as_bytes())))
            .await;
        ignored.assert_status(StatusCode::OK);
        assert!(ignored.text().contains("ignored"));
        // Its branch (main by default): deployed.
        let fired = app.request(send("push", push, sign(push.as_bytes()))).await;
        fired.assert_status(StatusCode::ACCEPTED);
        let first = last_deployment(&app, id).await;
        assert_eq!(first.status, "queued");
        // Another push meanwhile: deployed again once this one ends.
        let again = app.request(send("push", push, sign(push.as_bytes()))).await;
        again.assert_status(StatusCode::ACCEPTED);
        assert_eq!(again.json()["pending"], true);
        deployment_service::run(app.state(), first.id, FAST)
            .await
            .unwrap();
        let next = last_deployment(&app, id).await;
        assert!(next.id > first.id && next.status == "queued", "{next:?}");
        // A repository's default branch when the service names none.
        let master = r#"{"ref": "refs/heads/master", "repository": {"default_branch": "master"}}"#;
        let on_master = app
            .request(send("push", master, sign(master.as_bytes())))
            .await;
        on_master.assert_status(StatusCode::ACCEPTED);
    }
);

db_test!(
    a_service_in_maintenance_shows_its_page,
    migrate = Migrator,
    |db: TestDb| async move {
        let app = support::app_with_docker(db, FakeDocker::default()).await;
        let (admin, id) = shop(&app).await;
        app.get("/api/v1/maintenance/shop_web")
            .await
            .assert_status(StatusCode::NOT_FOUND);
        admin
            .send(
                rustcms::axum::http::Method::PATCH,
                &format!("{}/{id}", records(&app, "services")),
                Some(&json!({"maintenance": true, "maintenance_title": "<b>Back</b> soon", "maintenance_subtitle": "At 10:00"})),
            )
            .await
            .assert_status(StatusCode::OK);
        // Applied when saved: the page, no deployment.
        let page = app.get("/api/v1/maintenance/shop_web").await;
        page.assert_status(StatusCode::SERVICE_UNAVAILABLE);
        let html = page.text();
        assert!(
            html.contains("&lt;b&gt;Back&lt;/b&gt; soon") && html.contains("At 10:00"),
            "{html}"
        );
        app.get("/api/v1/maintenance/shop_nothing")
            .await
            .assert_status(StatusCode::NOT_FOUND);
    }
);

db_test!(
    a_script_runs_in_a_running_container,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::default();
        let app = support::app_with_docker(db, fake.clone()).await;
        let (admin, id) = shop(&app).await;
        let script = create(
            &app,
            &admin,
            "service_scripts",
            json!({"service_id": id.to_string(), "name": "Migrate", "command": "php artisan migrate --force"}),
        )
        .await;
        let url = app.admin_api(&format!("x/apps/scripts/{script}/run"));
        // Not running yet.
        let shown = json!({"command": "php artisan migrate --force"});
        admin
            .post_json(&url, &shown)
            .await
            .assert_status(StatusCode::BAD_REQUEST);
        assert_eq!(run(&app, &admin, "deploy", id).await.status, "succeeded");
        // Only the command shown.
        admin
            .post_json(&url, &json!({"command": "rm -rf /"}))
            .await
            .assert_status(StatusCode::CONFLICT);
        let ran = admin.post_json(&url, &shown).await;
        ran.assert_status(StatusCode::OK);
        assert_eq!(ran.json()["exit_code"], 0);
        assert!(
            ran.json()["output"]
                .as_str()
                .unwrap()
                .starts_with("dump of ctr-shop_web-")
        );
        assert!(
            fake.calls()
                .iter()
                .any(|c| c.ends_with("sh -c php artisan migrate --force"))
        );
        let page = admin
            .get(&app.admin_api(&format!("x/apps/services/{id}")))
            .await
            .json();
        assert_eq!(page["scripts"][0]["name"], "Migrate");
    }
);

db_test!(
    capabilities_and_sysctls_reach_the_container,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::default();
        let app = support::app_with_docker(db, fake.clone()).await;
        let (admin, id) = shop(&app).await;
        let url = format!("{}/{id}", records(&app, "services"));
        // Adding: the panel's administrators only.
        let editor =
            support::user_with(&app, "editor", &["services.view", "services.update"]).await;
        let editor = app.acting_as(&editor).await;
        editor
            .send(
                rustcms::axum::http::Method::PATCH,
                &url,
                Some(&json!({"cap_add": "NET_ADMIN"})),
            )
            .await
            .assert_validation_error("cap_add");
        editor
            .send(
                rustcms::axum::http::Method::PATCH,
                &url,
                Some(&json!({"cap_drop": "NET_RAW"})),
            )
            .await
            .assert_status(StatusCode::OK);
        admin
            .send(
                rustcms::axum::http::Method::PATCH,
                &url,
                Some(&json!({"sysctls": "not a pair"})),
            )
            .await
            .assert_validation_error("sysctls");
        admin
            .send(
                rustcms::axum::http::Method::PATCH,
                &url,
                Some(&json!({"cap_add": "cap_net_admin, SYS_TIME", "sysctls": "net.ipv4.ip_forward=1", "groups": "video"})),
            )
            .await
            .assert_status(StatusCode::OK);
        assert_eq!(run(&app, &admin, "deploy", id).await.status, "succeeded");
        let spec = fake.services.lock().unwrap()["shop_web"]
            .spec
            .clone()
            .unwrap()
            .task_template
            .unwrap()
            .container_spec
            .unwrap();
        assert_eq!(spec.capability_add.unwrap(), ["NET_ADMIN", "SYS_TIME"]);
        assert_eq!(spec.capability_drop.unwrap(), ["NET_RAW"]);
        assert_eq!(spec.sysctls.unwrap()["net.ipv4.ip_forward"], "1");
        assert_eq!(spec.groups.unwrap(), ["video"]);
        // Now privileged: what it runs is its administrators' to change.
        editor
            .send(
                rustcms::axum::http::Method::PATCH,
                &url,
                Some(&json!({"image": "evil:1"})),
            )
            .await
            .assert_status(StatusCode::FORBIDDEN);
        editor
            .send(
                rustcms::axum::http::Method::PUT,
                &app.admin_api(&format!("x/apps/services/{id}/env")),
                Some(&json!({"text": "X=1\n"})),
            )
            .await
            .assert_status(StatusCode::FORBIDDEN);
        // Nor deployed, nor given a trigger, by anyone else.
        let deployer = support::user_with(
            &app,
            "deployer",
            &["services.view", "services.deploy", "deployments.view"],
        )
        .await;
        let deployer = app.acting_as(&deployer).await;
        let denied = action(&app, &deployer, "services", "deploy", &id.to_string()).await;
        assert!(
            denied.status != StatusCode::OK || denied.text().contains("denied"),
            "{}",
            denied.text()
        );
        deployer
            .post_json(
                &app.admin_api(&format!("x/apps/services/{id}/trigger")),
                &json!({}),
            )
            .await
            .assert_status(StatusCode::FORBIDDEN);
    }
);

db_test!(
    a_network_made_before_instances_is_still_used,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::default();
        let legacy = |instance: Option<&str>| {
            let mut labels = std::collections::HashMap::from([
                ("rdockpanel.managed".to_owned(), "true".to_owned()),
                ("rdockpanel.project".to_owned(), "shop".to_owned()),
            ]);
            if let Some(i) = instance {
                labels.insert("rdockpanel.instance".to_owned(), i.to_owned());
            }
            rdockpanel::services::docker_service::NetworkItem {
                id: "netid-rdp_shop".to_owned(),
                name: "rdp_shop".to_owned(),
                driver: "overlay".to_owned(),
                scope: "swarm".to_owned(),
                labels,
            }
        };
        // Made by an earlier version (no instance label): deployed into.
        fake.networks.lock().unwrap().push(legacy(None));
        let app = support::app_with_docker(db, fake.clone()).await;
        let (admin, id) = shop(&app).await;
        assert_eq!(run(&app, &admin, "deploy", id).await.status, "succeeded");
        // Another panel's (another instance): never.
        *fake.networks.lock().unwrap() = vec![legacy(Some("another"))];
        let refused = run(&app, &admin, "deploy", id).await;
        assert_eq!(refused.status, "failed", "{:?}", refused.error);
    }
);

db_test!(
    force_rebuild_and_recent_errors,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::default();
        let app = support::app_with_docker(db, fake.clone()).await;
        let (admin, web) = shop(&app).await;
        let project = service_row(&app, web).await.project_id.to_string();
        let site: i64 = create(
            &app,
            &admin,
            "services",
            json!({"project_id": project, "name": "site", "replicas": 1, "source": "dockerfile", "dockerfile": "FROM nginx:1.27\n"}),
        )
        .await
        .parse()
        .unwrap();
        let forced = |fake: &FakeDocker, name: &str| {
            fake.services.lock().unwrap()[name]
                .spec
                .clone()
                .unwrap()
                .task_template
                .unwrap()
                .force_update
                .unwrap_or(0)
        };

        // Built: again without the cache, its tasks replaced.
        let first = run(&app, &admin, "deploy", site).await;
        assert_eq!(first.status, "succeeded", "{:?}", first.error);
        assert!(!first.force_rebuild);
        let before = forced(&fake, "shop_site");
        let rebuilt = run(&app, &admin, "rebuild", site).await;
        assert_eq!(rebuilt.status, "succeeded", "{:?}", rebuilt.error);
        assert_eq!(rebuilt.trigger, "deploy");
        assert!(rebuilt.force_rebuild);
        let calls = fake.calls();
        assert!(
            calls.contains(&format!("build no-cache rdp/shop_site:d{}", rebuilt.id)),
            "{calls:?}"
        );
        assert!(!calls.contains(&format!("build no-cache rdp/shop_site:d{}", first.id)));
        assert_eq!(forced(&fake, "shop_site"), before + 1);
        // Back to it: its recorded image, nothing built.
        action(
            &app,
            &admin,
            "deployments",
            "rollback",
            &rebuilt.id.to_string(),
        )
        .await
        .assert_status(StatusCode::OK);
        let back = last_deployment(&app, site).await;
        assert_eq!(back.trigger, "rollback");
        assert!(!back.force_rebuild);
        let builds = fake
            .calls()
            .iter()
            .filter(|c| c.starts_with("build "))
            .count();
        deployment_service::run(app.state(), back.id, FAST)
            .await
            .unwrap();
        assert_eq!(last_deployment(&app, site).await.status, "succeeded");
        assert_eq!(
            fake.calls()
                .iter()
                .filter(|c| c.starts_with("build "))
                .count(),
            builds
        );
        // A plain Deploy after it: with the cache again.
        let again = run(&app, &admin, "deploy", site).await;
        assert!(!again.force_rebuild);
        assert!(
            !fake
                .calls()
                .contains(&format!("build no-cache rdp/shop_site:d{}", again.id))
        );

        // An image: pulled again, its tasks replaced (nothing changed).
        assert_eq!(run(&app, &admin, "deploy", web).await.status, "succeeded");
        let before = forced(&fake, "shop_web");
        assert_eq!(run(&app, &admin, "rebuild", web).await.status, "succeeded");
        assert_eq!(forced(&fake, "shop_web"), before + 1);
        let page = admin
            .get(&app.admin_api(&format!("x/apps/services/{web}")))
            .await
            .json();
        assert_eq!(page["latest"]["rebuild"], true, "{page}");

        // Recent errors: its failed tasks, its last failed deployment.
        let errors = app.admin_api(&format!("x/apps/services/{web}/errors"));
        let seen = admin.get(&errors).await;
        seen.assert_status(StatusCode::OK);
        assert_eq!(seen.json()["tasks"], json!([]));
        assert_eq!(seen.json()["deployment"], Value::Null);
        *fake.task_state.lock().unwrap() =
            Some(rdockpanel::services::docker_service::TaskState::FAILED);
        let failed = last_deployment(&app, web).await;
        let mut row: deployment::ActiveModel = failed.clone().into();
        row.status = sea_orm::Set("failed".to_owned());
        row.error = sea_orm::Set(Some("health check failed".to_owned()));
        row.log = sea_orm::Set(Some(format!("{}\nthe end\n", "é".repeat(3000))));
        sea_orm::ActiveModelTrait::update(row, app.state().db())
            .await
            .unwrap();
        let seen = admin.get(&errors).await.json();
        let tasks = seen["tasks"].as_array().unwrap();
        assert_eq!(tasks.len(), 6, "{seen}");
        assert_eq!(tasks[0]["state"], "failed");
        assert_eq!(tasks[0]["error"], "exit code 1");
        // Nothing runs in their slot since: not recovered.
        assert_eq!(tasks[0]["recovered"], false);
        assert_eq!(seen["deployment"]["id"], failed.id);
        assert_eq!(seen["deployment"]["error"], "health check failed");
        // Its end, at a line: no part of the long first line.
        assert_eq!(seen["deployment"]["log_tail"], "the end\n");
        // A last line longer than the tail: its end, kept.
        let mut row: deployment::ActiveModel = last_deployment(&app, web).await.into();
        row.log = sea_orm::Set(Some(format!("start\n{}\n", "é".repeat(3000))));
        sea_orm::ActiveModelTrait::update(row, app.state().db())
            .await
            .unwrap();
        let tail = admin.get(&errors).await.json()["deployment"]["log_tail"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(
            tail.len() <= 4096 && tail.len() > 4000 && tail.ends_with("é\n"),
            "{}",
            tail.len()
        );
        // The deployment only with deployments.view.
        let viewer = support::user_with(&app, "viewer", &["services.view"]).await;
        let viewer = app.acting_as(&viewer).await;
        let seen = viewer.get(&errors).await;
        seen.assert_status(StatusCode::OK);
        let seen = seen.json();
        assert_eq!(seen["tasks"].as_array().unwrap().len(), 6);
        assert_eq!(seen["deployment"], Value::Null);
        // A Compose service's: its administrators'.
        let mut compose: service::ActiveModel = service_row(&app, web).await.into();
        compose.kind = sea_orm::Set("compose".to_owned());
        sea_orm::ActiveModelTrait::update(compose, app.state().db())
            .await
            .unwrap();
        viewer
            .get(&errors)
            .await
            .assert_status(StatusCode::FORBIDDEN);
        admin.get(&errors).await.assert_status(StatusCode::OK);
    }
);

db_test!(
    a_first_domain_added_after_its_deployment_is_routed,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::default();
        let app = support::app_with_docker(db, fake.clone()).await;
        let user = support::user_with(&app, "admin", &[]).await;
        app.make_super_admin(&user).await;
        let admin = app.acting_as(&user).await;
        let project = create(&app, &admin, "projects", json!({"name": "blog"})).await;
        let service = create(
            &app,
            &admin,
            "services",
            json!({"project_id": project, "name": "web", "image": "nginx:1.27", "replicas": 1}),
        )
        .await;
        let id: i64 = service.parse().unwrap();
        let traefik_networks = |fake: &FakeDocker| -> Vec<String> {
            fake.services.lock().unwrap()["rdp-traefik"]
                .spec
                .clone()
                .unwrap()
                .task_template
                .unwrap()
                .networks
                .unwrap_or_default()
                .into_iter()
                .filter_map(|n| n.target)
                .collect()
        };
        // Deployed without a domain: Traefik stays off its project's network.
        assert_eq!(run(&app, &admin, "deploy", id).await.status, "succeeded");
        assert!(!traefik_networks(&fake).contains(&"rdp_blog".to_owned()));
        create(&app, &admin, "service_domains", json!({"service_id": service, "host": "blog.localhost", "path": "/", "port": 80, "https": false})).await;
        // The job the save queued: Traefik joins, the route is written.
        rustcms::queue::run_until_idle(app.state(), None, 20)
            .await
            .unwrap();
        assert!(traefik_networks(&fake).contains(&"rdp_blog".to_owned()));
        let routes =
            std::fs::read_to_string(support::traefik_dir(&app).join("blog_web.yml")).unwrap();
        assert!(routes.contains("Host(`blog.localhost`)"), "{routes}");
        assert!(routes.contains("http://blog_web:80"), "{routes}");
        assert_eq!(aside(&app, &admin, id).await["pending"], json!(false));
    }
);
