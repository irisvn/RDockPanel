//! EasyPanel templates: the catalog, a template's form, its services created
//! (and deployed) in a project, "Create from Schema". The repository is
//! replaced by fixed files; `PANEL_TEST_TEMPLATES=1` also runs every real
//! template's generator (network) and counts what can be created.

use std::collections::HashMap;

use rdockpanel::database::migrations::Migrator;
use rdockpanel::models::{deployment, service};
use rdockpanel::services::template_service::Templates;
use rustcms::axum::http::StatusCode;
use rustcms_testing::{TestApp, TestDb, db_test};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::json;

use crate::deployments::create;
use crate::support::{self, fake_docker::FakeDocker};

const META: &str = r#"
name: Ghost
description: A blog.
instructions: Open it and create the admin.
links:
  - label: Website
    url: https://ghost.org/
schema:
  type: object
  required: [appServiceName, databaseServiceName]
  properties:
    appServiceName: { type: string, title: App Service Name, default: ghost }
    appServiceImage: { type: string, title: App Service Image, default: "ghost:6" }
    databaseServiceName: { type: string, title: Database Service Name, default: ghost-db }
"#;

const INDEX: &str = r#"
import { Output, randomPassword, Services } from "~templates-utils";
import { Input } from "./meta";

export function generate(input: Input): Output {
  const services: Services = [];
  const databasePassword = randomPassword();
  services.push({
    type: "app",
    data: {
      serviceName: input.appServiceName,
      source: { type: "image", image: input.appServiceImage },
      domains: [{ host: "$(EASYPANEL_DOMAIN)", port: 2368 }],
      mounts: [{ type: "volume", name: "content", mountPath: "/var/lib/ghost/content" }],
      env: [
        `url=https://$(PRIMARY_DOMAIN)`,
        `database__connection__host=$(PROJECT_NAME)_${input.databaseServiceName}`,
        `database__connection__password=${databasePassword}`,
      ].join("\n"),
    },
  });
  services.push({ type: "mysql", data: { serviceName: input.databaseServiceName, password: databasePassword } });
  return { services };
}
"#;

fn fixed() -> Templates {
    Templates::fixed(HashMap::from([
        (
            "list.json".to_owned(),
            json!([{"slug": "ghost", "logo": "logo.png", "name": "Ghost", "description": "A blog.", "tags": ["Blog"]}]).to_string(),
        ),
        ("ghost/meta.yaml".to_owned(), META.to_owned()),
        ("ghost/index.ts".to_owned(), INDEX.to_owned()),
    ]))
}

async fn app_with_templates(db: TestDb, fake: FakeDocker) -> TestApp {
    use rdockpanel::services::docker_service::Docker;
    let traefik = support::scratch_traefik();
    TestApp::new(
        move |config| {
            let mut app = rdockpanel::bootstrap::build(config);
            app.bind(Docker::new(fake));
            app.bind(traefik);
            app.bind(fixed());
            app
        },
        db,
    )
    .await
}

db_test!(
    a_template_makes_its_services,
    migrate = Migrator,
    |db: TestDb| async move {
        let app = app_with_templates(db, FakeDocker::default()).await;
        let user = support::user_with(&app, "admin", &[]).await;
        app.make_super_admin(&user).await;
        let admin = app.acting_as(&user).await;
        let project = create(&app, &admin, "projects", json!({"name": "blog"})).await;

        let list = admin.get(&app.admin_api("x/apps/templates")).await;
        list.assert_status(StatusCode::OK);
        assert_eq!(list.json()["templates"][0]["slug"], "ghost");
        assert_eq!(list.json()["templates"][0]["logo_file"], "logo.png");
        let page = admin
            .get(&app.admin_api("x/apps/templates/ghost"))
            .await
            .json();
        assert_eq!(
            page["schema"]["properties"]["appServiceName"]["default"],
            "ghost"
        );
        assert_eq!(page["instructions"], "Open it and create the admin.");
        admin
            .get(&app.admin_api("x/apps/templates/..%2Fx"))
            .await
            .assert_status(StatusCode::BAD_REQUEST);

        // The form's defaults fill what is not sent; deployed at once.
        let made = admin
            .post_json(
                &app.admin_api(&format!("x/apps/projects/{project}/templates/ghost")),
                &json!({"input": {"appServiceName": "site"}, "deploy": true}),
            )
            .await;
        made.assert_status(StatusCode::OK);
        let made = made.json();
        let names: Vec<&str> = made["services"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["ghost-db", "site"], "the database first");
        let site = made["services"][1]["id"].as_i64().unwrap();
        let page = admin
            .get(&app.admin_api(&format!("x/apps/services/{site}")))
            .await
            .json();
        // Its generated domain, when there are generated domains; else said.
        let host = &rdockpanel::services::deploy_rules_service::config().service_domain;
        if host.is_empty() {
            assert_eq!(page["domains"], json!([]));
            let warnings = made["warnings"].to_string();
            assert!(warnings.contains("service_domain"), "{warnings}");
        } else {
            assert_eq!(page["domains"][0]["host"], format!("blog-site.{host}"));
            assert_eq!(page["domains"][0]["port"], 2368);
        }
        assert_eq!(page["mounts"][0]["name"], "content");
        let env = admin
            .get(&app.admin_api(&format!("x/apps/services/{site}/env")))
            .await
            .json();
        let text = env["text"].as_str().unwrap();
        assert!(
            text.contains("database__connection__host=$(PROJECT_NAME)_ghost-db"),
            "{text}"
        );
        // The database's password is the app's.
        let db_id = made["services"][0]["id"].as_i64().unwrap();
        let secret = admin
            .get(&app.admin_api(&format!("x/apps/services/{db_id}/credentials")))
            .await
            .json();
        let password = secret["password"].as_str().unwrap();
        assert!(text.contains(&format!("password={password}")));
        let queued = deployment::Entity::find()
            .filter(deployment::Column::ServiceId.is_in([site, db_id]))
            .all(app.state().db())
            .await
            .unwrap();
        assert_eq!(queued.len(), 2);

        // Again: the names are taken, nothing is half made.
        let again = admin
            .post_json(
                &app.admin_api(&format!("x/apps/projects/{project}/templates/ghost")),
                &json!({"input": {}}),
            )
            .await;
        again.assert_validation_error("name");
        let services = service::Entity::find().all(app.state().db()).await.unwrap();
        assert_eq!(services.len(), 2);

        // Create from Schema: the same output, pasted.
        let schema = json!({"services": [{"type": "redis", "data": {"serviceName": "cache"}}]});
        admin
            .post_json(
                &app.admin_api(&format!("x/apps/projects/{project}/schema")),
                &json!({"schema": schema}),
            )
            .await
            .assert_status(StatusCode::OK);
        // Basic authentication, as EasyPanel's: made with it (hashed), deployed.
        let guarded = json!({"services": [{"type": "app", "data": {"serviceName": "registry", "source": {"type": "image", "image": "registry:2"}, "basicAuth": [{"username": "u", "password": "p4ssword"}], "redirects": [{"regex": "^http://old/(.*)", "replacement": "http://new/${1}", "permanent": true}]}}]});
        let made = admin
            .post_json(
                &app.admin_api(&format!("x/apps/projects/{project}/schema")),
                &json!({"schema": guarded, "deploy": true}),
            )
            .await;
        made.assert_status(StatusCode::OK);
        let registry = made.json()["services"][0]["id"].as_i64().unwrap();
        let page = admin
            .get(&app.admin_api(&format!("x/apps/services/{registry}")))
            .await;
        assert_eq!(page.json()["basic_auth"][0]["username"], "u");
        assert_eq!(page.json()["redirects"][0]["permanent"], true);
        assert!(
            !page.text().contains("p4ssword") && !page.text().contains("$2"),
            "{}",
            page.text()
        );
        // Domains need service_domains.create.
        let creator =
            support::user_with(&app, "creator", &["projects.view", "services.create"]).await;
        let creator = app.acting_as(&creator).await;
        let routed = json!({"services": [{"type": "app", "data": {"serviceName": "w", "source": {"type": "image", "image": "nginx"}, "domains": [{"host": "w.example.com"}]}}]});
        creator
            .post_json(
                &app.admin_api(&format!("x/apps/projects/{project}/schema")),
                &json!({"schema": routed}),
            )
            .await
            .assert_status(StatusCode::FORBIDDEN);
        // Added capabilities: the panel's administrators only.
        let capable = json!({"services": [{"type": "app", "data": {"serviceName": "vpn2", "source": {"type": "image", "image": "wg:1"}, "deploy": {"capAdd": ["NET_ADMIN"]}}}]});
        creator
            .post_json(
                &app.admin_api(&format!("x/apps/projects/{project}/schema")),
                &json!({"schema": capable}),
            )
            .await
            .assert_status(StatusCode::FORBIDDEN);
        // Deploying needs services.deploy.
        let maker = support::user_with(&app, "maker", &["projects.view", "services.create"]).await;
        let maker = app.acting_as(&maker).await;
        maker
            .post_json(
                &app.admin_api(&format!("x/apps/projects/{project}/schema")),
                &json!({"schema": {"services": [{"type": "redis", "data": {"serviceName": "q"}}]}, "deploy": true}),
            )
            .await
            .assert_status(StatusCode::FORBIDDEN);
    }
);

/// Every real template: generated, then planned (what a project would get).
#[tokio::test]
async fn every_real_template_runs() {
    if std::env::var_os("PANEL_TEST_TEMPLATES").is_none() {
        return;
    }
    use rdockpanel::services::{template_engine, template_service};
    let templates =
        Templates::new("https://raw.githubusercontent.com/easypanel-io/templates/main/templates");
    let list = templates.list().await.unwrap();
    let (mut ok, mut failed) = (0, vec![]);
    for t in &list {
        let slug = t["slug"].as_str().unwrap().to_owned();
        let result = async {
            let (meta, code) = templates.template(&slug).await?;
            let mut input = serde_json::Map::new();
            if let Some(props) = meta
                .pointer("/schema/properties")
                .and_then(|p| p.as_object())
            {
                for (k, v) in props {
                    let value = v.get("default").cloned().unwrap_or_else(|| {
                        match v.get("type").and_then(|t| t.as_str()) {
                            Some("boolean") => json!(false),
                            Some("number") | Some("integer") => json!(1),
                            _ => json!("x"),
                        }
                    });
                    input.insert(k.clone(), value);
                }
            }
            let input = serde_json::Value::Object(input);
            let output = template_engine::run_isolated(
                &template_engine::Runner::InProcess,
                &code,
                &meta,
                &input,
            )
            .await?;
            template_service::plan(&output, "p", Some("example.com")).map(|_| ())
        }
        .await;
        match result {
            Ok(()) => ok += 1,
            Err(e) => failed.push(format!("{slug}: {e}")),
        }
    }
    eprintln!(
        "{ok} of {} templates run; failed:\n{}",
        list.len(),
        failed.join("\n")
    );
    assert!(ok * 100 >= list.len() * 95, "{ok} of {}", list.len());
}
