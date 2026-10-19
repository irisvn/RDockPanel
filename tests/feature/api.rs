//! The public API (`/api/v1`) with tokens of Settings → API tokens: a token
//! does what its user may (their roles, their projects' roles), within its
//! scopes — never more, never beyond its projects.

use rdockpanel::database::migrations::Migrator;
use rustcms::axum::http::{Method, StatusCode};
use rustcms_testing::{TestDb, db_test};
use serde_json::json;

use crate::deployments::create;
use crate::support::{self, fake_docker::FakeDocker};

db_test!(
    tokens_act_within_their_scopes_and_projects,
    migrate = Migrator,
    |db: TestDb| async move {
        let app = support::app_with_docker(db, FakeDocker::default()).await;
        let root = support::user_with(&app, "root", &[]).await;
        app.make_super_admin(&root).await;
        let admin = app.acting_as(&root).await;
        let shop = create(&app, &admin, "projects", json!({"name": "shop"})).await;
        let other = create(&app, &admin, "projects", json!({"name": "other"})).await;
        let web: i64 = create(
            &app,
            &admin,
            "services",
            json!({"project_id": shop, "name": "web", "image": "nginx:1.27", "replicas": 1}),
        )
        .await
        .parse()
        .unwrap();
        create(
            &app,
            &admin,
            "services",
            json!({"project_id": other, "name": "api", "image": "nginx:1.27", "replicas": 1}),
        )
        .await;
        let api = |p: &str| format!("/api/v1{p}");
        let tokens = app.admin_api("x/apps/tokens");

        // Made in Settings: scopes required and known; shown once, never cached.
        admin
            .post_json(&tokens, &json!({"name": "ci", "scopes": [], "days": 30}))
            .await
            .assert_validation_error("scopes");
        admin
            .post_json(
                &tokens,
                &json!({"name": "ci", "scopes": ["everything"], "days": 30}),
            )
            .await
            .assert_validation_error("scopes");
        let made = admin
            .post_json(
                &tokens,
                &json!({"name": "ci", "scopes": ["read"], "days": 30}),
            )
            .await;
        made.assert_status(StatusCode::OK);
        assert!(
            made.headers
                .get("cache-control")
                .unwrap()
                .to_str()
                .unwrap()
                .contains("no-store")
        );
        let plain = made.json()["token"].as_str().unwrap().to_owned();
        let listed = admin.get(&tokens).await;
        assert!(!listed.text().contains(&plain));
        assert_eq!(listed.json()["tokens"][0]["scopes"], json!(["read"]));
        let id = listed.json()["tokens"][0]["id"].as_i64().unwrap();

        // Read: everything its user sees; nothing else (a super admin's
        // token still cannot deploy or read secrets).
        let reader = app.client().bearer(&plain);
        let seen = reader.get(&api("/projects")).await;
        seen.assert_status(StatusCode::OK);
        assert_eq!(seen.json()["projects"].as_array().unwrap().len(), 2);
        let one = reader.get(&api("/services/shop%2Fweb")).await;
        one.assert_status(StatusCode::OK);
        assert_eq!(one.json()["service"]["id"], web);
        reader
            .send(
                Method::POST,
                &api(&format!("/services/{web}/deploy")),
                Some(&json!({})),
            )
            .await
            .assert_status(StatusCode::FORBIDDEN);
        reader
            .get(&api(&format!("/services/{web}/env")))
            .await
            .assert_status(StatusCode::FORBIDDEN);
        // Revoked: no more.
        admin
            .send(Method::DELETE, &format!("{tokens}/{id}"), None)
            .await
            .assert_status(StatusCode::OK);
        app.client()
            .bearer(&plain)
            .get(&api("/projects"))
            .await
            .assert_status(StatusCode::UNAUTHORIZED);

        // Any API token signs in (`rdp login`), whatever its scopes.
        let deployer = app.with_token(&root, &["services.deploy"]).await;
        let me = deployer.get(&api("/me")).await;
        me.assert_status(StatusCode::OK);
        assert_eq!(me.json()["scopes"], json!(["deploy"]));
        deployer
            .get(&api("/projects"))
            .await
            .assert_status(StatusCode::FORBIDDEN);

        // A console token (`*`) is no API token here.
        app.with_token(&root, &["*"])
            .await
            .get(&api("/projects"))
            .await
            .assert_status(StatusCode::FORBIDDEN);

        // A member's token: their project only, as their role allows.
        let dev = support::user_with(&app, "dev", &[]).await;
        admin
            .post_json(
                &app.admin_api(&format!("x/apps/projects/{shop}/members")),
                &json!({"email": "dev@example.test", "role": "developer"}),
            )
            .await
            .assert_status(StatusCode::OK);
        let abilities = rdockpanel::services::api_token_service::abilities(&[
            "read".to_owned(),
            "deploy".to_owned(),
        ])
        .unwrap();
        let abilities: Vec<&str> = abilities.iter().map(String::as_str).collect();
        let member = app.with_token(&dev, &abilities).await;
        let seen = member.get(&api("/projects")).await.json();
        assert_eq!(seen["projects"].as_array().unwrap().len(), 1, "{seen}");
        assert_eq!(seen["projects"][0]["name"], "shop");
        // Another project's or none at all: the same answer.
        for path in [
            "/services/other%2Fapi",
            "/services/nope%2Fnone",
            "/services/999999",
        ] {
            member
                .get(&api(path))
                .await
                .assert_status(StatusCode::NOT_FOUND);
        }
        member
            .send(
                Method::POST,
                &api("/services/other%2Fapi/deploy"),
                Some(&json!({})),
            )
            .await
            .assert_status(StatusCode::NOT_FOUND);
        let queued = member
            .send(
                Method::POST,
                &api("/services/shop%2Fweb/deploy"),
                Some(&json!({})),
            )
            .await;
        queued.assert_status(StatusCode::ACCEPTED);
        let deployment = queued.json()["deployment"]["id"].as_i64().unwrap();
        let status = member
            .get(&api(&format!("/deployments/{deployment}")))
            .await;
        status.assert_status(StatusCode::OK);
        assert_eq!(status.json()["deployment"]["trigger"], "deploy");

        // A token with an ability beyond the scopes (console-made) is refused
        // whole, even beside known ones.
        app.with_token(&root, &["services.view", "users.delete"])
            .await
            .get(&api("/projects"))
            .await
            .assert_status(StatusCode::FORBIDDEN);
        // Related records with their own permission: a token for services
        // only sees no deployment (nor domains).
        let narrow = app.with_token(&root, &["services.view"]).await;
        let shown = narrow.get(&api(&format!("/services/{web}"))).await;
        shown.assert_status(StatusCode::OK);
        assert!(
            shown.json()["service"]["latest_deployment"].is_null(),
            "{}",
            shown.text()
        );
        assert_eq!(shown.json()["service"]["domains"], json!([]));
        let wide = app
            .with_token(&root, &["services.view", "deployments.view"])
            .await;
        let shown = wide.get(&api(&format!("/services/{web}"))).await.json();
        assert_eq!(shown["service"]["latest_deployment"]["id"], deployment);
        narrow
            .get(&api(&format!("/deployments/{deployment}")))
            .await
            .assert_status(StatusCode::FORBIDDEN);
        // Another project's deployment, to a member: as missing.
        let elsewhere = app.with_token(&dev, &["deployments.view"]).await;
        elsewhere
            .get(&api("/deployments/999999"))
            .await
            .assert_status(StatusCode::NOT_FOUND);
    }
);
