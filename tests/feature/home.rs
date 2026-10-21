//! The homepage at `/`: public, in English or Vietnamese, with its own
//! strict policy and nothing of this installation on it; turned off in
//! Settings → Panel (administrators only, audited), `/` then leads to the
//! panel.

use rdockpanel::database::migrations::Migrator;
use rustcms::axum::body::Body;
use rustcms::axum::http::{Method, Request, StatusCode};
use rustcms_testing::{TestDb, db_test};
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};
use serde_json::json;

use crate::support::{self, fake_docker::FakeDocker};

db_test!(
    the_homepage_and_its_switch,
    migrate = Migrator,
    |db: TestDb| async move {
        let app = support::app_with_docker(db, FakeDocker::default()).await;

        // Public, in English by default.
        let page = app.get("/").await;
        page.assert_status(StatusCode::OK);
        assert!(
            page.header("content-type")
                .unwrap()
                .starts_with("text/html")
        );
        let csp = page.header("content-security-policy").unwrap();
        assert!(
            csp.contains("default-src 'none'") && csp.contains("script-src 'self'"),
            "{csp}"
        );
        let html = page.text();
        assert!(html.contains(r#"<html lang="en">"#));
        assert!(html.contains("https://github.com/irisvn/rdockpanel"));
        assert!(html.contains(r#"href="/admin""#));
        assert!(!html.contains("{{"), "every text filled in");
        // One script, its own file (no inline code).
        assert_eq!(html.matches("<script").count(), 1);
        assert!(html.contains(r#"<script src="/home/home.js?v="#));
        assert!(
            !html.contains(env!("CARGO_PKG_VERSION")),
            "never the version"
        );

        // Vietnamese: asked for, or the browser's language.
        assert!(
            app.get("/?lang=vi")
                .await
                .text()
                .contains(r#"<html lang="vi">"#)
        );
        let request = Request::builder()
            .uri("/")
            .header("accept-language", "vi-VN,vi;q=0.9,en;q=0.8")
            .body(Body::empty())
            .unwrap();
        assert!(app.request(request).await.text().contains("Đăng nhập"));

        // Its stylesheet and images; an unknown image is none.
        let css = app.get("/home/home.css").await;
        css.assert_status(StatusCode::OK);
        assert!(css.header("content-type").unwrap().starts_with("text/css"));
        let script = app.get("/home/home.js").await;
        script.assert_status(StatusCode::OK);
        assert!(
            script
                .header("content-type")
                .unwrap()
                .starts_with("text/javascript")
        );
        let brand = app.get("/home/brand.css").await;
        brand.assert_status(StatusCode::OK);
        assert!(
            brand
                .header("content-type")
                .unwrap()
                .starts_with("text/css")
        );
        let image = app.get("/home/img/dashboard.jpg").await;
        image.assert_status(StatusCode::OK);
        assert_eq!(image.header("content-type").as_deref(), Some("image/jpeg"));
        app.get("/home/img/../../Cargo.toml")
            .await
            .assert_status(StatusCode::NOT_FOUND);
        app.get("/home/img/missing.jpg")
            .await
            .assert_status(StatusCode::NOT_FOUND);

        // The switch: administrators only.
        let url = app.admin_api("x/apps/home");
        let user = support::user_with(&app, "dev", &["projects.view_any"]).await;
        let dev = app.acting_as(&user).await;
        dev.get(&url).await.assert_status(StatusCode::FORBIDDEN);
        dev.send(Method::PUT, &url, Some(&json!({"enabled": false})))
            .await
            .assert_status(StatusCode::FORBIDDEN);
        let root = support::user_with(&app, "root", &[]).await;
        app.make_super_admin(&root).await;
        let admin = app.acting_as(&root).await;
        admin.get(&url).await.assert_json("/enabled", json!(true));

        // Off: `/` leads to the panel; the change is audited.
        admin
            .send(Method::PUT, &url, Some(&json!({"enabled": false})))
            .await
            .assert_json("/enabled", json!(false));
        let off = app.get("/").await;
        assert!(off.status.is_redirection(), "{}", off.status);
        assert_eq!(off.header("location").as_deref(), Some("/admin"));
        let audited = rustcms::models::audit_log::Entity::find()
            .filter(rustcms::models::audit_log::Column::Event.eq("home.update"))
            .count(app.state().db())
            .await
            .unwrap();
        assert_eq!(audited, 1);

        // On again.
        admin
            .send(Method::PUT, &url, Some(&json!({"enabled": true})))
            .await
            .assert_status(StatusCode::OK);
        app.get("/").await.assert_status(StatusCode::OK);
    }
);
