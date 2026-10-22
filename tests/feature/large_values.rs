//! The largest values the panel takes are kept whole (MySQL's TEXT holds
//! 64 KiB: Compose files, notes, the shared maintenance page);
//! sealed ones are refused over RustCMS's 32 KiB (an environment, a
//! service's files together), never failing on the way.

use rdockpanel::database::migrations::Migrator;
use rdockpanel::models::{service, service_mount};
use rustcms::axum::http::{Method, StatusCode};
use rustcms::crypto::Encrypted;
use rustcms_testing::{TestDb, db_test};
use sea_orm::{ActiveModelTrait, EntityTrait, Set};
use serde_json::json;

use crate::deployments::{FAST, action, create, last_deployment, records};
use crate::support::{self, fake_docker::FakeDocker};

/// `.env` text of `n` variables of `size` bytes each.
fn env_text(n: usize, size: usize) -> String {
    (0..n)
        .map(|i| format!("VAR_{i}={}\n", "v".repeat(size)))
        .collect()
}

db_test!(
    the_largest_values_are_kept_or_refused,
    migrate = Migrator,
    |db: TestDb| async move {
        let app = support::app_with_docker(db, FakeDocker::default()).await;
        let user = support::user_with(&app, "admin", &[]).await;
        app.make_super_admin(&user).await;
        let admin = app.acting_as(&user).await;
        let project = create(&app, &admin, "projects", json!({"name": "big"})).await;
        let service = create(
            &app,
            &admin,
            "services",
            json!({"project_id": project, "name": "web", "image": "nginx:1.27", "replicas": 1}),
        )
        .await;
        let id: i64 = service.parse().unwrap();
        let env = app.admin_api(&format!("x/apps/services/{service}/env"));
        let mounts = records(&app, "service_mounts");

        // An environment: 30 KiB kept, 64 KiB refused (sealed whole).
        admin
            .send(
                Method::PUT,
                &env,
                Some(&json!({ "text": env_text(64, 1024) })),
            )
            .await
            .assert_validation_error("text");
        admin
            .send(
                Method::PUT,
                &env,
                Some(&json!({ "text": env_text(30, 1000) })),
            )
            .await
            .assert_status(StatusCode::OK);

        // Files: one over 32 KiB refused; together over 32 KiB refused.
        let file = |target: &str, size: usize| json!({"service_id": service, "kind": "file", "target": target, "sealed_content": {"replace": "f".repeat(size)}});
        admin
            .post_json(&mounts, &file("/etc/huge.conf", 40_000))
            .await
            .assert_validation_error("sealed_content");
        create(&app, &admin, "service_mounts", file("/etc/a.conf", 20_000)).await;
        admin
            .post_json(&mounts, &file("/etc/b.conf", 20_000))
            .await
            .assert_validation_error("sealed_content");
        create(&app, &admin, "service_mounts", file("/etc/b.conf", 5_000)).await;

        // Notes at their length in characters of several bytes (over 64
        // KiB); a Dockerfile at its length in bytes.
        admin
            .send(
                Method::PATCH,
                &format!("{}/{service}", records(&app, "services")),
                Some(&json!({
                    "notes": "🚀".repeat(20_000),
                    "dockerfile": "é".repeat(32_767),
                })),
            )
            .await
            .assert_status(StatusCode::OK);
        let row = service::Entity::find_by_id(id)
            .one(app.state().db())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.notes.unwrap().chars().count(), 20_000);
        assert_eq!(row.dockerfile.unwrap().len(), 65_534);

        // The deployment keeps the environment and the files.
        action(&app, &admin, "services", "deploy", &service)
            .await
            .assert_status(StatusCode::OK);
        let queued = last_deployment(&app, id).await;
        assert!(queued.env.is_some() && queued.files.is_some());
        // Files over 32 KiB together (written before this check): the
        // deployment is refused with why, not failed.
        service_mount::ActiveModel {
            service_id: Set(id),
            kind: Set("file".to_owned()),
            target: Set("/etc/c.conf".to_owned()),
            read_only: Set(true),
            sealed_content: Set(Some(
                Encrypted::seal(
                    app.state().keyring(),
                    &service_mount::content_context(),
                    &"c".repeat(20_000),
                )
                .unwrap(),
            )),
            created_at: Set(chrono::Utc::now()),
            updated_at: Set(chrono::Utc::now()),
            ..Default::default()
        }
        .insert(app.state().db())
        .await
        .unwrap();
        rdockpanel::services::deployment_service::run(app.state(), queued.id, FAST)
            .await
            .unwrap();
        let refused = action(&app, &admin, "services", "deploy", &service).await;
        assert!(
            refused.text().contains("32 KiB together"),
            "{}",
            refused.text()
        );

        // A file renamed (its content kept) is checked with the others too;
        // a change of anything else is not.
        let other = create(
            &app,
            &admin,
            "services",
            json!({"project_id": project, "name": "files", "image": "nginx:1.27", "replicas": 1}),
        )
        .await;
        // `{"/a":"…"}`: exactly 32 KiB.
        let exact = create(
            &app,
            &admin,
            "service_mounts",
            json!({"service_id": other, "kind": "file", "target": "/a", "sealed_content": {"replace": "x".repeat(32_759)}}),
        )
        .await;
        let mount = format!("{mounts}/{exact}");
        admin
            .send(Method::PATCH, &mount, Some(&json!({"target": "/ab"})))
            .await
            .assert_validation_error("sealed_content");
        admin
            .send(Method::PATCH, &mount, Some(&json!({"read_only": false})))
            .await
            .assert_status(StatusCode::OK);
        // Renamed within it, and its content replaced (the old not counted).
        admin
            .send(Method::PATCH, &mount, Some(&json!({"target": "/b"})))
            .await
            .assert_status(StatusCode::OK);
        admin
            .send(
                Method::PATCH,
                &mount,
                Some(&json!({"sealed_content": {"replace": "y".repeat(32_759)}})),
            )
            .await
            .assert_status(StatusCode::OK);

        // A Compose file of 200 KiB.
        let compose = format!(
            "services:\n  web:\n    image: nginx:1.27\n    labels:\n      note: \"{}\"\n",
            "c".repeat(200 * 1024)
        );
        let made = admin
            .post_json(
                &app.admin_api(&format!("x/apps/projects/{project}/services")),
                &json!({"name": "stack", "kind": "compose", "compose": compose}),
            )
            .await;
        made.assert_status(StatusCode::OK);
        let stack = service::Entity::find_by_id(made.json()["id"].as_i64().unwrap())
            .one(app.state().db())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stack.compose.as_deref(), Some(compose.as_str()));

        // The shared maintenance page of 200 KiB.
        let page = format!("<p>{}</p>", "m".repeat(200 * 1024));
        admin
            .send(
                Method::PUT,
                &app.admin_api("x/apps/settings/maintenance"),
                Some(&json!({ "template": page, "contact": "" })),
            )
            .await
            .assert_status(StatusCode::OK);
        let read = admin
            .get(&app.admin_api("x/apps/settings/maintenance"))
            .await;
        assert_eq!(read.json()["template"], json!(page));

        // Back to TEXT: refused while a value is longer, nothing changed.
        {
            use sea_orm_migration::MigratorTrait;
            let undone = Migrator::down(app.state().db(), Some(1)).await;
            assert!(
                undone
                    .as_ref()
                    .is_err_and(|e| e.to_string().contains("over 64 KiB")),
                "{undone:?}"
            );
            let kept = service::Entity::find_by_id(stack.id)
                .one(app.state().db())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(kept.compose.as_deref(), Some(compose.as_str()));
        }

        // From a template: the same limits, as field errors.
        let schema = app.admin_api(&format!("x/apps/projects/{project}/schema"));
        let big_env = json!({"services": [{"type": "app", "data": {"serviceName": "envy", "source": {"type": "image", "image": "nginx:1.27"}, "env": env_text(64, 1024)}}]});
        admin
            .post_json(&schema, &json!({ "schema": big_env }))
            .await
            .assert_validation_error("env");
        let big_files = json!({"services": [{"type": "app", "data": {"serviceName": "filer", "source": {"type": "image", "image": "nginx:1.27"}, "mounts": [
            {"type": "file", "mountPath": "/etc/a.conf", "content": "a".repeat(20_000)},
            {"type": "file", "mountPath": "/etc/b.conf", "content": "b".repeat(20_000)},
        ]}}]});
        admin
            .post_json(&schema, &json!({ "schema": big_files }))
            .await
            .assert_validation_error("mounts");
    }
);
