//! GitHub tokens (Settings → Git): checked by GitHub, sealed, given to
//! projects; a service of such a project picks a repository and clones with
//! it — and no other project's service can. GitHub is a local stand-in.

use rdockpanel::database::migrations::Migrator;
use rdockpanel::models::git_provider;
use rdockpanel::services::github_service::GitHub;
use rustcms::axum::http::{HeaderMap, Method, StatusCode};
use rustcms::axum::response::IntoResponse;
use rustcms::axum::{Json, Router, routing::get};
use rustcms_testing::{TestApp, TestDb, db_test};
use sea_orm::EntityTrait;
use serde_json::{Value, json};

use crate::deployments::{action, create, last_deployment, records};
use crate::support::{self, fake_docker::FakeDocker};

const TOKEN: &str = "ghp_rdpTestToken0123456789";

fn signed_in(headers: &HeaderMap) -> bool {
    headers.get("authorization").and_then(|v| v.to_str().ok()) == Some(&format!("Bearer {TOKEN}"))
}

/// GitHub's API as the panel asks it: `octo`, its repositories, branches.
async fn github() -> String {
    let app = Router::new()
        .route(
            "/user",
            get(|headers: HeaderMap| async move {
                if !signed_in(&headers) {
                    return StatusCode::UNAUTHORIZED.into_response();
                }
                ([("x-oauth-scopes", "repo")], Json(json!({"login": "octo"}))).into_response()
            }),
        )
        .route(
            "/user/repos",
            get(|headers: HeaderMap| async move {
                if !signed_in(&headers) {
                    return StatusCode::UNAUTHORIZED.into_response();
                }
                Json(json!([
                    {"full_name": "octo/app", "private": true, "default_branch": "main"},
                    {"full_name": "octo/..", "private": false, "default_branch": "main"},
                ]))
                .into_response()
            }),
        )
        .route(
            "/repos/octo/app/branches",
            get(|headers: HeaderMap| async move {
                if !signed_in(&headers) {
                    return StatusCode::UNAUTHORIZED.into_response();
                }
                Json(json!([{"name": "main"}, {"name": "dev"}])).into_response()
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { rustcms::axum::serve(listener, app).await });
    url
}

async fn app(db: TestDb, docker: FakeDocker, api: String) -> TestApp {
    use rdockpanel::services::docker_service::Docker;
    let traefik = support::scratch_traefik();
    TestApp::new(
        move |config| {
            let mut app = rdockpanel::bootstrap::build(config);
            app.bind(Docker::new(docker));
            app.bind(traefik);
            app.bind(GitHub { api });
            app
        },
        db,
    )
    .await
}

db_test!(
    github_tokens_for_their_projects,
    migrate = Migrator,
    |db: TestDb| async move {
        let api = github().await;
        let app = app(db, FakeDocker::default(), api).await;
        let user = support::user_with(&app, "admin", &[]).await;
        app.make_super_admin(&user).await;
        let admin = app.acting_as(&user).await;
        let shop: i64 = create(&app, &admin, "projects", json!({"name": "shop"}))
            .await
            .parse()
            .unwrap();
        let other: i64 = create(&app, &admin, "projects", json!({"name": "other"}))
            .await
            .parse()
            .unwrap();
        let providers = app.admin_api("x/apps/git/providers");

        // GitHub must accept it.
        admin
            .post_json(&providers, &json!({"name": "company", "token": "wrong"}))
            .await
            .assert_validation_error("token");
        let added = admin
            .post_json(
                &providers,
                &json!({"name": "company", "token": TOKEN, "projects": [shop, 999]}),
            )
            .await;
        added.assert_status(StatusCode::OK);
        assert!(!added.text().contains(TOKEN));
        let added = added.json();
        assert_eq!(added["provider"]["account"], "octo");
        assert_eq!(added["provider"]["projects"], json!([shop]));
        assert_eq!(added["scopes"], "repo");
        let id = added["provider"]["id"].as_i64().unwrap();
        // Sealed: never in clear in the database, never listed.
        let row = git_provider::Entity::find_by_id(id)
            .one(app.state().db())
            .await
            .unwrap()
            .unwrap();
        assert!(!serde_json::to_string(&row.token).unwrap().contains(TOKEN));
        assert!(!admin.get(&providers).await.text().contains(TOKEN));
        admin
            .post_json(&providers, &json!({"name": "company", "token": TOKEN}))
            .await
            .assert_validation_error("name");

        // A service of the project picks a repository and its branch.
        let web: i64 = create(&app, &admin, "services", json!({"project_id": shop.to_string(), "name": "web", "replicas": 1, "source": "git", "git_url": "https://github.com/octo/app.git"})).await.parse().unwrap();
        let service = |id: i64| format!("{}/{id}", records(&app, "services"));
        let patch = |id: i64, body: Value| {
            let url = service(id);
            let admin = &admin;
            async move { admin.send(Method::PATCH, &url, Some(&body)).await }
        };
        assert!(
            patch(web, json!({"git_provider_id": id.to_string()}))
                .await
                .status
                .is_success()
        );
        patch(web, json!({"git_url": "https://gitlab.com/octo/app.git"}))
            .await
            .assert_validation_error("git_url");
        let base = app.admin_api(&format!("x/apps/services/{web}/github"));
        let given = admin.get(&base).await.json();
        assert_eq!(given["providers"][0]["account"], "octo");
        let repos = admin.get(&format!("{base}/{id}/repos")).await.json();
        assert_eq!(
            repos["repositories"].as_array().unwrap().len(),
            1,
            "{repos}"
        );
        assert_eq!(
            repos["repositories"][0]["url"],
            "https://github.com/octo/app.git"
        );
        let branches = admin
            .get(&format!("{base}/{id}/branches?repo=octo/app"))
            .await
            .json();
        assert_eq!(branches["branches"], json!(["main", "dev"]));

        // Another project's service: neither its token nor its repositories.
        let api_service: i64 = create(&app, &admin, "services", json!({"project_id": other.to_string(), "name": "api", "replicas": 1, "source": "git", "git_url": "https://github.com/octo/app.git"})).await.parse().unwrap();
        patch(api_service, json!({"git_provider_id": id.to_string()}))
            .await
            .assert_validation_error("git_provider_id");
        let elsewhere = app.admin_api(&format!("x/apps/services/{api_service}/github"));
        assert_eq!(admin.get(&elsewhere).await.json()["providers"], json!([]));
        admin
            .get(&format!("{elsewhere}/{id}/repos"))
            .await
            .assert_status(StatusCode::NOT_FOUND);

        // A developer (services.update) lists them; a viewer does not.
        let developer = support::user_with(
            &app,
            "developer",
            &["services.update", "services.view", "services.view_any"],
        )
        .await;
        let developer = app.acting_as(&developer).await;
        developer
            .get(&format!("{base}/{id}/repos"))
            .await
            .assert_status(StatusCode::OK);
        developer
            .get(&providers)
            .await
            .assert_status(StatusCode::FORBIDDEN);
        let viewer = support::user_with(&app, "viewer", &["services.view"]).await;
        app.acting_as(&viewer)
            .await
            .get(&base)
            .await
            .assert_status(StatusCode::FORBIDDEN);

        // Taken from the project: its deployments fail, saying why (nothing cloned).
        let changed = admin
            .send(
                Method::PATCH,
                &format!("{providers}/{id}"),
                Some(&json!({"projects": []})),
            )
            .await;
        changed.assert_status(StatusCode::OK);
        action(&app, &admin, "services", "deploy", &web.to_string())
            .await
            .assert_status(StatusCode::OK);
        let queued = last_deployment(&app, web).await;
        rdockpanel::services::deployment_service::run(
            app.state(),
            queued.id,
            crate::deployments::FAST,
        )
        .await
        .unwrap();
        let failed = last_deployment(&app, web).await;
        assert_eq!(failed.status, "failed");
        assert!(
            failed.error.unwrap_or_default().contains("GitHub token"),
            "{:?}",
            failed.log
        );
        assert!(!failed.log.unwrap_or_default().contains("cloning"));

        // In use: kept; then removed.
        admin
            .send(Method::DELETE, &format!("{providers}/{id}"), None)
            .await
            .assert_validation_error("provider");
        assert!(
            patch(web, json!({"git_provider_id": null}))
                .await
                .status
                .is_success()
        );
        admin
            .send(Method::DELETE, &format!("{providers}/{id}"), None)
            .await
            .assert_status(StatusCode::OK);
    }
);
