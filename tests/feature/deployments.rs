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

const FAST: Timings = Timings {
    poll: Duration::from_millis(5),
    settle: Duration::from_millis(20),
    timeout: Duration::from_secs(2),
};

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
async fn run(app: &TestApp, admin: &TestClient<'_>, name: &str, service: i64) -> deployment::Model {
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
        create(&app, &admin, "service_mounts", json!({"service_id": id.to_string(), "kind": "file", "target": "/etc/app.ini", "content": "a=1\n"})).await;
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

        // A new content: a new config; the old one is removed once unused.
        let mount = service_mount_id(&app, id).await;
        admin
            .send(
                rustcms::axum::http::Method::PATCH,
                &format!("{}/{mount}", records(&app, "service_mounts")),
                Some(&json!({"content": "a=2\n"})),
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

        // A domain added: routed only once deployed.
        create(&app, &admin, "service_domains", json!({"service_id": id.to_string(), "host": "www.shop.localhost", "path": "/", "port": 80, "https": true})).await;
        assert!(
            !std::fs::read_to_string(&file)
                .unwrap()
                .contains("www.shop.localhost")
        );
        assert_eq!(run(&app, &admin, "deploy", id).await.status, "succeeded");
        let routes = std::fs::read_to_string(&file).unwrap();
        assert!(
            routes.contains("www.shop.localhost") && routes.contains("websecure"),
            "{routes}"
        );

        // A failed deployment (of another domain) leaves the routes of what
        // still runs.
        create(&app, &admin, "service_domains", json!({"service_id": id.to_string(), "host": "new.shop.localhost", "path": "/", "port": 80, "https": false})).await;
        *fake.task_state.lock().unwrap() = Some(TaskState::FAILED);
        assert_eq!(run(&app, &admin, "deploy", id).await.status, "failed");
        let routes = std::fs::read_to_string(&file).unwrap();
        assert!(
            routes.contains("www.shop.localhost") && !routes.contains("new.shop.localhost"),
            "{routes}"
        );
        *fake.task_state.lock().unwrap() = None;
        // Swarm rolled it back: the same.
        *fake.update_state.lock().unwrap() = Some(ServiceUpdateStatusStateEnum::ROLLBACK_COMPLETED);
        assert_eq!(run(&app, &admin, "deploy", id).await.status, "rolled_back");
        assert!(
            !std::fs::read_to_string(&file)
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
        action(&app, &admin, "services", "republish", &id.to_string())
            .await
            .assert_status(StatusCode::OK);
        let jobs = rustcms::models::job::Entity::find()
            .filter(rustcms::models::job::Column::Kind.eq("deploy.publish"))
            .count(app.state().db())
            .await
            .unwrap();
        assert_eq!(jobs, 1);
        let traefik = app
            .state()
            .service::<rdockpanel::services::traefik_service::Traefik>()
            .unwrap();
        // (The job's body.)
        assert!(
            deployment_service::publish(app.state().db(), &traefik, id, unpublished.id)
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
