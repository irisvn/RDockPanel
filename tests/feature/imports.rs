//! Importing existing containers (docs/import.md).

use rdockpanel::database::migrations::Migrator;
use rdockpanel::services::docker_service::ContainerItem;
use rustcms::axum::http::{Method, StatusCode};
use rustcms_testing::{TestApp, TestClient, TestDb, db_test};
use sea_orm::ConnectionTrait;
use serde_json::{Value, json};

use crate::support::{self, fake_docker::FakeDocker};

db_test!(
    the_import_tables_are_the_panels_own,
    migrate = Migrator,
    |db: TestDb| async move {
        // RustCMS has an `imports` table (CSV files): a table of the same name
        // would be skipped by `create_table` (if not exists) without a word.
        let app = support::app_with_docker(db, FakeDocker::default()).await;
        let db = app.state().db();
        for query in [
            "SELECT state, lease_token, plan FROM container_imports",
            "SELECT choice, original_restart FROM container_import_items",
            "SELECT panel_volume, copied_at FROM container_import_volumes",
            "SELECT container_id, volume FROM container_import_claims",
            "SELECT backup_path, sha256 FROM container_import_files",
            "SELECT admin_only FROM services",
            "SELECT link_target FROM service_mounts",
        ] {
            db.execute_unprepared(query)
                .await
                .unwrap_or_else(|e| panic!("{query}: {e}"));
        }
    }
);

const SECRET: &str = "hunter2-do-not-leak";

/// A Compose project `shop`: `web` (a secret in its env and command) and
/// `db`, sharing the volume `shared`; and `tika`, alone in another project.
fn shop(fake: &FakeDocker) {
    let container = |name: &str, service: &str, project: &str, env: Value, volume: bool| {
        let id = format!("id-{service}");
        fake.containers.lock().unwrap().push(ContainerItem {
            id: id.clone(),
            name: name.to_owned(),
            image: format!("{service}:1"),
            image_id: format!("sha256:{service}"),
            state: "running".to_owned(),
            status: "Up".to_owned(),
            created: 0,
            ports: vec![],
            labels: Default::default(),
        });
        let labels = json!({
            "com.docker.compose.project": project,
            "com.docker.compose.service": service,
        });
        fake.detailed.lock().unwrap().push(
            json!({"Id": id, "Names": [format!("/{name}")], "Image": format!("{service}:1"),
                "State": "running", "Labels": labels}),
        );
        let mounts = if volume {
            json!([{"Type": "volume", "Name": "shared", "Destination": "/data", "RW": true}])
        } else {
            json!([])
        };
        fake.inspected.lock().unwrap().insert(
            name.to_owned(),
            json!({
                "Id": id, "Name": format!("/{name}"), "Image": format!("sha256:{service}"),
                "State": {"Status": "running"},
                "Config": {"Image": format!("{service}:1"), "Env": env, "Labels": labels,
                    "Cmd": ["serve", "--requirepass", SECRET]},
                "HostConfig": {"RestartPolicy": {"Name": "always"}, "NetworkMode": "default"},
                "Mounts": mounts,
                "NetworkSettings": {"Ports": {}, "Networks": {
                    format!("{project}_default"): {"Aliases": [service], "IPAddress": "172.20.0.9"}}},
            }),
        );
    };
    container(
        "shop-web-1",
        "web",
        "shop",
        json!([
            format!("DB_PASSWORD={SECRET}"),
            "DATABASE_URL=postgres://u:p@db:5432/x"
        ]),
        true,
    );
    container("shop-db-1", "db", "shop", json!([]), true);
    container("other-tika-1", "tika", "other", json!([]), false);
}

async fn manager(app: &TestApp) -> TestClient<'_> {
    let user = support::user_with(app, "manager", &["system.manage"]).await;
    app.acting_as(&user).await
}

async fn wait_planned(app: &TestApp, admin: &TestClient<'_>, id: i64) -> Value {
    rustcms::queue::run_until_idle(app.state(), None, 20)
        .await
        .unwrap();
    let got = admin.get(&app.admin_api(&format!("x/imports/{id}"))).await;
    got.assert_status(StatusCode::OK);
    got.json()
}

db_test!(
    an_import_is_read_planned_chosen_and_deleted,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::default();
        shop(&fake);
        let app = support::app_with_docker(db, fake.clone()).await;
        let admin = manager(&app).await;

        // The sources: grouped by Compose project, names only.
        let sources = admin.get(&app.admin_api("x/imports/sources")).await;
        sources.assert_status(StatusCode::OK);
        let groups: Vec<String> = sources.json()["groups"]
            .as_array()
            .unwrap()
            .iter()
            .map(|g| g["project"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(groups, ["other", "shop"]);

        // Reading a project: 202, then the job writes the plan.
        let made = admin
            .post_json(
                &app.admin_api("x/imports"),
                &json!({"source_kind": "project", "source_ref": "shop"}),
            )
            .await;
        made.assert_status(StatusCode::ACCEPTED);
        let id = made.json()["id"].as_i64().unwrap();
        let read = wait_planned(&app, &admin, id).await;
        assert_eq!(read["state"], "planned", "{read}");
        let items = read["plan"]["items"].as_array().unwrap();
        let mut names: Vec<&str> = items.iter().map(|i| i["name"].as_str().unwrap()).collect();
        names.sort();
        assert_eq!(names, ["shop-db-1", "shop-web-1"]);
        assert!(items.iter().all(|i| i["choice"] == "bring"));
        // No answer, plan or row carries a secret.
        assert!(!read.to_string().contains(SECRET), "{read}");
        assert!(!sources.text().contains(SECRET));
        let plan_row: (Option<String>,) = {
            use rdockpanel::models::container_import;
            use sea_orm::EntityTrait;
            let row = container_import::Entity::find_by_id(id)
                .one(app.state().db())
                .await
                .unwrap()
                .unwrap();
            (row.plan,)
        };
        assert!(!plan_row.0.unwrap().contains(SECRET));

        // They share writable data: neither can be left behind.
        let url = app.admin_api(&format!("x/imports/{id}/plan"));
        admin
            .send(
                Method::PATCH,
                &url,
                Some(&json!({"choices": {"id-web": "leave"}})),
            )
            .await
            .assert_validation_error("choices");
        // The project's name: valid ones only.
        admin
            .send(
                Method::PATCH,
                &url,
                Some(&json!({"project_name": "Bad Name"})),
            )
            .await
            .assert_validation_error("project_name");
        admin
            .send(Method::PATCH, &url, Some(&json!({"project_name": "rdp-x"})))
            .await
            .assert_validation_error("project_name");
        let renamed = admin
            .send(Method::PATCH, &url, Some(&json!({"project_name": "store"})))
            .await;
        renamed.assert_status(StatusCode::OK);
        assert_eq!(renamed.json()["plan"]["project_name"], "store");

        // Read again, then deleted (it never started).
        admin
            .post_json(
                &app.admin_api(&format!("x/imports/{id}/rescan")),
                &json!({}),
            )
            .await
            .assert_status(StatusCode::ACCEPTED);
        assert_eq!(wait_planned(&app, &admin, id).await["state"], "planned");
        admin
            .send(
                Method::DELETE,
                &app.admin_api(&format!("x/imports/{id}")),
                None,
            )
            .await
            .assert_status(StatusCode::OK);
        admin
            .get(&app.admin_api(&format!("x/imports/{id}")))
            .await
            .assert_status(StatusCode::NOT_FOUND);
    }
);

db_test!(
    an_import_of_nothing_is_a_failed_read,
    migrate = Migrator,
    |db: TestDb| async move {
        let app = support::app_with_docker(db, FakeDocker::default()).await;
        let admin = manager(&app).await;
        let made = admin
            .post_json(
                &app.admin_api("x/imports"),
                &json!({"source_kind": "container", "source_ref": "ghost"}),
            )
            .await;
        made.assert_status(StatusCode::ACCEPTED);
        let id = made.json()["id"].as_i64().unwrap();
        let read = wait_planned(&app, &admin, id).await;
        assert_eq!(read["state"], "discover_failed");
        assert!(read["error"].as_str().unwrap().contains("no such"));
        // A bad kind is refused before anything is written.
        admin
            .post_json(
                &app.admin_api("x/imports"),
                &json!({"source_kind": "volume", "source_ref": "x"}),
            )
            .await
            .assert_validation_error("source_kind");
    }
);

fn tar_of(name: &str, content: &str) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    let mut header = tar::Header::new_gnu();
    header.set_size(content.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    builder
        .append_data(&mut header, name, content.as_bytes())
        .unwrap();
    builder.into_inner().unwrap()
}

db_test!(
    the_compose_file_is_read_for_facts_and_never_stored,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::default();
        shop(&fake);
        // The project's Compose labels, on the members' inspect and list.
        for name in ["shop-web-1", "shop-db-1"] {
            let mut inspected = fake.inspected.lock().unwrap();
            let labels = &mut inspected.get_mut(name).unwrap()["Config"]["Labels"];
            labels["com.docker.compose.project.working_dir"] = json!("/srv/shop");
            labels["com.docker.compose.project.config_files"] =
                json!("/srv/shop/docker-compose.yml");
        }
        let compose = format!(
            "services:\n  web:\n    image: web:1\n    environment:\n      DB_PASSWORD: {SECRET}\n  db:\n    image: db:1\n  worker:\n    image: worker:1\n    build: .\n"
        );
        fake.files.lock().unwrap().insert(
            "host:/srv/shop/docker-compose.yml".to_owned(),
            tar_of("docker-compose.yml", &compose),
        );
        let app = support::app_with_helper(db, fake, "helper:1").await;
        let admin = manager(&app).await;
        let made = admin
            .post_json(
                &app.admin_api("x/imports"),
                &json!({"source_kind": "project", "source_ref": "shop"}),
            )
            .await;
        let id = made.json()["id"].as_i64().unwrap();
        let read = wait_planned(&app, &admin, id).await;
        let info = &read["plan"]["compose"][0];
        assert_eq!(info["project"], "shop", "{read}");
        assert_eq!(info["files"][0]["read"], true);
        assert_eq!(info["files"][0]["sha256"].as_str().unwrap().len(), 64);
        assert_eq!(info["without_container"], json!(["worker"]));
        assert_eq!(info["built"], json!(["worker"]));
        // The text is parsed at once: not in the answer, not in the row.
        assert!(!read.to_string().contains(SECRET));
    }
);

db_test!(
    a_compose_file_that_cannot_be_read_is_said_not_fatal,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::default();
        shop(&fake);
        for name in ["shop-web-1", "shop-db-1"] {
            let mut inspected = fake.inspected.lock().unwrap();
            let labels = &mut inspected.get_mut(name).unwrap()["Config"]["Labels"];
            labels["com.docker.compose.project.working_dir"] = json!("/srv/shop");
            // Outside the working directory: never read.
            labels["com.docker.compose.project.config_files"] =
                json!("/srv/other/docker-compose.yml");
        }
        let app = support::app_with_helper(db, fake, "helper:1").await;
        let admin = manager(&app).await;
        let made = admin
            .post_json(
                &app.admin_api("x/imports"),
                &json!({"source_kind": "project", "source_ref": "shop"}),
            )
            .await;
        let read = wait_planned(&app, &admin, made.json()["id"].as_i64().unwrap()).await;
        assert_eq!(read["state"], "planned", "{read}");
        let info = &read["plan"]["compose"][0];
        assert_eq!(info["files"][0]["read"], false);
        assert!(info["notes"][0].as_str().unwrap().contains("may read"));
    }
);
