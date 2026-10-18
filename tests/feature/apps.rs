//! The apps interface's routes (`/admin/api/x/apps`): the dashboard's
//! overview, a service's page, its environment as `.env` text.

use rdockpanel::database::migrations::Migrator;
use rustcms::axum::http::StatusCode;
use rustcms_testing::{TestDb, db_test};
use sea_orm::EntityTrait;
use serde_json::{Value, json};

use crate::deployments::{create, records};
use crate::support::{self, fake_docker::FakeDocker};

db_test!(
    the_dashboard_and_a_services_page,
    migrate = Migrator,
    |db: TestDb| async move {
        let app = support::app_with_docker(db, FakeDocker::default()).await;
        let user = support::user_with(&app, "admin", &[]).await;
        app.make_super_admin(&user).await;
        let admin = app.acting_as(&user).await;
        let project = create(&app, &admin, "projects", json!({"name": "shop"})).await;
        let service = create(
            &app,
            &admin,
            "services",
            json!({"project_id": project, "name": "web", "image": "nginx:1.27", "replicas": 1}),
        )
        .await;
        create(&app, &admin, "service_domains", json!({"service_id": service, "host": "shop.localhost", "path": "/", "port": 80, "https": false})).await;
        create(&app, &admin, "projects", json!({"name": "empty"})).await;

        let overview = admin.get(&app.admin_api("x/apps/overview")).await;
        overview.assert_status(StatusCode::OK);
        let projects = overview.json()["projects"].clone();
        assert_eq!(projects.as_array().unwrap().len(), 2);
        let shop = projects
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == "shop")
            .unwrap();
        assert_eq!(shop["services"][0]["name"], "web");
        assert_eq!(shop["services"][0]["status"], "new");
        assert_eq!(shop["services"][0]["domain"], "shop.localhost");

        let page = admin
            .get(&app.admin_api(&format!("x/apps/services/{service}")))
            .await;
        page.assert_status(StatusCode::OK);
        let page = page.json();
        assert_eq!(page["project"]["name"], "shop");
        assert_eq!(page["internal_host"], "shop_web");
        assert_eq!(page["domains"][0]["host"], "shop.localhost");
        assert_eq!(page["can"]["secrets"], true);
        assert_eq!(page["pending"], true);

        // Records only with their resource's permission.
        let reader = support::user_with(&app, "reader", &["projects.view_any"]).await;
        let reader = app.acting_as(&reader).await;
        let seen = reader.get(&app.admin_api("x/apps/overview")).await.json();
        assert_eq!(seen["services_visible"], false);
        assert!(
            seen["projects"]
                .as_array()
                .unwrap()
                .iter()
                .all(|p| p["services"] == json!([]))
        );
        let viewer = support::user_with(&app, "viewer", &["services.view"]).await;
        let viewer = app.acting_as(&viewer).await;
        let page = viewer
            .get(&app.admin_api(&format!("x/apps/services/{service}")))
            .await
            .json();
        assert_eq!(page["domains"], json!([]), "{page}");
        assert_eq!(page["can"]["domains"]["view"], false);
    }
);

db_test!(
    the_environment_as_text,
    migrate = Migrator,
    |db: TestDb| async move {
        const SECRET: &str = "s3cret-1d2e";
        let app = support::app_with_docker(db, FakeDocker::default()).await;
        let user = support::user_with(&app, "admin", &[]).await;
        app.make_super_admin(&user).await;
        let admin = app.acting_as(&user).await;
        let project = create(&app, &admin, "projects", json!({"name": "shop"})).await;
        let service = create(
            &app,
            &admin,
            "services",
            json!({"project_id": project, "name": "web", "image": "nginx:1.27", "replicas": 1}),
        )
        .await;
        let env = app.admin_api(&format!("x/apps/services/{service}/env"));

        // A line that is not NAME=value: refused, with its line.
        let bad = admin
            .send(
                rustcms::axum::http::Method::PUT,
                &env,
                Some(&json!({"text": "OK=1\nnot a pair"})),
            )
            .await;
        bad.assert_validation_error("text");
        assert!(bad.text().contains("line 2"), "{}", bad.text());

        let saved = admin
            .send(
                rustcms::axum::http::Method::PUT,
                &env,
                Some(&json!({"text": format!("# db\nDB_PASSWORD={SECRET}\nMODE=prod\n")})),
            )
            .await;
        saved.assert_status(StatusCode::OK);
        assert_eq!(saved.json()["names"], json!(["DB_PASSWORD", "MODE"]));
        assert!(!saved.text().contains(SECRET));
        // The page: names only.
        let page = admin
            .get(&app.admin_api(&format!("x/apps/services/{service}")))
            .await
            .text();
        assert!(
            page.contains("DB_PASSWORD") && !page.contains(SECRET),
            "{page}"
        );
        // Read back in clear (services.secrets), audited without values.
        let read = admin.get(&env).await;
        read.assert_status(StatusCode::OK);
        assert_eq!(
            read.json()["text"],
            format!("DB_PASSWORD={SECRET}\nMODE=prod\n")
        );
        // A save names the revision it read: someone else's change in between is refused.
        let revision = read.json()["revision"].clone();
        admin
            .send(
                rustcms::axum::http::Method::PUT,
                &env,
                Some(&json!({"text": "A=1", "revision": "stale"})),
            )
            .await
            .assert_status(StatusCode::CONFLICT);
        admin
            .send(rustcms::axum::http::Method::PUT, &env, Some(&json!({"text": format!("DB_PASSWORD={SECRET}\nMODE=prod\n"), "revision": revision})))
            .await
            .assert_status(StatusCode::OK);
        let audit = rustcms::models::audit_log::Entity::find()
            .all(app.state().db())
            .await
            .unwrap();
        let events: Vec<String> = audit.iter().map(|a| format!("{a:?}")).collect();
        assert!(events.iter().any(|e| e.contains("services.env.read")));
        assert!(events.iter().any(|e| e.contains("services.env.update")));
        assert!(!events.iter().any(|e| e.contains(SECRET)));

        // Without services.secrets: no values (writing needs services.update).
        let editor = support::user_with(
            &app,
            "editor",
            &[
                "projects.view_any",
                "services.view_any",
                "services.view",
                "services.update",
            ],
        )
        .await;
        let editor = app.acting_as(&editor).await;
        editor.get(&env).await.assert_status(StatusCode::FORBIDDEN);
        let page = editor
            .get(&app.admin_api(&format!("x/apps/services/{service}")))
            .await
            .json();
        assert_eq!(page["can"]["secrets"], false);
        assert_eq!(page["env_names"], json!(["DB_PASSWORD", "MODE"]));
        editor
            .send(
                rustcms::axum::http::Method::PUT,
                &env,
                Some(&json!({"text": "ONLY=1"})),
            )
            .await
            .assert_status(StatusCode::OK);
        let names: Value = admin
            .get(&app.admin_api(&format!("x/apps/services/{service}")))
            .await
            .json()["env_names"]
            .clone();
        assert_eq!(names, json!(["ONLY"]));
        let _ = records(&app, "services");
    }
);
