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

db_test!(
    databases_come_configured,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::default();
        let app = support::app_with_docker(db, fake.clone()).await;
        let user = support::user_with(&app, "admin", &[]).await;
        app.make_super_admin(&user).await;
        let admin = app.acting_as(&user).await;
        let project = create(&app, &admin, "projects", json!({"name": "shop"})).await;
        let url = app.admin_api(&format!("x/apps/projects/{project}/services"));

        // A database: only a name.
        let made = admin
            .post_json(&url, &json!({"name": "db", "kind": "postgres"}))
            .await;
        made.assert_status(StatusCode::OK);
        let id = made.json()["id"].as_i64().unwrap();
        let page = admin
            .get(&app.admin_api(&format!("x/apps/services/{id}")))
            .await
            .json();
        assert_eq!(page["service"]["kind"], "postgres");
        assert_eq!(page["service"]["image"], "postgres:17");
        assert_eq!(page["service"]["replicas"], 1);
        assert_eq!(page["mounts"][0]["target"], "/var/lib/postgresql/data");
        let facts = &page["database"];
        assert_eq!(facts["user"], "postgres");
        assert_eq!(facts["database"], "shop");
        assert_eq!(facts["host"], "shop_db");
        assert!(facts["password"].is_null(), "never on the page");
        // The passwords: an audited read.
        let secret = admin
            .get(&app.admin_api(&format!("x/apps/services/{id}/credentials")))
            .await;
        secret.assert_status(StatusCode::OK);
        let secret = secret.json();
        let password = secret["password"].as_str().unwrap().to_owned();
        let password = password.as_str();
        assert_eq!(password.len(), 24);
        assert_eq!(
            secret["url"],
            format!("postgres://postgres:{password}@shop_db:5432/shop?sslmode=disable")
        );
        let audit = rustcms::models::audit_log::Entity::find()
            .all(app.state().db())
            .await
            .unwrap();
        assert!(format!("{audit:?}").contains("services.credentials.read"));
        // Without services.secrets: no password, no URL.
        let viewer = support::user_with(&app, "viewer", &["services.view"]).await;
        let viewer = app.acting_as(&viewer).await;
        let seen = viewer
            .get(&app.admin_api(&format!("x/apps/services/{id}")))
            .await
            .json();
        viewer
            .get(&app.admin_api(&format!("x/apps/services/{id}/credentials")))
            .await
            .assert_status(StatusCode::FORBIDDEN);
        assert!(!format!("{seen}").contains(password));
        // One replica only.
        admin
            .send(
                rustcms::axum::http::Method::PATCH,
                &format!("{}/{id}", records(&app, "services")),
                Some(&json!({"replicas": 2})),
            )
            .await
            .assert_validation_error("replicas");
        // Never two tasks on its data: no zero downtime.
        admin
            .send(
                rustcms::axum::http::Method::PATCH,
                &format!("{}/{id}", records(&app, "services")),
                Some(&json!({"zero_downtime": true})),
            )
            .await
            .assert_validation_error("zero_downtime");

        // Deployed with its credentials in the environment, its volume and check.
        let deployment = admin
            .post_json(
                &app.admin_api("resources/services/actions/deploy"),
                &json!({"keys": [id.to_string()]}),
            )
            .await;
        deployment.assert_status(StatusCode::OK);
        let queued = crate::deployments::last_deployment(&app, id).await;
        rdockpanel::services::deployment_service::run(app.state(), queued.id, FAST)
            .await
            .unwrap();
        let spec = fake.services.lock().unwrap()["shop_db"]
            .spec
            .clone()
            .unwrap()
            .task_template
            .unwrap()
            .container_spec
            .unwrap();
        let env = spec.env.unwrap();
        assert!(
            env.contains(&format!("POSTGRES_PASSWORD={password}")),
            "{env:?}"
        );
        assert!(env.contains(&"POSTGRES_DB=shop".to_owned()));
        assert!(spec.health_check.unwrap().test.unwrap()[1].contains("pg_isready"));

        // Redis: the password from the environment, never in the arguments.
        let made = admin
            .post_json(&url, &json!({"name": "cache", "kind": "redis"}))
            .await;
        made.assert_status(StatusCode::OK);
        // An app needs an image; a name is checked; a taken one refused.
        admin
            .post_json(&url, &json!({"name": "web", "kind": "app"}))
            .await
            .assert_validation_error("image");
        admin
            .post_json(
                &url,
                &json!({"name": "Bad Name", "kind": "app", "image": "nginx"}),
            )
            .await
            .assert_validation_error("name");
        admin
            .post_json(&url, &json!({"name": "db", "kind": "mysql"}))
            .await
            .assert_validation_error("name");
        admin
            .post_json(&url, &json!({"name": "x", "kind": "oracle"}))
            .await
            .assert_validation_error("kind");
    }
);

const FAST: rdockpanel::services::deployment_service::Timings =
    rdockpanel::services::deployment_service::Timings {
        poll: std::time::Duration::from_millis(5),
        settle: std::time::Duration::from_millis(20),
        timeout: std::time::Duration::from_secs(2),
    };

db_test!(
    a_running_service_shows_its_resources,
    migrate = Migrator,
    |db: TestDb| async move {
        use rdockpanel::services::deployment_service;
        let fake = crate::support::fake_docker::FakeDocker::default();
        let app = support::app_with_docker(db, fake).await;
        let user = support::user_with(&app, "admin", &[]).await;
        app.make_super_admin(&user).await;
        let admin = app.acting_as(&user).await;
        let project =
            crate::deployments::create(&app, &admin, "projects", json!({"name": "shop"})).await;
        let id = crate::deployments::create(
            &app,
            &admin,
            "services",
            json!({"project_id": project, "name": "web", "image": "nginx:1.27", "replicas": 2}),
        )
        .await;
        crate::deployments::action(&app, &admin, "services", "deploy", &id)
            .await
            .assert_status(StatusCode::OK);
        let queued = crate::deployments::last_deployment(&app, id.parse().unwrap()).await;
        let fast = deployment_service::Timings {
            poll: std::time::Duration::from_millis(5),
            settle: std::time::Duration::from_millis(20),
            timeout: std::time::Duration::from_secs(2),
        };
        deployment_service::run(app.state(), queued.id, fast)
            .await
            .unwrap();
        let stats = admin
            .get(&app.admin_api(&format!("x/apps/services/{id}/stats")))
            .await;
        stats.assert_status(StatusCode::OK);
        let stats = stats.json();
        assert_eq!(stats["tasks"].as_array().unwrap().len(), 2, "{stats}");
        assert_eq!(stats["cpu_percent"], 25.0);
    }
);
