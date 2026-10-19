//! The Docker and server endpoints under `/admin/api/x`: who may call
//! which, the panel's own objects, and the actions log.

use rdockpanel::database::migrations::Migrator;
use rdockpanel::models::action;
use rdockpanel::routes::admin::routes;
use rustcms::axum::http::{Method, StatusCode};
use rustcms_testing::admin_routes::{Case, assert_access};
use rustcms_testing::{TestDb, db_test};
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder};
use serde_json::json;

use crate::support::{self, fake_docker::FakeDocker};

const VIEWER: &[&str] = &["docker.view", "docker.logs", "server.view"];
const OPERATOR: &[&str] = &["docker.view", "docker.logs", "docker.manage", "server.view"];

db_test!(
    every_route_checks_its_permission,
    migrate = Migrator,
    |db: TestDb| async move {
        let app = support::app_with_docker(db, FakeDocker::with_app_and_panel()).await;
        let nobody = support::user_with(&app, "nobody", &[]).await;
        let viewer = support::user_with(&app, "viewer", VIEWER).await;
        let operator = support::user_with(&app, "operator", OPERATOR).await;
        let anonymous = app.client();
        let nobody = app.acting_as(&nobody).await;
        let viewer = app.acting_as(&viewer).await;
        let operator = app.acting_as(&operator).await;

        let read = |path: &str| {
            Case::new(Method::GET, path)
                .expect("anonymous", &anonymous, StatusCode::UNAUTHORIZED)
                .expect("nobody", &nobody, StatusCode::FORBIDDEN)
                .expect("viewer", &viewer, StatusCode::OK)
        };
        // `{}` bodies: allowed writes answer 200 (no body needed) or 422 (one is).
        let write = |method: Method, path: &str, allowed: StatusCode| {
            Case::new(method, path)
                .expect("nobody", &nobody, StatusCode::FORBIDDEN)
                .expect("viewer", &viewer, StatusCode::FORBIDDEN)
                .expect("operator", &operator, allowed)
        };
        assert_access(
            &app,
            &routes(),
            &[
                read("/docker/info"),
                read("/docker/containers"),
                read("/docker/containers/web"),
                read("/docker/containers/web/logs"),
                read("/docker/images"),
                read("/docker/volumes"),
                read("/docker/networks"),
                read("/server/info"),
                // Streams: admitted with a 200 (the body is not read here).
                read("/server/live"),
                read("/docker/containers/web/stats"),
                read("/docker/containers/web/logs/follow"),
                read("/docker/containers/web/logs/follow/10"),
                // The apps routes need the resources' permissions, which none of these has.
                // Every signed-in user: their projects (none here).
                Case::new(Method::GET, "/apps/overview")
                    .expect("anonymous", &anonymous, StatusCode::UNAUTHORIZED)
                    .expect("viewer", &viewer, StatusCode::OK),
                // A project's members: its admins (projects.members there).
                Case::new(Method::GET, "/apps/projects/1/members")
                    .expect("anonymous", &anonymous, StatusCode::UNAUTHORIZED)
                    .expect("viewer", &viewer, StatusCode::FORBIDDEN),
                Case::new(Method::POST, "/apps/projects/1/members").expect(
                    "viewer",
                    &viewer,
                    StatusCode::FORBIDDEN,
                ),
                Case::new(Method::PATCH, "/apps/projects/1/members/1").expect(
                    "viewer",
                    &viewer,
                    StatusCode::FORBIDDEN,
                ),
                Case::new(Method::DELETE, "/apps/projects/1/members/1").expect(
                    "viewer",
                    &viewer,
                    StatusCode::FORBIDDEN,
                ),
                Case::new(Method::GET, "/apps/users")
                    .expect("anonymous", &anonymous, StatusCode::UNAUTHORIZED)
                    .expect("viewer", &viewer, StatusCode::FORBIDDEN),
                // A service's logs and terminal: its own permissions, or Docker's.
                Case::new(Method::GET, "/apps/services/1/logs/10")
                    .expect("anonymous", &anonymous, StatusCode::UNAUTHORIZED)
                    .expect("nobody", &nobody, StatusCode::FORBIDDEN),
                // A socket: refused unsigned here (a WebSocket upgrade first;
                // its permission in tests/feature/members.rs).
                Case::new(Method::GET, "/apps/services/1/exec/abc").expect(
                    "anonymous",
                    &anonymous,
                    StatusCode::UNAUTHORIZED,
                ),
                // GitHub tokens: system.manage; a service's: services.update.
                Case::new(Method::GET, "/apps/git/providers")
                    .expect("anonymous", &anonymous, StatusCode::UNAUTHORIZED)
                    .expect("operator", &operator, StatusCode::FORBIDDEN),
                Case::new(Method::POST, "/apps/git/providers").expect(
                    "operator",
                    &operator,
                    StatusCode::FORBIDDEN,
                ),
                Case::new(Method::PATCH, "/apps/git/providers/1").expect(
                    "operator",
                    &operator,
                    StatusCode::FORBIDDEN,
                ),
                Case::new(Method::DELETE, "/apps/git/providers/1").expect(
                    "operator",
                    &operator,
                    StatusCode::FORBIDDEN,
                ),
                Case::new(Method::POST, "/apps/git/providers/1/test").expect(
                    "operator",
                    &operator,
                    StatusCode::FORBIDDEN,
                ),
                Case::new(Method::GET, "/apps/services/1/github")
                    .expect("anonymous", &anonymous, StatusCode::UNAUTHORIZED)
                    .expect("viewer", &viewer, StatusCode::FORBIDDEN),
                Case::new(Method::GET, "/apps/services/1/github/1/repos").expect(
                    "viewer",
                    &viewer,
                    StatusCode::FORBIDDEN,
                ),
                Case::new(Method::GET, "/apps/services/1/github/1/branches").expect(
                    "viewer",
                    &viewer,
                    StatusCode::FORBIDDEN,
                ),
                // Settings → Panel's controls: system.manage.
                Case::new(Method::GET, "/apps/system/status")
                    .expect("anonymous", &anonymous, StatusCode::UNAUTHORIZED)
                    .expect("operator", &operator, StatusCode::FORBIDDEN),
                Case::new(Method::GET, "/apps/system/logs/10")
                    .expect("anonymous", &anonymous, StatusCode::UNAUTHORIZED)
                    .expect("operator", &operator, StatusCode::FORBIDDEN),
                Case::new(Method::POST, "/apps/system/restart").expect(
                    "operator",
                    &operator,
                    StatusCode::FORBIDDEN,
                ),
                Case::new(Method::POST, "/apps/system/reboot").expect(
                    "operator",
                    &operator,
                    StatusCode::FORBIDDEN,
                ),
                Case::new(Method::POST, "/apps/system/update").expect(
                    "operator",
                    &operator,
                    StatusCode::FORBIDDEN,
                ),
                Case::new(Method::POST, "/apps/system/ip").expect(
                    "operator",
                    &operator,
                    StatusCode::FORBIDDEN,
                ),
                Case::new(Method::POST, "/apps/system/prune/volumes").expect(
                    "operator",
                    &operator,
                    StatusCode::FORBIDDEN,
                ),
                Case::new(Method::GET, "/apps/domains")
                    .expect("anonymous", &anonymous, StatusCode::UNAUTHORIZED)
                    .expect("viewer", &viewer, StatusCode::FORBIDDEN),
                Case::new(Method::GET, "/apps/services/1")
                    .expect("anonymous", &anonymous, StatusCode::UNAUTHORIZED)
                    .expect("viewer", &viewer, StatusCode::FORBIDDEN),
                Case::new(Method::GET, "/apps/services/1/errors")
                    .expect("anonymous", &anonymous, StatusCode::UNAUTHORIZED)
                    .expect("viewer", &viewer, StatusCode::FORBIDDEN),
                Case::new(Method::GET, "/apps/templates")
                    .expect("anonymous", &anonymous, StatusCode::UNAUTHORIZED)
                    .expect("viewer", &viewer, StatusCode::FORBIDDEN),
                Case::new(Method::GET, "/apps/templates/ghost/logo")
                    .expect("anonymous", &anonymous, StatusCode::UNAUTHORIZED)
                    .expect("viewer", &viewer, StatusCode::FORBIDDEN),
                Case::new(Method::GET, "/apps/templates/ghost")
                    .expect("anonymous", &anonymous, StatusCode::UNAUTHORIZED)
                    .expect("viewer", &viewer, StatusCode::FORBIDDEN),
                // Traefik's resolvers: the panel's administrators' only.
                Case::new(Method::GET, "/apps/traefik/resolvers")
                    .expect("anonymous", &anonymous, StatusCode::UNAUTHORIZED)
                    .expect("viewer", &viewer, StatusCode::FORBIDDEN)
                    .expect("operator", &operator, StatusCode::FORBIDDEN),
                Case::new(Method::GET, "/apps/panel")
                    .expect("anonymous", &anonymous, StatusCode::UNAUTHORIZED)
                    .expect("operator", &operator, StatusCode::FORBIDDEN),
                Case::new(Method::POST, "/apps/panel/check")
                    .expect("nobody", &nobody, StatusCode::FORBIDDEN)
                    .expect("operator", &operator, StatusCode::FORBIDDEN),
                Case::new(Method::GET, "/apps/traefik/status")
                    .expect("anonymous", &anonymous, StatusCode::UNAUTHORIZED)
                    .expect("operator", &operator, StatusCode::FORBIDDEN),
                Case::new(Method::GET, "/apps/traefik/logs/10")
                    .expect("anonymous", &anonymous, StatusCode::UNAUTHORIZED)
                    .expect("operator", &operator, StatusCode::FORBIDDEN),
                Case::new(Method::POST, "/apps/traefik/restart")
                    .expect("nobody", &nobody, StatusCode::FORBIDDEN)
                    .expect("operator", &operator, StatusCode::FORBIDDEN),
                Case::new(Method::PUT, "/apps/traefik/middlewares")
                    .expect("nobody", &nobody, StatusCode::FORBIDDEN)
                    .expect("operator", &operator, StatusCode::FORBIDDEN),
                Case::new(Method::GET, "/apps/certificates")
                    .expect("anonymous", &anonymous, StatusCode::UNAUTHORIZED)
                    .expect("operator", &operator, StatusCode::FORBIDDEN),
                Case::new(Method::POST, "/apps/certificates")
                    .expect("nobody", &nobody, StatusCode::FORBIDDEN)
                    .expect("operator", &operator, StatusCode::FORBIDDEN),
                Case::new(Method::PUT, "/apps/certificates/1")
                    .expect("nobody", &nobody, StatusCode::FORBIDDEN)
                    .expect("operator", &operator, StatusCode::FORBIDDEN),
                Case::new(Method::DELETE, "/apps/certificates/1")
                    .expect("nobody", &nobody, StatusCode::FORBIDDEN)
                    .expect("operator", &operator, StatusCode::FORBIDDEN),
                Case::new(Method::POST, "/apps/traefik/apply")
                    .expect("nobody", &nobody, StatusCode::FORBIDDEN)
                    .expect("operator", &operator, StatusCode::FORBIDDEN),
                Case::new(Method::POST, "/apps/templates/refresh")
                    .expect("nobody", &nobody, StatusCode::FORBIDDEN)
                    .expect("viewer", &viewer, StatusCode::FORBIDDEN),
                Case::new(Method::POST, "/apps/projects/1/templates/ghost")
                    .expect("nobody", &nobody, StatusCode::FORBIDDEN)
                    .expect("viewer", &viewer, StatusCode::FORBIDDEN),
                Case::new(Method::POST, "/apps/projects/1/schema")
                    .expect("nobody", &nobody, StatusCode::FORBIDDEN)
                    .expect("viewer", &viewer, StatusCode::FORBIDDEN),
                Case::new(Method::POST, "/apps/projects/1/services")
                    .expect("nobody", &nobody, StatusCode::FORBIDDEN)
                    .expect("viewer", &viewer, StatusCode::FORBIDDEN),
                Case::new(Method::GET, "/apps/deployments/1")
                    .expect("anonymous", &anonymous, StatusCode::UNAUTHORIZED)
                    .expect("viewer", &viewer, StatusCode::FORBIDDEN),
                Case::new(Method::GET, "/apps/services/1/credentials")
                    .expect("anonymous", &anonymous, StatusCode::UNAUTHORIZED)
                    .expect("viewer", &viewer, StatusCode::FORBIDDEN),
                Case::new(Method::GET, "/apps/services/1/env")
                    .expect("anonymous", &anonymous, StatusCode::UNAUTHORIZED)
                    .expect("viewer", &viewer, StatusCode::FORBIDDEN),
                Case::new(Method::GET, "/apps/services/1/backups")
                    .expect("anonymous", &anonymous, StatusCode::UNAUTHORIZED)
                    .expect("viewer", &viewer, StatusCode::FORBIDDEN),
                Case::new(Method::GET, "/apps/backup-runs/1")
                    .expect("anonymous", &anonymous, StatusCode::UNAUTHORIZED)
                    .expect("viewer", &viewer, StatusCode::FORBIDDEN),
                Case::new(Method::POST, "/apps/backups/1/run")
                    .expect("nobody", &nobody, StatusCode::FORBIDDEN)
                    .expect("viewer", &viewer, StatusCode::FORBIDDEN),
                Case::new(Method::POST, "/apps/services/1/restore")
                    .expect("nobody", &nobody, StatusCode::FORBIDDEN)
                    .expect("viewer", &viewer, StatusCode::FORBIDDEN),
                Case::new(Method::GET, "/apps/services/1/stats")
                    .expect("anonymous", &anonymous, StatusCode::UNAUTHORIZED)
                    .expect("viewer", &viewer, StatusCode::FORBIDDEN),
                Case::new(Method::POST, "/apps/services/1/basic-auth")
                    .expect("nobody", &nobody, StatusCode::FORBIDDEN)
                    .expect("viewer", &viewer, StatusCode::FORBIDDEN),
                Case::new(Method::DELETE, "/apps/services/1/basic-auth/1")
                    .expect("nobody", &nobody, StatusCode::FORBIDDEN)
                    .expect("viewer", &viewer, StatusCode::FORBIDDEN),
                Case::new(Method::POST, "/apps/services/1/github-webhook")
                    .expect("nobody", &nobody, StatusCode::FORBIDDEN)
                    .expect("viewer", &viewer, StatusCode::FORBIDDEN),
                Case::new(Method::POST, "/apps/scripts/1/run")
                    .expect("nobody", &nobody, StatusCode::FORBIDDEN)
                    .expect("viewer", &viewer, StatusCode::FORBIDDEN)
                    .expect("operator", &operator, StatusCode::FORBIDDEN),
                // Every signed-in user: what they may see (none here).
                Case::new(Method::GET, "/apps/actions")
                    .expect("anonymous", &anonymous, StatusCode::UNAUTHORIZED)
                    .expect("viewer", &viewer, StatusCode::OK),
                Case::new(Method::GET, "/apps/notification-channels")
                    .expect("anonymous", &anonymous, StatusCode::UNAUTHORIZED)
                    .expect("viewer", &viewer, StatusCode::FORBIDDEN),
                Case::new(Method::POST, "/apps/notification-channels/1/test")
                    .expect("nobody", &nobody, StatusCode::FORBIDDEN)
                    .expect("viewer", &viewer, StatusCode::FORBIDDEN),
                Case::new(Method::GET, "/apps/storage-providers")
                    .expect("anonymous", &anonymous, StatusCode::UNAUTHORIZED)
                    .expect("viewer", &viewer, StatusCode::FORBIDDEN),
                Case::new(Method::GET, "/apps/storage-providers/1/files")
                    .expect("anonymous", &anonymous, StatusCode::UNAUTHORIZED)
                    .expect("viewer", &viewer, StatusCode::FORBIDDEN),
                Case::new(Method::POST, "/apps/storage-providers/1/check")
                    .expect("nobody", &nobody, StatusCode::FORBIDDEN)
                    .expect("viewer", &viewer, StatusCode::FORBIDDEN),
                Case::new(Method::POST, "/apps/services/1/deploy-key")
                    .expect("nobody", &nobody, StatusCode::FORBIDDEN)
                    .expect("viewer", &viewer, StatusCode::FORBIDDEN),
                Case::new(Method::POST, "/apps/services/1/trigger")
                    .expect("nobody", &nobody, StatusCode::FORBIDDEN)
                    .expect("viewer", &viewer, StatusCode::FORBIDDEN),
                Case::new(Method::PUT, "/apps/services/1/env")
                    .expect("nobody", &nobody, StatusCode::FORBIDDEN)
                    .expect("viewer", &viewer, StatusCode::FORBIDDEN),
                // A deployment's log needs deployments.view, which none of these has.
                Case::new(Method::GET, "/deploy/deployments/1/log")
                    .expect("anonymous", &anonymous, StatusCode::UNAUTHORIZED)
                    .expect("viewer", &viewer, StatusCode::FORBIDDEN),
                // The terminal needs docker.exec, which none of these has (its
                // own tests open it: tests/feature/terminal.rs).
                Case::new(Method::GET, "/docker/containers/web/exec")
                    .expect("anonymous", &anonymous, StatusCode::UNAUTHORIZED)
                    .expect("viewer", &viewer, StatusCode::FORBIDDEN)
                    .expect("operator", &operator, StatusCode::FORBIDDEN),
                Case::new(Method::GET, "/docker/actions/1")
                    .expect("anonymous", &anonymous, StatusCode::UNAUTHORIZED)
                    .expect("nobody", &nobody, StatusCode::FORBIDDEN)
                    .expect("viewer", &viewer, StatusCode::NOT_FOUND),
                read("/server/samples"),
                write(Method::DELETE, "/docker/containers/web", StatusCode::OK),
                write(
                    Method::POST,
                    "/docker/containers/web/restart",
                    StatusCode::OK,
                ),
                write(
                    Method::POST,
                    "/docker/images/pull",
                    StatusCode::UNPROCESSABLE_ENTITY,
                ),
                write(Method::DELETE, "/docker/images/web:latest", StatusCode::OK),
                write(Method::POST, "/docker/prune", StatusCode::OK),
                write(
                    Method::POST,
                    "/docker/volumes",
                    StatusCode::UNPROCESSABLE_ENTITY,
                ),
                write(Method::DELETE, "/docker/volumes/web-data", StatusCode::OK),
                write(
                    Method::POST,
                    "/docker/networks",
                    StatusCode::UNPROCESSABLE_ENTITY,
                ),
                write(Method::DELETE, "/docker/networks/web-net", StatusCode::OK),
            ],
        )
        .await;
    }
);

db_test!(
    the_panels_own_objects_need_system_manage,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::with_app_and_panel();
        let app = support::app_with_docker(db, fake.clone()).await;
        let operator = support::user_with(&app, "operator", OPERATOR).await;
        let admin = support::user_with(&app, "admin", &[]).await;
        app.make_super_admin(&admin).await;
        let operator = app.acting_as(&operator).await;
        let admin = app.acting_as(&admin).await;
        let url = |path: &str| format!("/admin/api/x/docker{path}");

        let names = |response: rustcms_testing::TestResponse| {
            let list = response.json()["containers"].as_array().unwrap().clone();
            list.iter()
                .map(|c| c["name"].as_str().unwrap().to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(names(operator.get(&url("/containers")).await), ["web"]);
        assert_eq!(
            names(admin.get(&url("/containers")).await),
            ["web", "panel"]
        );

        operator
            .get(&url("/containers/panel"))
            .await
            .assert_status(StatusCode::FORBIDDEN);
        operator
            .get(&url("/containers/panel/logs"))
            .await
            .assert_status(StatusCode::FORBIDDEN);
        operator
            .post_json(&url("/containers/panel/stop"), &json!({}))
            .await
            .assert_status(StatusCode::FORBIDDEN);
        operator
            .send(Method::DELETE, &url("/volumes/panel-data"), None)
            .await
            .assert_status(StatusCode::FORBIDDEN);
        assert!(fake.calls().is_empty(), "{:?}", fake.calls());

        admin
            .post_json(&url("/containers/panel/restart"), &json!({}))
            .await
            .assert_status(StatusCode::OK);
        assert_eq!(fake.calls(), ["restart id-panel"]);
        // Secrets stay hidden even from the system scope.
        admin
            .get(&url("/containers/panel"))
            .await
            .assert_status(StatusCode::OK)
            .assert_json("/Config/Env/1", json!("DB_PASSWORD=******"));
    }
);

db_test!(
    changes_are_logged_as_actions,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::with_app_and_panel();
        let app = support::app_with_docker(db, fake.clone()).await;
        let operator = support::user_with(&app, "operator", OPERATOR).await;
        let operator_id = operator.id;
        let operator = app.acting_as(&operator).await;
        let url = |path: &str| format!("/admin/api/x/docker{path}");

        operator
            .post_json(&url("/containers/web/restart"), &json!({}))
            .await
            .assert_status(StatusCode::OK);
        operator
            .post_json(&url("/containers/nope/restart"), &json!({}))
            .await
            .assert_status(StatusCode::NOT_FOUND);
        operator
            .post_json(&url("/containers/web/explode"), &json!({}))
            .await
            .assert_status(StatusCode::BAD_REQUEST);
        operator
            .post_json(&url("/volumes"), &json!({"name": "-bad"}))
            .await
            .assert_status(StatusCode::UNPROCESSABLE_ENTITY);

        let rows = action::Entity::find()
            .order_by_asc(action::Column::Id)
            .all(app.state().db())
            .await
            .unwrap();
        let summary: Vec<_> = rows
            .iter()
            .map(|r| (r.kind.as_str(), r.target.as_str(), r.status.as_str()))
            .collect();
        assert_eq!(
            summary,
            [
                ("container.restart", "web", "ok"),
                ("container.restart", "nope", "failed")
            ]
        );
        assert!(
            rows.iter()
                .all(|r| r.user_id == Some(operator_id) && r.finished_at.is_some())
        );

        let audited = rustcms::models::audit_log::Entity::find()
            .filter(rustcms::models::audit_log::Column::Event.eq("docker.container.restart"))
            .count(app.state().db())
            .await
            .unwrap();
        assert_eq!(audited, 2);
        let state = app.state().clone();
        let count = |event: &'static str| {
            let state = state.clone();
            async move {
                rustcms::models::audit_log::Entity::find()
                    .filter(rustcms::models::audit_log::Column::Event.eq(event))
                    .count(state.db())
                    .await
                    .unwrap()
            }
        };

        // A pull is queued: its row waits for the `docker` queue's worker.
        let pulled = operator
            .post_json(&url("/images/pull"), &json!({"image": "alpine:3.20"}))
            .await;
        pulled
            .assert_status(StatusCode::ACCEPTED)
            .assert_json("/action/status", json!("queued"));
        // Its action: readable by who asked, not by another operator.
        let id = &pulled.json()["action"]["id"];
        let action_url = url(&format!(
            "/actions/{}",
            id.as_str().map_or_else(|| id.to_string(), str::to_owned)
        ));
        operator
            .get(&action_url)
            .await
            .assert_status(StatusCode::OK);
        let other = support::user_with(&app, "other", OPERATOR).await;
        app.acting_as(&other)
            .await
            .get(&action_url)
            .await
            .assert_status(StatusCode::NOT_FOUND);
        operator
            .post_json(&url("/images/pull"), &json!({"image": "bad image"}))
            .await
            .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(count("docker.image.pull").await, 1);
        let jobs = rustcms::models::job::Entity::find()
            .filter(rustcms::models::job::Column::Kind.eq("docker.image_pull"))
            .all(app.state().db())
            .await
            .unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].queue, "docker");
    }
);

db_test!(
    abandoned_actions_end_failed,
    migrate = Migrator,
    |db: TestDb| async move {
        use chrono::Utc;
        use rdockpanel::services::action_service;
        use sea_orm::{ActiveModelTrait, Set};
        let app = support::app_with_docker(db, FakeDocker::with_app_and_panel()).await;
        let db = app.state().db();
        let row = |status: &str, age: chrono::Duration| action::ActiveModel {
            kind: Set("image.pull".to_owned()),
            target: Set("alpine".to_owned()),
            status: Set(status.to_owned()),
            created_at: Set(Utc::now() - age),
            updated_at: Set(Utc::now() - age),
            ..Default::default()
        };
        let old = chrono::Duration::hours(2);
        let stale_running = row(action::RUNNING, old).insert(db).await.unwrap();
        let stale_queued = row(action::QUEUED, old).insert(db).await.unwrap();
        let fresh = row(action::RUNNING, chrono::Duration::minutes(1))
            .insert(db)
            .await
            .unwrap();
        let done = row(action::OK, old).insert(db).await.unwrap();

        assert_eq!(action_service::reap_stale(db).await.unwrap(), 2);
        let status = |id: i64| async move {
            action::Entity::find_by_id(id)
                .one(db)
                .await
                .unwrap()
                .unwrap()
                .status
        };
        assert_eq!(status(stale_running.id).await, action::FAILED);
        assert_eq!(status(stale_queued.id).await, action::FAILED);
        assert_eq!(status(fresh.id).await, action::RUNNING);
        assert_eq!(status(done.id).await, action::OK);
    }
);

db_test!(
    pages_open_by_permission_and_tell_what_to_offer,
    migrate = Migrator,
    |db: TestDb| async move {
        let app = support::app_with_docker(db, FakeDocker::with_app_and_panel()).await;
        let viewer = support::user_with(&app, "viewer", VIEWER).await;
        let operator = support::user_with(&app, "operator", OPERATOR).await;
        let viewer = app.acting_as(&viewer).await;
        let operator = app.acting_as(&operator).await;
        let page = |slug: &str| format!("/admin/api/pages/{slug}");

        for slug in ["containers", "images", "volumes", "networks"] {
            viewer.get(&page(slug)).await.assert_status(StatusCode::OK);
        }
        // The server's state is the Dashboard's (no page of its own).
        viewer
            .get(&page("server"))
            .await
            .assert_status(StatusCode::NOT_FOUND);

        let offered = |response: rustcms_testing::TestResponse| response.json().to_string();
        let as_viewer = offered(viewer.get(&page("containers")).await);
        assert!(
            as_viewer.contains(r#""docker.manage":false"#),
            "{as_viewer}"
        );
        let as_operator = offered(operator.get(&page("containers")).await);
        assert!(
            as_operator.contains(r#""docker.manage":true"#),
            "{as_operator}"
        );
        assert!(
            as_operator.contains(r#""system.manage":false"#),
            "{as_operator}"
        );
    }
);
