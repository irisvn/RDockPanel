//! Database backups: storage providers, a database's backup configurations,
//! runs by hand and by schedule (a FakeDocker "dumps"), retention, restore.

use std::io::Read;
use std::time::Duration;

use chrono::Utc;
use rdockpanel::database::migrations::Migrator;
use rdockpanel::models::{backup_run, database_backup};
use rdockpanel::services::backup_service;
use rdockpanel::services::deployment_service::{self, Timings};
use rustcms::axum::http::StatusCode;
use rustcms_testing::{TestApp, TestClient, TestDb, db_test};
use sea_orm::{ActiveModelTrait, EntityTrait, IntoActiveModel, Set};
use serde_json::{Value, json};

use crate::deployments::{action, create, last_deployment, records};
use crate::support::{self, fake_docker::FakeDocker};

const FAST: Timings = Timings {
    poll: Duration::from_millis(5),
    settle: Duration::from_millis(20),
    timeout: Duration::from_secs(2),
};

/// A super admin, a project `shop` with a deployed PostgreSQL service `db`,
/// and a local storage provider in a scratch directory.
async fn setup(app: &TestApp) -> (TestClient<'_>, i64, String, std::path::PathBuf) {
    let user = support::user_with(app, "admin", &[]).await;
    app.make_super_admin(&user).await;
    let admin = app.acting_as(&user).await;
    let project = create(app, &admin, "projects", json!({"name": "shop"})).await;
    let made = admin
        .post_json(
            &app.admin_api(&format!("x/apps/projects/{project}/services")),
            &json!({"name": "db", "kind": "postgres", "database": {"database": "shop"}}),
        )
        .await;
    made.assert_status(StatusCode::OK);
    let id = made.json()["id"].as_i64().unwrap();
    action(app, &admin, "services", "deploy", &id.to_string())
        .await
        .assert_status(StatusCode::OK);
    let queued = last_deployment(app, id).await;
    deployment_service::run(app.state(), queued.id, FAST)
        .await
        .unwrap();
    let root = std::env::temp_dir().join(format!(
        "rdp-backups-{}",
        rdockpanel::services::database_service::random_string(10).to_lowercase()
    ));
    let provider = create(
        app,
        &admin,
        "storage_providers",
        json!({"name": "disk", "kind": "local", "local_path": root.to_string_lossy()}),
    )
    .await;
    (admin, id, provider, root)
}

async fn run_last(app: &TestApp, service: i64) -> backup_run::Model {
    let runs = backup_run::Entity::find()
        .all(app.state().db())
        .await
        .unwrap();
    let last = runs
        .into_iter()
        .filter(|r| r.service_id == service)
        .max_by_key(|r| r.id)
        .unwrap();
    backup_service::run(app.state(), last.id).await.unwrap();
    backup_run::Entity::find_by_id(last.id)
        .one(app.state().db())
        .await
        .unwrap()
        .unwrap()
}

fn gunzip(path: &std::path::Path) -> String {
    let mut text = String::new();
    flate2::read::GzDecoder::new(std::fs::File::open(path).unwrap())
        .read_to_string(&mut text)
        .unwrap();
    text
}

db_test!(
    a_database_is_backed_up_and_restored,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::default();
        let app = support::app_with_docker(db, fake.clone()).await;
        let (admin, id, provider, root) = setup(&app).await;

        // A provider: an absolute directory, or a bucket with its keys.
        admin
            .post_json(
                &records(&app, "storage_providers"),
                &json!({"name": "rel", "kind": "local", "local_path": "backups"}),
            )
            .await
            .assert_validation_error("local_path");
        let s3 = admin
            .post_json(
                &records(&app, "storage_providers"),
                &json!({"name": "s3", "kind": "s3", "endpoint": "file:///x", "secret_key": {"replace": "S3CRET"}}),
            )
            .await;
        s3.assert_validation_error("endpoint");
        s3.assert_validation_error("bucket");
        let checked = admin
            .post_json(
                &app.admin_api(&format!("x/apps/storage-providers/{provider}/check")),
                &json!({}),
            )
            .await;
        checked.assert_status(StatusCode::OK);
        assert_eq!(checked.json()["ok"], true, "{}", checked.text());

        // A configuration: a database service, a cron, a folder.
        let config = json!({
            "service_id": id.to_string(),
            "storage_provider_id": provider,
            "database_name": "shop",
            "enabled": true,
            "schedule": "0 2 * * *",
            "retention": 2,
            "path": "prod/shop",
        });
        for (field, value) in [
            ("schedule", json!("every day")),
            ("path", json!("../etc")),
            ("database_name", json!("--all-databases")),
            ("retention", json!(0)),
        ] {
            let mut bad = config.clone();
            bad[field] = value;
            admin
                .post_json(&records(&app, "database_backups"), &bad)
                .await
                .assert_validation_error(field);
        }
        let backup: i64 = create(&app, &admin, "database_backups", config.clone())
            .await
            .parse()
            .unwrap();

        // By hand: dumped in the database's container, gzipped, written.
        let asked = admin
            .post_json(
                &app.admin_api(&format!("x/apps/backups/{backup}/run")),
                &json!({}),
            )
            .await;
        asked.assert_status(StatusCode::ACCEPTED);
        let first = run_last(&app, id).await;
        assert_eq!(
            first.status, "succeeded",
            "{:?} {:?}",
            first.error, first.log
        );
        let file = first.file.clone().unwrap();
        assert!(
            file.starts_with("prod/shop/shop_db.shop.") && file.ends_with(".sql.gz"),
            "{file}"
        );
        assert_eq!(gunzip(&root.join(&file)), "dump of ctr-shop_db-1\n");
        let calls = fake.calls();
        let dump = calls
            .iter()
            .find(|c| c.starts_with("run ctr-shop_db-1:"))
            .unwrap();
        assert!(
            dump.contains("pg_dump") && dump.ends_with(" shop"),
            "{dump}"
        );
        assert!(!dump.contains("PASSWORD=") || dump.contains("\"$POSTGRES_PASSWORD\""));

        // Retention: two files of this service's database kept; others
        // (another service's, or not a backup) left alone.
        std::fs::write(root.join("prod/shop/notes.txt"), "mine").unwrap();
        std::fs::write(
            root.join("prod/shop/other_db.shop.20200101T000000Z.r0000000001.sql.gz"),
            "theirs",
        )
        .unwrap();
        for _ in 0..2 {
            admin
                .post_json(
                    &app.admin_api(&format!("x/apps/backups/{backup}/run")),
                    &json!({}),
                )
                .await
                .assert_status(StatusCode::ACCEPTED);
            assert_eq!(run_last(&app, id).await.status, "succeeded");
        }
        let mut kept: Vec<String> = std::fs::read_dir(root.join("prod/shop"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        kept.sort();
        assert_eq!(kept.len(), 4, "{kept:?}");
        assert!(kept.contains(&"notes.txt".to_owned()));
        assert!(kept.iter().any(|k| k.starts_with("other_db.")));
        assert!(
            !kept.iter().any(|k| file.ends_with(k.as_str())),
            "the oldest went"
        );

        // The tab: configurations, runs, providers; a run's log.
        let tab = admin
            .get(&app.admin_api(&format!("x/apps/services/{id}/backups")))
            .await;
        tab.assert_status(StatusCode::OK);
        let tab = tab.json();
        assert_eq!(tab["configs"][0]["database_name"], "shop");
        assert_eq!(tab["runs"].as_array().unwrap().len(), 3);
        assert_eq!(tab["providers"][0]["name"], "disk");
        let shown = admin
            .get(&app.admin_api(&format!("x/apps/backup-runs/{}", first.id)))
            .await
            .json();
        assert!(
            shown["log"].as_str().unwrap().contains("uploaded"),
            "{shown}"
        );

        // Restore: the files listed, one fed (gunzipped) to psql.
        let listed = admin
            .get(&app.admin_api(&format!(
                "x/apps/storage-providers/{provider}/files?path=prod/shop"
            )))
            .await
            .json();
        let newest = listed["files"][0]["file"].as_str().unwrap().to_owned();
        assert!(newest.ends_with(".sql.gz"), "{listed}");
        for (body, field) in [
            (
                json!({"storage_provider_id": provider.parse::<i64>().unwrap(), "file": "../x.sql.gz", "database": "shop"}),
                "file",
            ),
            (
                json!({"storage_provider_id": provider.parse::<i64>().unwrap(), "file": newest, "database": "a b"}),
                "database",
            ),
        ] {
            admin
                .post_json(
                    &app.admin_api(&format!("x/apps/services/{id}/restore")),
                    &body,
                )
                .await
                .assert_validation_error(field);
        }
        admin
            .post_json(
                &app.admin_api(&format!("x/apps/services/{id}/restore")),
                &json!({"storage_provider_id": provider.parse::<i64>().unwrap(), "file": newest, "database": "shop"}),
            )
            .await
            .assert_status(StatusCode::ACCEPTED);
        let restored = run_last(&app, id).await;
        assert_eq!(
            restored.status, "succeeded",
            "{:?} {:?}",
            restored.error, restored.log
        );
        assert_eq!(restored.kind, "restore");
        assert_eq!(
            String::from_utf8(fake.fed.lock().unwrap().clone()).unwrap(),
            "dump of ctr-shop_db-1\n"
        );
        assert!(fake.calls().iter().any(|c| c.contains("psql")));

        // A corrupt file fails before anything reaches the database.
        std::fs::write(root.join("prod/shop/broken.sql.gz"), b"\x1f\x8bnot gzip").unwrap();
        let runs_before = fake.calls().iter().filter(|c| c.contains("psql")).count();
        admin
            .post_json(
                &app.admin_api(&format!("x/apps/services/{id}/restore")),
                &json!({"storage_provider_id": provider.parse::<i64>().unwrap(), "file": "prod/shop/broken.sql.gz", "database": "shop"}),
            )
            .await
            .assert_status(StatusCode::ACCEPTED);
        let broken = run_last(&app, id).await;
        assert_eq!(broken.status, "failed");
        assert!(broken.error.unwrap().contains("not a valid gzip"));
        assert_eq!(
            fake.calls().iter().filter(|c| c.contains("psql")).count(),
            runs_before
        );

        // One run of a service at a time: another one running fails this one.
        let mut busy = backup_run::Entity::find_by_id(restored.id)
            .one(app.state().db())
            .await
            .unwrap()
            .unwrap()
            .into_active_model();
        busy.status = Set("running".to_owned());
        busy.updated_at = Set(Utc::now());
        busy.update(app.state().db()).await.unwrap();
        admin
            .post_json(
                &app.admin_api(&format!("x/apps/backups/{backup}/run")),
                &json!({}),
            )
            .await
            .assert_status(StatusCode::ACCEPTED);
        let refused = run_last(&app, id).await;
        assert_eq!(refused.status, "failed");
        assert!(refused.error.unwrap().contains("another backup"));
        // A run whose process died (no heartbeat) is failed by the tick.
        let mut dead = backup_run::Entity::find_by_id(restored.id)
            .one(app.state().db())
            .await
            .unwrap()
            .unwrap()
            .into_active_model();
        dead.updated_at = Set(Utc::now() - chrono::Duration::minutes(10));
        dead.update(app.state().db()).await.unwrap();
        backup_service::tick(app.state()).await.unwrap();
        let dead = backup_run::Entity::find_by_id(restored.id)
            .one(app.state().db())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(dead.status, "failed");
        // Its command killed in the container (a restore never goes on).
        assert!(
            fake.calls()
                .iter()
                .any(|c| c.starts_with("kill ctr-shop_db-1 rdp-backup-")),
            "{:?}",
            fake.calls()
        );

        // A failing dump fails the run, with the engine's message.
        *fake.exec_exit.lock().unwrap() = 1;
        admin
            .post_json(
                &app.admin_api(&format!("x/apps/backups/{backup}/run")),
                &json!({}),
            )
            .await
            .assert_status(StatusCode::ACCEPTED);
        let failed = run_last(&app, id).await;
        assert_eq!(failed.status, "failed");
        assert!(failed.error.unwrap().contains("exited with 1"));
        assert!(failed.log.unwrap().contains("access denied"));

        // A provider in use cannot be deleted.
        admin
            .send(
                rustcms::axum::http::Method::DELETE,
                &format!("{}/{provider}", records(&app, "storage_providers")),
                None,
            )
            .await
            .assert_status(StatusCode::FORBIDDEN);
        let _ = std::fs::remove_dir_all(&root);
    }
);

db_test!(
    a_schedule_runs_each_slot_once,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::default();
        let app = support::app_with_docker(db, fake.clone()).await;
        let (admin, id, provider, root) = setup(&app).await;
        let mut config = json!({
            "service_id": id.to_string(),
            "storage_provider_id": provider,
            "database_name": "shop",
            "enabled": true,
            "schedule": "0 2 * * *",
            "retention": 5,
            "path": "nightly",
        });
        let on: i64 = create(&app, &admin, "database_backups", config.clone())
            .await
            .parse()
            .unwrap();
        config["enabled"] = json!(false);
        create(&app, &admin, "database_backups", config).await;
        // Made two days ago: its last slot (today or yesterday 02:00) is due.
        let row = database_backup::Entity::find_by_id(on)
            .one(app.state().db())
            .await
            .unwrap()
            .unwrap();
        let mut active = row.into_active_model();
        active.created_at = Set(Utc::now() - chrono::Duration::days(2));
        active.update(app.state().db()).await.unwrap();

        assert_eq!(backup_service::tick(app.state()).await.unwrap(), 1);
        assert_eq!(
            backup_service::tick(app.state()).await.unwrap(),
            0,
            "once per slot"
        );
        let run = run_last(&app, id).await;
        assert_eq!(
            (run.trigger.as_str(), run.status.as_str()),
            ("schedule", "succeeded")
        );
        let _ = std::fs::remove_dir_all(&root);
    }
);

db_test!(
    only_databases_are_backed_up,
    migrate = Migrator,
    |db: TestDb| async move {
        let app = support::app_with_docker(db, FakeDocker::default()).await;
        let (admin, _, provider, _) = setup(&app).await;
        let project = rdockpanel::models::project::Entity::find()
            .one(app.state().db())
            .await
            .unwrap()
            .unwrap()
            .id;
        let mut ids: Vec<Value> = vec![];
        for body in [
            json!({"name": "cache", "kind": "redis"}),
            json!({"name": "web", "kind": "app", "image": "nginx:1.27"}),
        ] {
            let made = admin
                .post_json(
                    &app.admin_api(&format!("x/apps/projects/{project}/services")),
                    &body,
                )
                .await;
            made.assert_status(StatusCode::OK);
            ids.push(made.json()["id"].clone());
        }
        for id in ids {
            admin
                .post_json(
                    &records(&app, "database_backups"),
                    &json!({"service_id": id.to_string(), "storage_provider_id": provider, "database_name": "x", "schedule": "0 2 * * *", "retention": 1, "path": "x"}),
                )
                .await
                .assert_validation_error("service_id");
        }
    }
);
