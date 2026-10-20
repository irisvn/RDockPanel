//! Settings → Branding: the panel's name, color and images, checked as the
//! panel serves them, shown at once (runtime branding); administrators only.

use rdockpanel::database::migrations::Migrator;
use rustcms::axum::http::{Method, StatusCode};
use rustcms_testing::{TestDb, db_test};
use serde_json::json;

use crate::support::{self, fake_docker::FakeDocker};

/// The smallest PNG (1×1).
const PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE,
    0x42, 0x60, 0x82,
];

fn base64(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

db_test!(
    branding_is_checked_and_shown_at_once,
    migrate = Migrator,
    |db: TestDb| async move {
        let app = support::app_with_docker(db, FakeDocker::default()).await;
        let url = |p: &str| app.admin_api(&format!("x/apps/branding{p}"));

        // Administrators only.
        let user = support::user_with(&app, "dev", &["projects.view_any"]).await;
        let dev = app.acting_as(&user).await;
        dev.get(&url("")).await.assert_status(StatusCode::FORBIDDEN);
        dev.send(Method::PUT, &url(""), Some(&json!({"name": "Evil"})))
            .await
            .assert_status(StatusCode::FORBIDDEN);

        let root = support::user_with(&app, "root", &[]).await;
        app.make_super_admin(&root).await;
        let admin = app.acting_as(&root).await;
        let current = admin.get(&url("")).await.json();
        assert_eq!(current["name"], serde_json::Value::Null);
        assert_eq!(current["images"], json!({}));

        // Checked as the panel serves them.
        admin
            .send(
                Method::PUT,
                &url(""),
                Some(&json!({"name": "x".repeat(65)})),
            )
            .await
            .assert_validation_error("name");
        admin
            .send(Method::PUT, &url(""), Some(&json!({"primary": "red; }"})))
            .await
            .assert_validation_error("primary");
        let svg = base64(b"<svg xmlns='http://www.w3.org/2000/svg'/>");
        admin
            .send(
                Method::PUT,
                &url("/images/logo"),
                Some(&json!({"mime": "image/svg+xml", "data": svg})),
            )
            .await
            .assert_validation_error("data");
        // A PNG called WebP: refused (the bytes decide).
        admin
            .send(
                Method::PUT,
                &url("/images/logo"),
                Some(&json!({"mime": "image/webp", "data": base64(PNG)})),
            )
            .await
            .assert_validation_error("data");
        admin
            .send(
                Method::PUT,
                &url("/images/banner"),
                Some(&json!({"mime": "image/png", "data": base64(PNG)})),
            )
            .await
            .assert_status(StatusCode::NOT_FOUND);

        // Saved: the panel shows it on its next page (this instance at once).
        admin
            .send(
                Method::PUT,
                &url(""),
                Some(&json!({"name": "Acme Cloud", "primary": "#7c3aed"})),
            )
            .await
            .assert_status(StatusCode::OK);
        let saved = admin
            .send(
                Method::PUT,
                &url("/images/logo"),
                Some(&json!({"mime": "image/png", "data": base64(PNG)})),
            )
            .await;
        saved.assert_status(StatusCode::OK);
        assert_eq!(saved.json()["images"]["logo"]["mime"], "image/png");
        let index = app.client().get("/admin/").await;
        let html = index.text();
        assert!(html.contains("Acme Cloud"), "{html}");
        assert!(html.contains("assets/brand/logo"), "{html}");
        let logo = app.client().get("/admin/assets/brand/logo").await;
        logo.assert_status(StatusCode::OK);
        assert_eq!(logo.body.as_ref(), PNG);

        // Beyond MySQL's BLOB (64 KiB): kept whole.
        let mut large = PNG.to_vec();
        large.resize(300 * 1024, 0);
        admin
            .send(
                Method::PUT,
                &url("/images/favicon"),
                Some(&json!({"mime": "image/png", "data": base64(&large)})),
            )
            .await
            .assert_status(StatusCode::OK);
        let favicon = app.client().get("/admin/assets/brand/favicon").await;
        assert_eq!(favicon.body.len(), large.len());
        admin
            .send(Method::DELETE, &url("/images/favicon"), None)
            .await
            .assert_status(StatusCode::OK);

        // Cleared: RDockPanel's own again.
        admin
            .send(Method::DELETE, &url("/images/logo"), None)
            .await
            .assert_status(StatusCode::OK);
        admin
            .send(
                Method::PUT,
                &url(""),
                Some(&json!({"name": "", "primary": ""})),
            )
            .await
            .assert_status(StatusCode::OK);
        let html = app.client().get("/admin/").await.text();
        assert!(!html.contains("Acme Cloud"));
        assert!(html.contains("RDockPanel"));
    }
);
