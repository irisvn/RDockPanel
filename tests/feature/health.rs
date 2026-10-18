use rdockpanel::database::migrations::Migrator;
use rustcms::axum::http::StatusCode;
use rustcms_testing::{Factory, TestDb, UserFactory, db_test};
use serde_json::json;

use crate::support;

db_test!(
    health_and_ping,
    migrate = Migrator,
    |db: TestDb| async move {
        let app = support::app(db).await;
        app.get("/health")
            .await
            .assert_status(StatusCode::OK)
            .assert_json("/database", json!("ok"));
        app.get("/api/v1/ping")
            .await
            .assert_status(StatusCode::OK)
            .assert_json("/pong", json!(true));
        app.get("/missing")
            .await
            .assert_status(StatusCode::NOT_FOUND)
            .assert_json("/code", json!("not_found"));
    }
);

db_test!(admin_sign_in, migrate = Migrator, |db: TestDb| async move {
    let app = support::app(db).await;
    UserFactory::new()
        .email("ada@example.test")
        .password("secret password")
        .create(app.state().db())
        .await;
    let browser = app.login_as("ada@example.test", "secret password").await;
    browser
        .get("/admin/api/auth/me")
        .await
        .assert_status(StatusCode::OK)
        .assert_json("/user/email", json!("ada@example.test"));
    browser
        .get("/admin/api/panel")
        .await
        .assert_status(StatusCode::OK);
});

db_test!(
    seeding_succeeds,
    migrate = Migrator,
    |db: TestDb| async move {
        use rustcms::database::Seeder;
        let app = support::app(db).await;
        rdockpanel::database::seeders::DatabaseSeeder
            .run(app.state())
            .await
            .unwrap();
    }
);
