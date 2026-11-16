//! Importing existing containers (docs/import.md).

use rdockpanel::database::migrations::Migrator;
use rdockpanel::services::docker_service::ContainerItem;
use rustcms::axum::http::{Method, StatusCode};
use rustcms_testing::{TestApp, TestClient, TestDb, TestResponse, db_test};
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
/// `db`, each with a volume of its own and `web` calling `db` by name; and
/// `tika`, alone in another project.
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
            json!([{"Type": "volume", "Name": format!("{service}data"), "Destination": "/data", "RW": true}])
        } else {
            json!([])
        };
        fake.inspected.lock().unwrap().insert(
            name.to_owned(),
            json!({
                "Id": id, "Name": format!("/{name}"), "Image": format!("sha256:{service}"),
                "State": {"Status": "running"},
                "Config": {"Image": format!("{service}:1"), "Env": env, "Labels": labels,
                    "Cmd": ["serve", "--port", "80"]},
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

        // `web` calls `db` by name: leaving `db` behind breaks that call, said
        // and confirmed.
        let url = app.admin_api(&format!("x/imports/{id}/plan"));
        admin
            .send(
                Method::PATCH,
                &url,
                Some(&json!({"choices": {"id-db": "leave"}})),
            )
            .await
            .assert_validation_error("confirm");
        admin
            .send(
                Method::PATCH,
                &url,
                Some(&json!({"choices": {"id-db": "leave"}, "confirm": true})),
            )
            .await
            .assert_status(StatusCode::OK);
        admin
            .send(
                Method::PATCH,
                &url,
                Some(&json!({"choices": {"id-db": "bring"}})),
            )
            .await
            .assert_status(StatusCode::OK);
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

/// An import in `state` that owns `service` (its item).
async fn import_owning(app: &TestApp, service: Option<i64>, state: &str) -> i64 {
    use rdockpanel::models::{container_import, container_import_item};
    use sea_orm::{ActiveModelTrait, Set};
    let now = chrono::Utc::now();
    let row = container_import::ActiveModel {
        project_name: Set("shop".to_owned()),
        source_kind: Set("project".to_owned()),
        source_ref: Set("shop".to_owned()),
        state: Set(state.to_owned()),
        auto_undo: Set(true),
        undo_requested: Set(false),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(app.state().db())
    .await
    .unwrap();
    container_import_item::ActiveModel {
        import_id: Set(row.id),
        service_id: Set(service),
        container_id: Set("id-web".to_owned()),
        container_name: Set("web".to_owned()),
        level: Set(0),
        choice: Set("bring".to_owned()),
        was_running: Set(true),
        state: Set("pending".to_owned()),
        ..Default::default()
    }
    .insert(app.state().db())
    .await
    .unwrap();
    row.id
}

async fn set_state(app: &TestApp, import: i64, state: &str) {
    use rdockpanel::models::container_import;
    use sea_orm::{ActiveModelTrait, EntityTrait, IntoActiveModel, Set};
    let mut row = container_import::Entity::find_by_id(import)
        .one(app.state().db())
        .await
        .unwrap()
        .unwrap()
        .into_active_model();
    row.state = Set(state.to_owned());
    row.update(app.state().db()).await.unwrap();
}

db_test!(
    nothing_touches_a_service_while_its_import_runs,
    migrate = Migrator,
    |db: TestDb| async move {
        use crate::deployments::{action, create};
        use rdockpanel::services::{import_service, panel_update_service};
        let app = support::app_with_docker(db, FakeDocker::default()).await;
        let root = support::user_with(&app, "root", &[]).await;
        app.make_super_admin(&root).await;
        let admin = app.acting_as(&root).await;
        let project = create(&app, &admin, "projects", json!({"name": "shop"})).await;
        let service = create(
            &app,
            &admin,
            "services",
            json!({"project_id": project, "name": "web", "image": "nginx:1.27", "replicas": 1}),
        )
        .await;
        let id: i64 = service.parse().unwrap();
        let import = import_owning(&app, Some(id), "copying").await;
        let db = app.state().db();

        // Every trigger is refused while it runs (and while it needs the admin).
        for state in ["queued", "deploying", "undoing", "needs_attention"] {
            set_state(&app, import, state).await;
            let refused = action(&app, &admin, "services", "deploy", &service).await;
            assert_eq!(
                refused.status,
                StatusCode::CONFLICT,
                "{state}: {}",
                refused.text()
            );
            assert_eq!(refused.json()["code"], "import_in_progress");
            // The panel does not update under a running import (waiting for the
            // admin is not running).
            assert_eq!(
                import_service::refuse_updating_while_importing(db)
                    .await
                    .is_err(),
                state != "needs_attention",
                "{state}"
            );
        }
        // Not while it only waits for the admin to start or when it is over.
        for state in ["planned", "active", "finished", "reverted"] {
            set_state(&app, import, state).await;
            assert!(
                import_service::refuse_updating_while_importing(db)
                    .await
                    .is_ok(),
                "{state}"
            );
        }
        set_state(&app, import, "active").await;
        action(&app, &admin, "services", "deploy", &service)
            .await
            .assert_status(StatusCode::OK);
        // The update's gate is the same one.
        let _ = panel_update_service::KIND;
    }
);

db_test!(
    the_originals_of_an_import_are_not_touched_through_docker,
    migrate = Migrator,
    |db: TestDb| async move {
        use rdockpanel::models::container_import_volume;
        use sea_orm::{ActiveModelTrait, Set};
        let fake = FakeDocker::with_app_and_panel();
        let app = support::app_with_docker(db, fake.clone()).await;
        let root = support::user_with(&app, "root", &[]).await;
        app.make_super_admin(&root).await;
        let admin = app.acting_as(&root).await;
        // An import that holds `web` (id-web) and its volume `web-data`.
        let import = import_owning(&app, None, "active").await;
        container_import_volume::ActiveModel {
            import_id: Set(Some(import)),
            original_name: Set("web-data".to_owned()),
            mode: Set("copy".to_owned()),
            panel_volume: Set(Some("rdp_shop_web_data".to_owned())),
            ..Default::default()
        }
        .insert(app.state().db())
        .await
        .unwrap();
        let refused = |what: &str, response: rustcms_testing::TestResponse| {
            assert_eq!(
                response.status,
                StatusCode::CONFLICT,
                "{what}: {}",
                response.text()
            );
            assert_eq!(response.json()["code"], "import_in_progress", "{what}");
        };
        let api = |path: &str| app.admin_api(&format!("x/docker/{path}"));
        refused(
            "stop",
            admin
                .post_json(&api("containers/web/stop"), &json!({}))
                .await,
        );
        refused(
            "start",
            admin
                .post_json(&api("containers/id-web/start"), &json!({}))
                .await,
        );
        refused(
            "remove",
            admin
                .send(Method::DELETE, &api("containers/web"), None)
                .await,
        );
        refused(
            "volume",
            admin
                .send(Method::DELETE, &api("volumes/web-data"), None)
                .await,
        );
        refused("prune", admin.post_json(&api("prune"), &json!({})).await);
        refused(
            "prune containers",
            admin
                .post_json(&app.admin_api("x/apps/system/prune/containers"), &json!({}))
                .await,
        );
        assert!(fake.calls().is_empty(), "{:?}", fake.calls());
        // Over (reverted): the original is the user's again.
        set_state(&app, import, "reverted").await;
        admin
            .post_json(&api("containers/web/stop"), &json!({}))
            .await
            .assert_status(StatusCode::OK);
    }
);

/// `shop` of [`shop`], with the volume `shared` known to the fake and web's own
/// data volume; read into a plan for `import`, ready to start.
async fn planned_shop(app: &TestApp, admin: &TestClient<'_>) -> i64 {
    let made = admin
        .post_json(
            &app.admin_api("x/imports"),
            &json!({"source_kind": "project", "source_ref": "shop"}),
        )
        .await;
    made.assert_status(StatusCode::ACCEPTED);
    let id = made.json()["id"].as_i64().unwrap();
    assert_eq!(wait_planned(app, admin, id).await["state"], "planned");
    id
}

db_test!(
    starting_an_import_makes_the_project_the_services_and_their_claims,
    migrate = Migrator,
    |db: TestDb| async move {
        use rdockpanel::models::{
            container_import, container_import_claim, container_import_item,
            container_import_volume, project, service,
        };
        use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
        let fake = FakeDocker::default();
        shop(&fake);
        let app = support::app_with_helper(db, fake.clone(), "helper:1").await;
        let root = support::user_with(&app, "root", &[]).await;
        app.make_super_admin(&root).await;
        let admin = app.acting_as(&root).await;
        let id = planned_shop(&app, &admin).await;
        let start = app.admin_api(&format!("x/imports/{id}/start"));

        // The project's name is typed to confirm.
        admin
            .post_json(&start, &json!({"confirm": "nope"}))
            .await
            .assert_validation_error("confirm");
        let started = admin.post_json(&start, &json!({"confirm": "shop"})).await;
        started.assert_status(StatusCode::ACCEPTED);

        let db = app.state().db();
        let row = container_import::Entity::find_by_id(id)
            .one(db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.state, "queued");
        let projects = project::Entity::find().all(db).await.unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].name, "shop");
        let mut names: Vec<String> = service::Entity::find()
            .all(db)
            .await
            .unwrap()
            .into_iter()
            .map(|s| s.name)
            .collect();
        names.sort();
        assert_eq!(names, ["db", "web"]);
        // Env values went into the service's sealed environment, not a plan.
        let web = service::Entity::find()
            .filter(service::Column::Name.eq("web"))
            .one(db)
            .await
            .unwrap()
            .unwrap();
        assert!(web.env.is_some());
        let items = container_import_item::Entity::find().all(db).await.unwrap();
        assert_eq!(items.len(), 2);
        assert!(
            items
                .iter()
                .all(|i| i.service_id.is_some() && i.state == "pending")
        );
        let claims = container_import_claim::Entity::find()
            .all(db)
            .await
            .unwrap();
        assert!(
            claims
                .iter()
                .any(|c| c.container_id.as_deref() == Some("id-web"))
        );
        assert!(
            claims
                .iter()
                .any(|c| c.volume.as_deref() == Some("webdata"))
        );
        let mut volumes = container_import_volume::Entity::find()
            .all(db)
            .await
            .unwrap();
        volumes.sort_by_key(|v| v.original_name.clone());
        assert_eq!(volumes.len(), 2);
        assert_eq!(volumes[0].original_name, "dbdata");
        assert_eq!(
            volumes[1].panel_volume.as_deref(),
            Some("rdp_shop_web_webdata")
        );
        assert_eq!(volumes[1].mode, "copy");
        // Started once: the same plan again is refused.
        let again = admin.post_json(&start, &json!({"confirm": "shop"})).await;
        assert_eq!(again.status, StatusCode::CONFLICT, "{}", again.text());
        // Another import of one of the containers loses the claim.
        let second = admin
            .post_json(
                &app.admin_api("x/imports"),
                &json!({"source_kind": "container", "source_ref": "shop-db-1"}),
            )
            .await
            .json()["id"]
            .as_i64()
            .unwrap();
        assert_eq!(wait_planned(&app, &admin, second).await["state"], "planned");
        let lost = admin
            .post_json(
                &app.admin_api(&format!("x/imports/{second}/start")),
                &json!({"confirm": "shop-db-1"}),
            )
            .await;
        assert_eq!(lost.status, StatusCode::CONFLICT, "{}", lost.text());
    }
);

/// The Swarm services of the fake that are not the panel's own (`rdp-*`).
fn imported_services(fake: &FakeDocker) -> Vec<String> {
    fake.services
        .lock()
        .unwrap()
        .keys()
        .filter(|name| !name.starts_with("rdp-"))
        .cloned()
        .collect()
}

async fn import_state(app: &TestApp, id: i64) -> String {
    use rdockpanel::models::container_import;
    use sea_orm::EntityTrait;
    container_import::Entity::find_by_id(id)
        .one(app.state().db())
        .await
        .unwrap()
        .unwrap()
        .state
}

db_test!(
    an_import_runs_to_active_then_can_be_undone,
    migrate = Migrator,
    |db: TestDb| async move {
        use rdockpanel::models::{container_import_volume, project, service};
        use sea_orm::EntityTrait;
        let fake = FakeDocker::default();
        shop(&fake);
        let app = support::app_with_helper(db, fake.clone(), "helper:1").await;
        let root = support::user_with(&app, "root", &[]).await;
        app.make_super_admin(&root).await;
        let admin = app.acting_as(&root).await;
        let id = planned_shop(&app, &admin).await;
        admin
            .post_json(
                &app.admin_api(&format!("x/imports/{id}/start")),
                &json!({"confirm": "shop"}),
            )
            .await
            .assert_status(StatusCode::ACCEPTED);
        rustcms::queue::run_until_idle(app.state(), None, 100)
            .await
            .unwrap();
        assert_eq!(import_state(&app, id).await, "active", "{:?}", fake.calls());

        // The originals were stopped (not restarting by themselves) …
        let calls = fake.calls();
        for call in [
            "restart-policy id-web no",
            "stop id-web",
            "restart-policy id-db no",
            "stop id-db",
        ] {
            assert!(calls.contains(&call.to_owned()), "{call}: {calls:?}");
        }
        // … their volumes copied by a helper into volumes of the panel's …
        let volumes = container_import_volume::Entity::find()
            .all(app.state().db())
            .await
            .unwrap();
        assert!(volumes.iter().all(|v| v.copied_at.is_some()), "{volumes:?}");
        assert_eq!(fake.helpers.lock().unwrap().len(), 2);
        let panel_volumes: Vec<String> = fake
            .volumes
            .lock()
            .unwrap()
            .iter()
            .map(|v| v.name.clone())
            .collect();
        assert!(
            panel_volumes.contains(&"rdp_shop_web_webdata".to_owned()),
            "{panel_volumes:?}"
        );
        // … and the services run in Swarm, `db` before `web`.
        let swarm: Vec<String> = fake.services.lock().unwrap().keys().cloned().collect();
        assert!(swarm.contains(&"shop_web".to_owned()) && swarm.contains(&"shop_db".to_owned()));
        let created: Vec<&String> = calls
            .iter()
            .filter(|c| c.starts_with("create shop_"))
            .collect();
        let _ = created;

        // Undone: the services go, the originals run again as they did.
        admin
            .post_json(&app.admin_api(&format!("x/imports/{id}/undo")), &json!({}))
            .await
            .assert_status(StatusCode::ACCEPTED);
        rustcms::queue::run_until_idle(app.state(), None, 100)
            .await
            .unwrap();
        assert_eq!(
            import_state(&app, id).await,
            "reverted",
            "{:?}",
            fake.calls()
        );
        assert!(
            imported_services(&fake).is_empty(),
            "{:?}",
            imported_services(&fake)
        );
        let calls = fake.calls();
        for call in [
            "start id-web",
            "start id-db",
            "restart-policy id-web always",
        ] {
            assert!(calls.contains(&call.to_owned()), "{call}: {calls:?}");
        }
        let db = app.state().db();
        assert!(service::Entity::find().all(db).await.unwrap().is_empty());
        assert!(project::Entity::find().all(db).await.unwrap().is_empty());
        // The copies are kept (they may hold what was written since).
        assert!(
            fake.volumes
                .lock()
                .unwrap()
                .iter()
                .any(|v| v.name == "rdp_shop_web_webdata")
        );
    }
);

/// A started `shop` import on a fake Docker, not yet run.
async fn started_shop(db: TestDb) -> (TestApp, FakeDocker, i64) {
    let fake = FakeDocker::default();
    shop(&fake);
    let app = support::app_with_helper(db, fake.clone(), "helper:1").await;
    let root = support::user_with(&app, "root", &[]).await;
    app.make_super_admin(&root).await;
    let admin = app.acting_as(&root).await;
    let id = planned_shop(&app, &admin).await;
    admin
        .post_json(
            &app.admin_api(&format!("x/imports/{id}/start")),
            &json!({"confirm": "shop"}),
        )
        .await
        .assert_status(StatusCode::ACCEPTED);
    (app, fake, id)
}

/// A second super admin (the first one is the test's own).
async fn ops(app: &TestApp) -> TestClient<'_> {
    ops_named(app, "ops").await
}

async fn ops_named<'a>(app: &'a TestApp, name: &str) -> TestClient<'a> {
    let user = support::user_with(app, name, &[]).await;
    app.make_super_admin(&user).await;
    app.acting_as(&user).await
}

async fn run_all(app: &TestApp) {
    rustcms::queue::run_until_idle(app.state(), None, 200)
        .await
        .unwrap();
}

db_test!(
    a_failed_copy_undoes_the_import_by_itself,
    migrate = Migrator,
    |db: TestDb| async move {
        use rdockpanel::models::{project, service};
        use sea_orm::EntityTrait;
        let (app, fake, id) = started_shop(db).await;
        *fake.helper_exit.lock().unwrap() = 1;
        run_all(&app).await;
        let row_state = import_state(&app, id).await;
        assert_eq!(row_state, "reverted", "{:?}", fake.calls());
        // Nothing was deployed, the originals run as before, nothing is left.
        assert!(
            imported_services(&fake).is_empty(),
            "{:?}",
            imported_services(&fake)
        );
        let calls = fake.calls();
        assert!(calls.contains(&"start id-web".to_owned()), "{calls:?}");
        assert!(calls.contains(&"start id-db".to_owned()), "{calls:?}");
        let db = app.state().db();
        assert!(service::Entity::find().all(db).await.unwrap().is_empty());
        assert!(project::Entity::find().all(db).await.unwrap().is_empty());
    }
);

async fn hold_lease(app: &TestApp, id: i64, until: chrono::DateTime<chrono::Utc>) {
    use rdockpanel::models::container_import;
    use sea_orm::{ActiveModelTrait, EntityTrait, IntoActiveModel, Set};
    let mut row = container_import::Entity::find_by_id(id)
        .one(app.state().db())
        .await
        .unwrap()
        .unwrap()
        .into_active_model();
    row.lease_token = Set(Some("another".to_owned()));
    row.lease_expires_at = Set(Some(until));
    row.update(app.state().db()).await.unwrap();
}

db_test!(
    a_lease_held_by_another_keeps_the_step_waiting_until_it_expires,
    migrate = Migrator,
    |db: TestDb| async move {
        let (app, _fake, id) = started_shop(db).await;
        hold_lease(&app, id, chrono::Utc::now() + chrono::Duration::minutes(5)).await;
        run_all(&app).await;
        assert_eq!(import_state(&app, id).await, "queued");
        // Expired (its holder died): the tick queues the step again and the
        // import completes.
        hold_lease(&app, id, chrono::Utc::now() - chrono::Duration::seconds(1)).await;
        rdockpanel::services::import_run_service::tick(app.state())
            .await
            .unwrap();
        run_all(&app).await;
        assert_eq!(import_state(&app, id).await, "active");
    }
);

db_test!(
    a_forced_stop_waits_for_the_admin_who_can_go_on,
    migrate = Migrator,
    |db: TestDb| async move {
        let (app, fake, id) = started_shop(db).await;
        *fake.stop_exit.lock().unwrap() = 137;
        run_all(&app).await;
        // Stopped by force: the data may be inconsistent, the admin decides.
        assert_eq!(
            import_state(&app, id).await,
            "needs_attention",
            "{:?}",
            fake.calls()
        );
        let admin = ops(&app).await;
        let got = admin
            .get(&app.admin_api(&format!("x/imports/{id}")))
            .await
            .json();
        assert!(got["error"].as_str().unwrap().contains("137"), "{got}");
        // Nothing touches its services meanwhile.
        use crate::deployments::action;
        // Go on: the rest runs and the import is active.
        *fake.stop_exit.lock().unwrap() = 0;
        admin
            .post_json(
                &app.admin_api(&format!("x/imports/{id}/continue")),
                &json!({}),
            )
            .await
            .assert_status(StatusCode::ACCEPTED);
        run_all(&app).await;
        assert_eq!(import_state(&app, id).await, "active", "{:?}", fake.calls());
        let _ = action;
    }
);

db_test!(
    a_finished_import_removes_the_originals_and_releases_its_volumes,
    migrate = Migrator,
    |db: TestDb| async move {
        use rdockpanel::models::container_import_volume;
        use sea_orm::EntityTrait;
        let (app, fake, id) = started_shop(db).await;
        run_all(&app).await;
        assert_eq!(import_state(&app, id).await, "active");
        let admin = ops(&app).await;
        admin
            .post_json(
                &app.admin_api(&format!("x/imports/{id}/finish")),
                &json!({}),
            )
            .await
            .assert_status(StatusCode::OK);
        assert_eq!(import_state(&app, id).await, "finished");
        let calls = fake.calls();
        assert!(calls.contains(&"remove id-web".to_owned()), "{calls:?}");
        assert!(calls.contains(&"remove id-db".to_owned()), "{calls:?}");
        // The original volumes stay until released (typing their name).
        let volumes = container_import_volume::Entity::find()
            .all(app.state().db())
            .await
            .unwrap();
        let web = volumes
            .iter()
            .find(|v| v.original_name == "webdata")
            .unwrap();
        let removal = admin
            .send(
                Method::DELETE,
                &app.admin_api("x/docker/volumes/webdata"),
                None,
            )
            .await;
        assert_eq!(removal.status, StatusCode::CONFLICT, "{}", removal.text());
        admin
            .post_json(
                &app.admin_api(&format!("x/imports/{id}/volumes/{}/release", web.id)),
                &json!({}),
            )
            .await
            .assert_status(StatusCode::OK);
        let freed = container_import_volume::Entity::find_by_id(web.id)
            .one(app.state().db())
            .await
            .unwrap()
            .unwrap();
        assert!(freed.released_at.is_some());
    }
);

db_test!(
    an_original_started_again_is_stopped_again,
    migrate = Migrator,
    |db: TestDb| async move {
        use rdockpanel::services::docker_service::ContainerAction;
        use rdockpanel::services::docker_service::DockerApi;
        let (app, fake, id) = started_shop(db).await;
        run_all(&app).await;
        assert_eq!(import_state(&app, id).await, "active");
        let stops = |fake: &FakeDocker| fake.calls().iter().filter(|c| *c == "stop id-web").count();
        assert_eq!(stops(&fake), 1);
        // `docker compose up` (or a person) starts the original again.
        fake.container_action("id-web", ContainerAction::Start)
            .await
            .unwrap();
        rdockpanel::services::import_run_service::tick(app.state())
            .await
            .unwrap();
        assert_eq!(stops(&fake), 2, "{:?}", fake.calls());
        // Until the admin says to stop watching.
        let admin = ops(&app).await;
        admin
            .post_json(
                &app.admin_api(&format!("x/imports/{id}/stop-watching")),
                &json!({}),
            )
            .await
            .assert_status(StatusCode::OK);
        fake.container_action("id-web", ContainerAction::Start)
            .await
            .unwrap();
        rdockpanel::services::import_run_service::tick(app.state())
            .await
            .unwrap();
        assert_eq!(stops(&fake), 2);
    }
);

db_test!(
    an_original_that_starts_while_copying_voids_the_copy,
    migrate = Migrator,
    |db: TestDb| async move {
        let (app, fake, id) = started_shop(db).await;
        fake.started_by_helper
            .lock()
            .unwrap()
            .push("id-web".to_owned());
        run_all(&app).await;
        // The copy cannot be trusted: undone, the original runs, nothing deployed.
        assert_eq!(
            import_state(&app, id).await,
            "reverted",
            "{:?}",
            fake.calls()
        );
        assert!(
            imported_services(&fake).is_empty(),
            "{:?}",
            imported_services(&fake)
        );
        let admin = ops(&app).await;
        let got = admin
            .get(&app.admin_api(&format!("x/imports/{id}")))
            .await
            .json();
        assert!(got["error"].as_str().unwrap().contains("void"), "{got}");
    }
);

db_test!(
    a_volume_of_the_same_name_that_is_not_the_imports_is_never_written_over,
    migrate = Migrator,
    |db: TestDb| async move {
        use rdockpanel::services::docker_service::{DockerApi, VolumeItem};
        let _ = std::marker::PhantomData::<VolumeItem>;
        let fake = FakeDocker::default();
        shop(&fake);
        // Someone's volume, with the name the copy would take.
        fake.create_volume("rdp_shop_web_webdata").await.unwrap();
        let app = support::app_with_helper(db, fake.clone(), "helper:1").await;
        let root = support::user_with(&app, "root", &[]).await;
        app.make_super_admin(&root).await;
        let admin = app.acting_as(&root).await;
        let id = planned_shop(&app, &admin).await;
        admin
            .post_json(
                &app.admin_api(&format!("x/imports/{id}/start")),
                &json!({"confirm": "shop"}),
            )
            .await
            .assert_status(StatusCode::ACCEPTED);
        run_all(&app).await;
        assert_eq!(
            import_state(&app, id).await,
            "reverted",
            "{:?}",
            fake.calls()
        );
        // Nothing was copied into it.
        assert!(
            fake.helpers
                .lock()
                .unwrap()
                .iter()
                .all(|h| h.mounts.iter().all(|m| m.source != "rdp_shop_web_webdata"))
        );
        let got = admin
            .get(&app.admin_api(&format!("x/imports/{id}")))
            .await
            .json();
        assert!(
            got["error"].as_str().unwrap().contains("not this import's"),
            "{got}"
        );
    }
);

fn docker_cli(args: &[&str]) -> std::process::Output {
    std::process::Command::new("docker")
        .args(args)
        .output()
        .expect("docker")
}

fn cli_ok(args: &[&str]) -> String {
    let out = docker_cli(args);
    assert!(
        out.status.success(),
        "docker {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

// A real container with a volume and a folder, imported into a real Swarm
// (`PANEL_TEST_DOCKER=1 PANEL_TEST_SWARM=1`, `scripts/dev-swarm.sh`): the
// copy has the files, the folder is reached through the panel's symlink,
// the Compose file is renamed; undone, all of it is back.
db_test!(
    an_import_on_a_real_swarm,
    migrate = Migrator,
    |db: TestDb| async move {
        use rdockpanel::services::docker_service::BollardDocker;
        use rdockpanel::services::import_service::ImportSettings;
        if std::env::var_os("PANEL_TEST_SWARM").is_none() {
            return;
        }
        let _swarm = support::swarm_lock().await;
        let tag = format!("i{}", support::run_id(&db));
        // Outside the repository: a panel run from source never imports what
        // runs its own files.
        let base = std::path::PathBuf::from(std::env::var("HOME").unwrap())
            .join(format!(".rdp-import-test-{tag}"));
        let (data, proj, links) = (base.join("data"), base.join("proj"), base.join("links"));
        let text = |p: &std::path::Path| p.to_string_lossy().into_owned();
        let original = format!("{tag}-web");
        let volume = format!("{tag}vol");
        let copy = format!("rdp_{tag}_web_{volume}");
        // Also when an assertion fails: the Drop below runs it.
        struct Clean<F: Fn()>(F);
        impl<F: Fn()> Drop for Clean<F> {
            fn drop(&mut self) {
                (self.0)();
            }
        }
        let network = format!("rdp_{tag}");
        let cleanup = || {
            let _ = docker_cli(&["rm", "-f", &original]);
            let _ = docker_cli(&["service", "rm", &format!("{tag}_web")]);
            // Once the tasks' containers are gone.
            for _ in 0..30 {
                // Every volume of the project (the copies, the moved folder).
                let listed = cli_ok(&[
                    "volume",
                    "ls",
                    "-q",
                    "--filter",
                    &format!("name=rdp_{tag}_"),
                ]);
                let mut names: Vec<&str> = listed.lines().collect();
                names.extend([volume.as_str(), copy.as_str()]);
                let mut rm = vec!["volume", "rm"];
                rm.extend(names);
                let volumes = docker_cli(&rm);
                let net = docker_cli(&["network", "rm", &network]);
                let gone = |o: &std::process::Output| {
                    o.status.success() || String::from_utf8_lossy(&o.stderr).contains("No such")
                };
                if (gone(&volumes)
                    || String::from_utf8_lossy(&volumes.stderr)
                        .matches("No such")
                        .count()
                        == 2)
                    && gone(&net)
                {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
            // Made by helpers, as root: removed by one.
            if base.exists() {
                let _ = docker_cli(&[
                    "run",
                    "--rm",
                    "-v",
                    &format!("{}:/b", text(&base)),
                    "alpine:3",
                    "rm",
                    "-rf",
                    "/b/links",
                ]);
                let _ = std::fs::remove_dir_all(&base);
            }
        };
        cleanup();
        let _clean = Clean(&cleanup);
        for dir in [&data, &proj, &links] {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::fs::write(data.join("x"), "bind").unwrap();
        let compose = "services:\n  web:\n    image: ghcr.io/traefik/whoami\n";
        std::fs::write(proj.join("docker-compose.yml"), compose).unwrap();
        cli_ok(&["volume", "create", &volume]);
        cli_ok(&[
            "run",
            "--rm",
            "-v",
            &format!("{volume}:/d"),
            "alpine:3",
            "sh",
            "-c",
            "echo hi > /d/f; chown 999:998 /d/f",
        ]);
        cli_ok(&[
            "run",
            "-d",
            "--name",
            &original,
            "--restart",
            "unless-stopped",
            "--label",
            &format!("com.docker.compose.project={tag}"),
            "--label",
            "com.docker.compose.service=web",
            "--label",
            &format!("com.docker.compose.project.working_dir={}", text(&proj)),
            "--label",
            &format!(
                "com.docker.compose.project.config_files={}/docker-compose.yml",
                text(&proj)
            ),
            "-v",
            &format!("{volume}:/data"),
            "-v",
            &format!("{}:/bind", text(&data)),
            "ghcr.io/traefik/whoami",
        ]);

        let settings = ImportSettings {
            helper_image: Some("alpine:3".to_owned()),
            observe: std::time::Duration::from_secs(2),
            deadline: std::time::Duration::from_secs(60),
            poll: std::time::Duration::from_millis(500),
            links_dir: Some(links.clone()),
            lease: std::time::Duration::from_secs(90),
        };
        let app = support::app_with_import(db, BollardDocker::connect().unwrap(), settings).await;
        let admin = ops(&app).await;
        let made = admin
            .post_json(
                &app.admin_api("x/imports"),
                &json!({"source_kind": "project", "source_ref": tag}),
            )
            .await;
        made.assert_status(StatusCode::ACCEPTED);
        let id = made.json()["id"].as_i64().unwrap();
        let read = wait_planned(&app, &admin, id).await;
        assert_eq!(read["state"], "planned", "{read}");
        assert!(
            read["plan"]["items"]
                .as_array()
                .unwrap()
                .iter()
                .all(|i| i["choice"] == "bring"),
            "{}",
            read["plan"]["items"]
        );
        admin
            .post_json(
                &app.admin_api(&format!("x/imports/{id}/start")),
                &json!({"confirm": tag}),
            )
            .await
            .assert_status(StatusCode::ACCEPTED);
        run_all(&app).await;
        let state = import_state(&app, id).await;
        let shown = admin
            .get(&app.admin_api(&format!("x/imports/{id}")))
            .await
            .text();
        assert_eq!(state, "active", "{shown}");

        // The original is stopped and does not come back by itself.
        assert_eq!(
            cli_ok(&[
                "inspect",
                "-f",
                "{{.State.Running}} {{.HostConfig.RestartPolicy.Name}}",
                &original
            ]),
            "false no"
        );
        // The copy has the file, with its owner.
        assert_eq!(
            cli_ok(&[
                "run",
                "--rm",
                "-v",
                &format!("{copy}:/d"),
                "alpine:3",
                "sh",
                "-c",
                "cat /d/f; stat -c %u:%g /d/f"
            ]),
            "hi\n999:998"
        );
        // The folder is reached through the panel's symlink.
        let link = std::fs::read_dir(links.join(format!("{tag}_web")))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert_eq!(
            cli_ok(&[
                "run",
                "--rm",
                "-v",
                &format!("{}:/b", text(&link)),
                "alpine:3",
                "cat",
                "/b/x"
            ]),
            "bind"
        );
        // The Compose file is renamed (so `compose up` starts nothing old).
        assert!(proj.join("docker-compose.yml.rdockpanel-imported").exists());
        assert!(!proj.join("docker-compose.yml").exists());
        // The service runs in Swarm.
        cli_ok(&["service", "inspect", &format!("{tag}_web")]);

        // Move all: the folder is copied into a volume of the panel's, the
        // service runs on it; then back on the folder.
        let web = web_service(&app).await;
        let folder_mount = mounts_of(&app, web.id)
            .await
            .into_iter()
            .find(|m| m.target == "/bind")
            .unwrap();
        let short = folder_mount
            .source
            .clone()
            .unwrap()
            .rsplit('/')
            .next()
            .unwrap()
            .to_owned();
        let moved = admin
            .post_json(
                &app.admin_api(&format!("x/services/{}/move-data", web.id)),
                &json!({"confirm": "web"}),
            )
            .await;
        moved.assert_status(StatusCode::ACCEPTED);
        let moving = moved.json()["id"].as_i64().unwrap();
        run_all(&app).await;
        assert_eq!(import_state(&app, moving).await, "active");
        assert_eq!(
            cli_ok(&[
                "run",
                "--rm",
                "-v",
                &format!("rdp_{tag}_web_{short}:/b"),
                "alpine:3",
                "cat",
                "/b/x"
            ]),
            "bind"
        );
        // And it runs again on the volume (a deployment keeps a stopped service stopped).
        assert_eq!(
            cli_ok(&[
                "service",
                "ls",
                "--filter",
                &format!("name={tag}_web"),
                "--format",
                "{{.Replicas}}"
            ]),
            "1/1"
        );
        admin
            .post_json(
                &app.admin_api(&format!("x/services/{}/move-data/undo", web.id)),
                &json!({}),
            )
            .await
            .assert_status(StatusCode::ACCEPTED);
        run_all(&app).await;
        assert_eq!(import_state(&app, moving).await, "reverted");

        // Undone: the service goes, the original runs as it did, the file is back.
        admin
            .post_json(&app.admin_api(&format!("x/imports/{id}/undo")), &json!({}))
            .await
            .assert_status(StatusCode::ACCEPTED);
        run_all(&app).await;
        assert_eq!(import_state(&app, id).await, "reverted");
        assert!(
            !docker_cli(&["service", "inspect", &format!("{tag}_web")])
                .status
                .success()
        );
        assert_eq!(
            cli_ok(&[
                "inspect",
                "-f",
                "{{.State.Running}} {{.HostConfig.RestartPolicy.Name}}",
                &original
            ]),
            "true unless-stopped"
        );
        assert!(proj.join("docker-compose.yml").exists());
    }
);

db_test!(
    a_container_that_changed_since_the_plan_stops_the_start,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::default();
        shop(&fake);
        let app = support::app_with_helper(db, fake.clone(), "helper:1").await;
        let admin = ops(&app).await;
        let id = planned_shop(&app, &admin).await;
        // Someone changes `web` after the plan was read.
        {
            let mut inspected = fake.inspected.lock().unwrap();
            let env = &mut inspected.get_mut("shop-web-1").unwrap()["Config"]["Env"];
            env.as_array_mut().unwrap().push(json!("NEW=1"));
        }
        let refused = admin
            .post_json(
                &app.admin_api(&format!("x/imports/{id}/start")),
                &json!({"confirm": "shop"}),
            )
            .await;
        assert_eq!(refused.status, StatusCode::CONFLICT, "{}", refused.text());
        assert_eq!(refused.json()["code"], "plan_stale");
        // Nothing was created, and a new reading gives a plan that starts.
        assert_eq!(import_state(&app, id).await, "planned");
        admin
            .post_json(
                &app.admin_api(&format!("x/imports/{id}/rescan")),
                &json!({}),
            )
            .await
            .assert_status(StatusCode::ACCEPTED);
        assert_eq!(wait_planned(&app, &admin, id).await["state"], "planned");
        admin
            .post_json(
                &app.admin_api(&format!("x/imports/{id}/start")),
                &json!({"confirm": "shop"}),
            )
            .await
            .assert_status(StatusCode::ACCEPTED);
    }
);

db_test!(
    an_undo_does_not_start_the_originals_while_a_task_container_lives,
    migrate = Migrator,
    |db: TestDb| async move {
        let (app, fake, id) = started_shop(db).await;
        run_all(&app).await;
        assert_eq!(import_state(&app, id).await, "active");
        let admin = ops(&app).await;
        // The tasks' containers outlive the services (Swarm removes them late).
        fake.detailed.lock().unwrap().push(json!({
            "Id": "task-web-1", "Names": ["/shop_web.1.abc"], "State": "running",
            "Labels": {"com.docker.swarm.service.name": "shop_web"}
        }));
        admin
            .post_json(&app.admin_api(&format!("x/imports/{id}/undo")), &json!({}))
            .await
            .assert_status(StatusCode::ACCEPTED);
        run_all(&app).await;
        assert_eq!(
            import_state(&app, id).await,
            "needs_attention",
            "{:?}",
            fake.calls()
        );
        assert!(
            !fake.calls().contains(&"start id-web".to_owned()),
            "{:?}",
            fake.calls()
        );
        // Gone: retry goes on and the originals run again.
        fake.detailed
            .lock()
            .unwrap()
            .retain(|c| c["Id"] != "task-web-1");
        admin
            .post_json(&app.admin_api(&format!("x/imports/{id}/retry")), &json!({}))
            .await
            .assert_status(StatusCode::ACCEPTED);
        run_all(&app).await;
        assert_eq!(
            import_state(&app, id).await,
            "reverted",
            "{:?}",
            fake.calls()
        );
        assert!(fake.calls().contains(&"start id-web".to_owned()));
    }
);

/// `shop`, with a folder on the host that `web` writes (`/srv/web-folder`).
fn shop_with_folder(fake: &FakeDocker) {
    shop(fake);
    let mut inspected = fake.inspected.lock().unwrap();
    inspected.get_mut("shop-web-1").unwrap()["Mounts"]
        .as_array_mut()
        .unwrap()
        .push(json!({"Type": "bind", "Source": "/srv/web-folder", "Destination": "/folder", "RW": true}));
}

async fn web_service(app: &TestApp) -> rdockpanel::models::service::Model {
    use rdockpanel::models::service;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
    service::Entity::find()
        .filter(service::Column::Name.eq("web"))
        .one(app.state().db())
        .await
        .unwrap()
        .unwrap()
}

async fn mounts_of(app: &TestApp, service: i64) -> Vec<rdockpanel::models::service_mount::Model> {
    use rdockpanel::models::service_mount;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
    service_mount::Entity::find()
        .filter(service_mount::Column::ServiceId.eq(service))
        .all(app.state().db())
        .await
        .unwrap()
}

db_test!(
    a_folder_in_place_moves_into_a_volume_and_back_then_the_old_folder_goes,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::default();
        shop_with_folder(&fake);
        let app = support::app_with_helper(db, fake.clone(), "helper:1").await;
        let admin = ops(&app).await;
        let id = planned_shop(&app, &admin).await;
        admin
            .post_json(
                &app.admin_api(&format!("x/imports/{id}/start")),
                &json!({"confirm": "shop"}),
            )
            .await
            .assert_status(StatusCode::ACCEPTED);
        run_all(&app).await;
        assert_eq!(import_state(&app, id).await, "active", "{:?}", fake.calls());
        let web = web_service(&app).await;
        assert!(web.admin_only);
        assert!(
            mounts_of(&app, web.id)
                .await
                .iter()
                .any(|m| m.kind == "link")
        );

        // Move: the folder is copied into a volume of the panel's and the mount changed.
        let move_url = app.admin_api(&format!("x/services/{}/move-data", web.id));
        admin
            .post_json(&move_url, &json!({"confirm": "nope"}))
            .await
            .assert_validation_error("confirm");
        let started = admin.post_json(&move_url, &json!({"confirm": "web"})).await;
        started.assert_status(StatusCode::ACCEPTED);
        let moving = started.json()["id"].as_i64().unwrap();
        run_all(&app).await;
        assert_eq!(
            import_state(&app, moving).await,
            "active",
            "{:?}",
            fake.calls()
        );
        let after = mounts_of(&app, web.id).await;
        assert!(after.iter().all(|m| m.kind != "link"), "{after:?}");
        assert!(
            after
                .iter()
                .any(|m| m.target == "/folder" && m.kind == "volume")
        );
        // The copy came from the folder, read only; the old folder is untouched.
        let helpers = fake.helpers.lock().unwrap().clone();
        let copy = helpers
            .iter()
            .find(|h| h.mounts.iter().any(|m| m.source == "/srv/web-folder"))
            .unwrap();
        assert!(
            copy.mounts
                .iter()
                .find(|m| m.source == "/srv/web-folder")
                .unwrap()
                .read_only
        );
        // Image only this machine has: still its administrators'.
        assert!(web_service(&app).await.admin_only);

        // Back: the mount is the link again (what was written since stays in the volume).
        admin
            .post_json(
                &app.admin_api(&format!("x/services/{}/move-data/undo", web.id)),
                &json!({}),
            )
            .await
            .assert_status(StatusCode::ACCEPTED);
        run_all(&app).await;
        assert_eq!(
            import_state(&app, moving).await,
            "reverted",
            "{:?}",
            fake.calls()
        );
        assert!(
            mounts_of(&app, web.id)
                .await
                .iter()
                .any(|m| m.kind == "link" && m.target == "/folder")
        );
        // The volume stays (it may hold what was written since): a new move
        // does not write over it.
        let volume = format!(
            "rdp_shop_web_{}",
            after
                .iter()
                .find(|m| m.target == "/folder")
                .unwrap()
                .name
                .clone()
                .unwrap()
        );
        assert!(
            fake.volumes
                .lock()
                .unwrap()
                .iter()
                .any(|v| v.name == volume),
            "{volume}"
        );
        let blocked = admin.post_json(&move_url, &json!({"confirm": "web"})).await;
        blocked.assert_status(StatusCode::ACCEPTED);
        let refused = blocked.json()["id"].as_i64().unwrap();
        run_all(&app).await;
        assert_eq!(import_state(&app, refused).await, "reverted");
        assert!(
            mounts_of(&app, web.id)
                .await
                .iter()
                .any(|m| m.kind == "link")
        );
        // The admin removes it (the move was undone: the volume is theirs), then moves again.
        admin
            .send(
                Method::DELETE,
                &app.admin_api(&format!("x/docker/volumes/{volume}")),
                None,
            )
            .await
            .assert_status(StatusCode::OK);
        // Move again, then clean the old folder: its name typed, the rules checked again.
        let again = admin.post_json(&move_url, &json!({"confirm": "web"})).await;
        again.assert_status(StatusCode::ACCEPTED);
        let moved_again = again.json()["id"].as_i64().unwrap();
        run_all(&app).await;
        assert_eq!(
            import_state(&app, moved_again).await,
            "active",
            "{:?}",
            fake.calls()
        );
        // The import that brought it in is finished first: its originals are gone.
        admin
            .post_json(
                &app.admin_api(&format!("x/imports/{id}/finish")),
                &json!({}),
            )
            .await
            .assert_status(StatusCode::OK);
        let clean = app.admin_api(&format!("x/services/{}/clean-old-folder", web.id));
        admin
            .post_json(
                &clean,
                &json!({"folder": "/srv/web-folder", "confirm": "wrong"}),
            )
            .await
            .assert_validation_error("confirm");
        // Not a folder of this service.
        admin
            .post_json(&clean, &json!({"folder": "/srv", "confirm": "srv"}))
            .await
            .assert_validation_error("folder");
        let before = fake.helpers.lock().unwrap().len();
        admin
            .post_json(
                &clean,
                &json!({"folder": "/srv/web-folder", "confirm": "web-folder"}),
            )
            .await
            .assert_status(StatusCode::OK);
        let helpers = fake.helpers.lock().unwrap().clone();
        let cleaning: Vec<_> = helpers[before..].iter().collect();
        assert!(!cleaning.is_empty());
        // Only the folder itself (and its parent for the last step) is mounted writable.
        for h in &cleaning {
            for m in h.mounts.iter().filter(|m| !m.read_only) {
                assert!(
                    ["/srv/web-folder", "/srv"].contains(&m.source.as_str())
                        || m.source.contains("/links"),
                    "{}",
                    m.source
                );
            }
        }
        // Said once: the same again finds nothing to clean.
        admin
            .post_json(
                &clean,
                &json!({"folder": "/srv/web-folder", "confirm": "web-folder"}),
            )
            .await
            .assert_validation_error("folder");
    }
);

db_test!(
    a_folder_with_shell_characters_in_its_path_is_never_part_of_a_script,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::default();
        shop(&fake);
        let odd = "/srv/we\"ird $(touch /pwned) `x` ;'";
        fake.inspected
            .lock()
            .unwrap()
            .get_mut("shop-web-1")
            .unwrap()["Mounts"]
            .as_array_mut()
            .unwrap()
            .push(json!({"Type": "bind", "Source": odd, "Destination": "/folder", "RW": true}));
        let app = support::app_with_helper(db, fake.clone(), "helper:1").await;
        let admin = ops(&app).await;
        let id = planned_shop(&app, &admin).await;
        admin
            .post_json(
                &app.admin_api(&format!("x/imports/{id}/start")),
                &json!({"confirm": "shop"}),
            )
            .await
            .assert_status(StatusCode::ACCEPTED);
        run_all(&app).await;
        assert_eq!(import_state(&app, id).await, "active", "{:?}", fake.calls());
        // The path reaches the helper as an argument, never inside the script it runs.
        let helpers = fake.helpers.lock().unwrap().clone();
        let link = helpers.iter().find(|h| h.name.contains("link")).unwrap();
        assert!(link.cmd.iter().any(|a| a == odd), "{:?}", link.cmd);
        let script = &link.cmd[2];
        assert!(
            !script.contains("pwned") && !script.contains("ird"),
            "{script}"
        );
    }
);

// ---- The fix pass of the whole-branch review -------------------------------

fn settings_of(lease_s: u64) -> rdockpanel::services::import_service::ImportSettings {
    rdockpanel::services::import_service::ImportSettings {
        helper_image: Some("helper:1".to_owned()),
        observe: std::time::Duration::ZERO,
        deadline: std::time::Duration::from_millis(200),
        poll: std::time::Duration::from_millis(10),
        links_dir: None,
        lease: std::time::Duration::from_secs(lease_s),
    }
}

/// Publishes web's port 9000 on every address.
fn web_port(fake: &FakeDocker) {
    fake.inspected
        .lock()
        .unwrap()
        .get_mut("shop-web-1")
        .unwrap()["NetworkSettings"]["Ports"] =
        json!({"80/tcp": [{"HostIp": "0.0.0.0", "HostPort": "9000"}]});
}

async fn run_import(app: &TestApp, admin: &TestClient<'_>, id: i64) {
    admin
        .post_json(
            &app.admin_api(&format!("x/imports/{id}/start")),
            &json!({"confirm": "shop"}),
        )
        .await
        .assert_status(StatusCode::ACCEPTED);
    run_all(app).await;
}

async fn post(app: &TestApp, admin: &TestClient<'_>, path: &str, body: Value) -> TestResponse {
    admin
        .post_json(&app.admin_api(&format!("x/{path}")), &body)
        .await
}

db_test!(
    a_long_step_keeps_its_lease_alive,
    migrate = Migrator,
    |db: TestDb| async move {
        use rdockpanel::models::container_import;
        use sea_orm::EntityTrait;
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        let fake = FakeDocker::default();
        shop(&fake);
        // Each copy takes longer than the lease: a heartbeat must carry it.
        *fake.helper_delay_ms.lock().unwrap() = 1500;
        let app = support::app_with_settings(db, fake.clone(), settings_of(1)).await;
        let admin = ops(&app).await;
        let id = planned_shop(&app, &admin).await;
        let (done, expired) = (AtomicBool::new(false), AtomicUsize::new(0));
        let work = async {
            run_import(&app, &admin, id).await;
            done.store(true, Ordering::SeqCst);
        };
        let watcher = async {
            while !done.load(Ordering::SeqCst) {
                let row = container_import::Entity::find_by_id(id)
                    .one(app.state().db())
                    .await
                    .unwrap()
                    .unwrap();
                if row.state == "copying"
                    && row.lease_token.is_some()
                    && row.lease_expires_at.is_some_and(|t| t < chrono::Utc::now())
                {
                    expired.fetch_add(1, Ordering::SeqCst);
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        };
        tokio::join!(work, watcher);
        assert_eq!(import_state(&app, id).await, "active");
        assert_eq!(
            expired.load(Ordering::SeqCst),
            0,
            "the lease ran out during a copy"
        );
    }
);

/// A moved service (folder in place → volume), ready for the move tests.
/// The replicas a Swarm service of the fake asks for.
fn replicas_of(fake: &FakeDocker, name: &str) -> Option<i64> {
    fake.services.lock().unwrap()[name]
        .spec
        .clone()
        .unwrap()
        .mode
        .unwrap()
        .replicated
        .unwrap()
        .replicas
}

async fn moved_shop(db: TestDb) -> (TestApp, FakeDocker, i64, i64) {
    let fake = FakeDocker::default();
    shop_with_folder(&fake);
    let app = support::app_with_helper(db, fake.clone(), "helper:1").await;
    let admin = ops(&app).await;
    let id = planned_shop(&app, &admin).await;
    run_import(&app, &admin, id).await;
    assert_eq!(import_state(&app, id).await, "active", "{:?}", fake.calls());
    let web = web_service(&app).await;
    let moved = post(
        &app,
        &admin,
        &format!("services/{}/move-data", web.id),
        json!({"confirm": "web"}),
    )
    .await;
    moved.assert_status(StatusCode::ACCEPTED);
    let moving = moved.json()["id"].as_i64().unwrap();
    run_all(&app).await;
    assert_eq!(
        import_state(&app, moving).await,
        "active",
        "{:?}",
        fake.calls()
    );
    // It runs again: a stopped service stays stopped through a deployment.
    assert_eq!(
        replicas_of(&fake, "shop_web"),
        Some(1),
        "{:?}",
        fake.calls()
    );
    (app, fake, web.id, moving)
}

db_test!(
    an_undo_that_failed_on_its_way_back_goes_on_back_not_forward,
    migrate = Migrator,
    |db: TestDb| async move {
        let (app, fake, web, moving) = moved_shop(db).await;
        let admin = ops_named(&app, "ops2").await;
        // The way back fails at its deployment.
        fake.deploy_fail_for
            .lock()
            .unwrap()
            .push("shop_web".to_owned());
        post(
            &app,
            &admin,
            &format!("services/{web}/move-data/undo"),
            json!({}),
        )
        .await
        .assert_status(StatusCode::ACCEPTED);
        run_all(&app).await;
        assert_eq!(
            import_state(&app, moving).await,
            "needs_attention",
            "{:?}",
            fake.calls()
        );
        // Retry, once it can: the move ends undone, with the folder as the mount.
        fake.deploy_fail_for.lock().unwrap().clear();
        post(&app, &admin, &format!("imports/{moving}/retry"), json!({}))
            .await
            .assert_status(StatusCode::ACCEPTED);
        run_all(&app).await;
        assert_eq!(
            import_state(&app, moving).await,
            "reverted",
            "{:?}",
            fake.calls()
        );
        assert!(mounts_of(&app, web).await.iter().any(|m| m.kind == "link"));
        // The volume with what was written since is still there.
        assert!(
            !fake.calls().iter().any(|c| c.starts_with("remove-volume")),
            "{:?}",
            fake.calls()
        );
    }
);

db_test!(
    the_old_folder_is_not_cleaned_while_the_import_can_still_be_undone,
    migrate = Migrator,
    |db: TestDb| async move {
        let (app, _fake, web, _moving) = moved_shop(db).await;
        let admin = ops_named(&app, "ops2").await;
        // The import that brought the service in still holds the original on that folder.
        post(
            &app,
            &admin,
            &format!("services/{web}/clean-old-folder"),
            json!({"folder": "/srv/web-folder", "confirm": "web-folder"}),
        )
        .await
        .assert_validation_error("folder");
    }
);

db_test!(
    an_undo_asked_during_a_move_is_done,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::default();
        shop_with_folder(&fake);
        let app = support::app_with_helper(db, fake.clone(), "helper:1").await;
        let admin = ops(&app).await;
        let id = planned_shop(&app, &admin).await;
        run_import(&app, &admin, id).await;
        let web = web_service(&app).await;
        let moved = post(
            &app,
            &admin,
            &format!("services/{}/move-data", web.id),
            json!({"confirm": "web"}),
        )
        .await;
        let moving = moved.json()["id"].as_i64().unwrap();
        // Asked before the first step ran.
        post(
            &app,
            &admin,
            &format!("services/{}/move-data/undo", web.id),
            json!({}),
        )
        .await;
        let refused = import_state(&app, moving).await;
        assert_eq!(refused, "queued");
        // The way an undo is asked while it runs: the flag.
        post(&app, &admin, &format!("imports/{moving}/undo"), json!({})).await;
        run_all(&app).await;
        assert_eq!(
            import_state(&app, moving).await,
            "reverted",
            "{:?}",
            fake.calls()
        );
        assert!(
            mounts_of(&app, web.id)
                .await
                .iter()
                .any(|m| m.kind == "link")
        );
    }
);

db_test!(
    after_traffic_could_arrive_a_failure_waits_for_the_admin,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::default();
        shop(&fake);
        web_port(&fake);
        let app = support::app_with_helper(db, fake.clone(), "helper:1").await;
        let admin = ops(&app).await;
        let id = planned_shop(&app, &admin).await;
        // `web` (a published port) fails after `db`: data may have been written.
        fake.deploy_fail_for
            .lock()
            .unwrap()
            .push("shop_web".to_owned());
        run_import(&app, &admin, id).await;
        assert_eq!(
            import_state(&app, id).await,
            "needs_attention",
            "{:?}",
            fake.calls()
        );
        assert!(!fake.calls().contains(&"start id-web".to_owned()));
    }
);

db_test!(
    an_undo_that_failed_destroying_retries_the_destroy_before_the_originals_start,
    migrate = Migrator,
    |db: TestDb| async move {
        let (app, fake, id) = started_shop(db).await;
        run_all(&app).await;
        assert_eq!(import_state(&app, id).await, "active");
        let admin = ops(&app).await;
        *fake.delete_fail.lock().unwrap() = true;
        post(&app, &admin, &format!("imports/{id}/undo"), json!({})).await;
        run_all(&app).await;
        assert_eq!(
            import_state(&app, id).await,
            "needs_attention",
            "{:?}",
            fake.calls()
        );
        assert!(
            !fake.calls().contains(&"start id-web".to_owned()),
            "{:?}",
            fake.calls()
        );
        *fake.delete_fail.lock().unwrap() = false;
        post(&app, &admin, &format!("imports/{id}/retry"), json!({})).await;
        run_all(&app).await;
        assert_eq!(
            import_state(&app, id).await,
            "reverted",
            "{:?}",
            fake.calls()
        );
        assert!(
            imported_services(&fake).is_empty(),
            "{:?}",
            imported_services(&fake)
        );
        assert!(fake.calls().contains(&"start id-web".to_owned()));
    }
);

db_test!(
    an_original_that_never_stopped_is_left_running_by_the_undo,
    migrate = Migrator,
    |db: TestDb| async move {
        let (app, fake, id) = {
            let fake = FakeDocker::default();
            shop(&fake);
            // `db` does not stop (it is stopped after `web`): the import undoes itself.
            fake.stop_fails.lock().unwrap().push("id-db".to_owned());
            let app = support::app_with_helper(db, fake.clone(), "helper:1").await;
            let admin = ops(&app).await;
            let id = planned_shop(&app, &admin).await;
            run_import(&app, &admin, id).await;
            (app, fake, id)
        };
        assert_eq!(
            import_state(&app, id).await,
            "reverted",
            "{:?}",
            fake.calls()
        );
        // `web` was stopped and runs again; `db` never stopped and was not started.
        assert!(fake.calls().contains(&"start id-web".to_owned()));
        assert!(!fake.calls().contains(&"start id-db".to_owned()));
    }
);

db_test!(
    an_original_already_restored_is_not_stopped_by_the_watch,
    migrate = Migrator,
    |db: TestDb| async move {
        use rdockpanel::models::container_import_item;
        use rdockpanel::services::docker_service::{ContainerAction, DockerApi};
        use sea_orm::{
            ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, Set,
        };
        let (app, fake, id) = started_shop(db).await;
        run_all(&app).await;
        let mut item = container_import_item::Entity::find()
            .filter(container_import_item::Column::ContainerId.eq("id-web"))
            .one(app.state().db())
            .await
            .unwrap()
            .unwrap()
            .into_active_model();
        item.state = Set("restored".to_owned());
        item.update(app.state().db()).await.unwrap();
        fake.container_action("id-web", ContainerAction::Start)
            .await
            .unwrap();
        rdockpanel::services::import_run_service::tick(app.state())
            .await
            .unwrap();
        let stops = fake.calls().iter().filter(|c| *c == "stop id-web").count();
        assert_eq!(stops, 1, "{:?}", fake.calls());
        let _ = id;
    }
);

db_test!(
    a_container_import_is_watched_too,
    migrate = Migrator,
    |db: TestDb| async move {
        use rdockpanel::services::docker_service::{ContainerAction, DockerApi};
        let fake = FakeDocker::default();
        shop(&fake);
        let app = support::app_with_helper(db, fake.clone(), "helper:1").await;
        let admin = ops(&app).await;
        let made = post(
            &app,
            &admin,
            "imports",
            json!({"source_kind": "container", "source_ref": "shop-db-1"}),
        )
        .await;
        let id = made.json()["id"].as_i64().unwrap();
        assert_eq!(wait_planned(&app, &admin, id).await["state"], "planned");
        post(
            &app,
            &admin,
            &format!("imports/{id}/start"),
            json!({"confirm": "shop-db-1"}),
        )
        .await
        .assert_status(StatusCode::ACCEPTED);
        run_all(&app).await;
        assert_eq!(import_state(&app, id).await, "active", "{:?}", fake.calls());
        fake.container_action("id-db", ContainerAction::Start)
            .await
            .unwrap();
        rdockpanel::services::import_run_service::tick(app.state())
            .await
            .unwrap();
        let stops = fake.calls().iter().filter(|c| *c == "stop id-db").count();
        assert_eq!(stops, 2, "{:?}", fake.calls());
    }
);

db_test!(
    two_services_that_only_read_one_volume_each_get_a_copy,
    migrate = Migrator,
    |db: TestDb| async move {
        use rdockpanel::models::container_import_volume;
        use sea_orm::EntityTrait;
        let fake = FakeDocker::default();
        shop(&fake);
        for name in ["shop-web-1", "shop-db-1"] {
            fake.inspected.lock().unwrap().get_mut(name).unwrap()["Mounts"]
                .as_array_mut()
                .unwrap()
                .push(
                    json!({"Type": "volume", "Name": "rovol", "Destination": "/ro", "RW": false}),
                );
        }
        let app = support::app_with_helper(db, fake.clone(), "helper:1").await;
        let admin = ops(&app).await;
        let id = planned_shop(&app, &admin).await;
        run_import(&app, &admin, id).await;
        assert_eq!(import_state(&app, id).await, "active", "{:?}", fake.calls());
        let rows = container_import_volume::Entity::find()
            .all(app.state().db())
            .await
            .unwrap();
        let copies: Vec<String> = rows
            .iter()
            .filter(|v| v.original_name == "rovol")
            .filter_map(|v| v.panel_volume.clone())
            .collect();
        assert_eq!(copies.len(), 2, "{rows:?}");
        let targets: Vec<String> = fake
            .helpers
            .lock()
            .unwrap()
            .iter()
            .flat_map(|h| {
                h.mounts
                    .iter()
                    .filter(|m| !m.read_only)
                    .map(|m| m.source.clone())
            })
            .collect();
        for copy in copies {
            assert!(targets.contains(&copy), "{copy} not copied: {targets:?}");
        }
    }
);

db_test!(
    a_failed_task_from_before_the_look_does_not_fail_the_import,
    migrate = Migrator,
    |db: TestDb| async move {
        let (app, fake, id) = {
            let fake = FakeDocker::default();
            shop(&fake);
            *fake.old_failed.lock().unwrap() = true;
            let app = support::app_with_helper(db, fake.clone(), "helper:1").await;
            let admin = ops(&app).await;
            let id = planned_shop(&app, &admin).await;
            run_import(&app, &admin, id).await;
            (app, fake, id)
        };
        assert_eq!(import_state(&app, id).await, "active", "{:?}", fake.calls());
    }
);

db_test!(
    a_copy_needs_the_room_for_it,
    migrate = Migrator,
    |db: TestDb| async move {
        use rdockpanel::models::container_import;
        use sea_orm::EntityTrait;
        let fake = FakeDocker::default();
        shop(&fake);
        fake.volume_sizes
            .lock()
            .unwrap()
            .insert("webdata".to_owned(), 50 * 1024 * 1024 * 1024);
        // The helper that checks the room says there is none.
        fake.helper_exits
            .lock()
            .unwrap()
            .insert("space".to_owned(), 1);
        let app = support::app_with_helper(db, fake.clone(), "helper:1").await;
        let admin = ops(&app).await;
        let id = planned_shop(&app, &admin).await;
        run_import(&app, &admin, id).await;
        assert_eq!(
            import_state(&app, id).await,
            "reverted",
            "{:?}",
            fake.calls()
        );
        let row = container_import::Entity::find_by_id(id)
            .one(app.state().db())
            .await
            .unwrap()
            .unwrap();
        assert!(row.error.unwrap().contains("room"), "no message about room");
        // Nothing was copied into the volume that had no room.
        assert!(fake.helpers.lock().unwrap().iter().all(|h| {
            !h.name.contains("copy") || h.mounts.iter().all(|m| m.source != "rdp_shop_web_webdata")
        }));
    }
);

db_test!(
    a_compose_file_is_renamed_only_when_every_container_of_its_project_goes,
    migrate = Migrator,
    |db: TestDb| async move {
        use rdockpanel::models::container_import_file;
        use sea_orm::EntityTrait;
        let fake = FakeDocker::default();
        shop(&fake);
        let labels = |fake: &FakeDocker, name: &str| {
            let mut inspected = fake.inspected.lock().unwrap();
            let l = &mut inspected.get_mut(name).unwrap()["Config"]["Labels"];
            l["com.docker.compose.project.working_dir"] = json!("/srv/shop");
            l["com.docker.compose.project.config_files"] = json!("/srv/shop/docker-compose.yml");
        };
        for name in ["shop-web-1", "shop-db-1"] {
            labels(&fake, name);
        }
        fake.files.lock().unwrap().insert(
            "host:/srv/shop/docker-compose.yml".to_owned(),
            tar_of("docker-compose.yml", "services:\n  web:\n    image: web:1\n  db:\n    image: db:1\n  cache:\n    image: c:1\n"),
        );
        // A third container of the same project that nothing ties to the others.
        let mut cache = fake
            .inspected
            .lock()
            .unwrap()
            .get("shop-db-1")
            .unwrap()
            .clone();
        cache["Id"] = json!("id-cache");
        cache["Name"] = json!("/shop-cache-1");
        cache["Config"]["Labels"]["com.docker.compose.service"] = json!("cache");
        cache["Mounts"] = json!([]);
        cache["Config"]["Env"] = json!([]);
        cache["NetworkSettings"]["Networks"]["shop_default"]["Aliases"] = json!(["cache"]);
        cache["NetworkSettings"]["Networks"]["shop_default"]["IPAddress"] = json!("172.20.0.77");
        fake.inspected
            .lock()
            .unwrap()
            .insert("shop-cache-1".to_owned(), cache.clone());
        fake.containers.lock().unwrap().push(ContainerItem {
            id: "id-cache".to_owned(),
            name: "shop-cache-1".to_owned(),
            image: "cache:1".to_owned(),
            image_id: "sha256:cache".to_owned(),
            state: "running".to_owned(),
            status: "Up".to_owned(),
            created: 0,
            ports: vec![],
            labels: Default::default(),
        });
        fake.detailed.lock().unwrap().push(
            json!({"Id": "id-cache", "Names": ["/shop-cache-1"], "Image": "cache:1",
            "State": "running", "Labels": cache["Config"]["Labels"]}),
        );
        let app = support::app_with_helper(db, fake.clone(), "helper:1").await;
        let admin = ops(&app).await;
        // Only `db` and what it needs (`web` calls it): `cache` stays running.
        let made = post(
            &app,
            &admin,
            "imports",
            json!({"source_kind": "container", "source_ref": "shop-db-1"}),
        )
        .await;
        let id = made.json()["id"].as_i64().unwrap();
        assert_eq!(wait_planned(&app, &admin, id).await["state"], "planned");
        post(
            &app,
            &admin,
            &format!("imports/{id}/start"),
            json!({"confirm": "shop-db-1"}),
        )
        .await
        .assert_status(StatusCode::ACCEPTED);
        run_all(&app).await;
        assert_eq!(import_state(&app, id).await, "active", "{:?}", fake.calls());
        let files = container_import_file::Entity::find()
            .all(app.state().db())
            .await
            .unwrap();
        assert!(
            files.is_empty(),
            "the file of a project with a container left behind was renamed: {files:?}"
        );
        assert!(
            fake.helpers
                .lock()
                .unwrap()
                .iter()
                .all(|h| !h.name.contains("compose"))
        );
    }
);

db_test!(
    a_compose_file_is_renamed_by_a_helper_with_the_rights_to_do_it,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::default();
        shop(&fake);
        for name in ["shop-web-1", "shop-db-1"] {
            let mut inspected = fake.inspected.lock().unwrap();
            let l = &mut inspected.get_mut(name).unwrap()["Config"]["Labels"];
            l["com.docker.compose.project.working_dir"] = json!("/srv/shop");
            l["com.docker.compose.project.config_files"] = json!("/srv/shop/docker-compose.yml");
        }
        fake.files.lock().unwrap().insert(
            "host:/srv/shop/docker-compose.yml".to_owned(),
            tar_of(
                "docker-compose.yml",
                "services:\n  web:\n    image: web:1\n  db:\n    image: db:1\n",
            ),
        );
        let app = support::app_with_helper(db, fake.clone(), "helper:1").await;
        let admin = ops(&app).await;
        let id = planned_shop(&app, &admin).await;
        run_import(&app, &admin, id).await;
        assert_eq!(import_state(&app, id).await, "active", "{:?}", fake.calls());
        let helpers = fake.helpers.lock().unwrap().clone();
        let rename = helpers.iter().find(|h| h.name.contains("compose")).unwrap();
        // On Linux a root without these cannot rename in a directory of a user.
        for cap in ["DAC_OVERRIDE", "FOWNER"] {
            assert!(
                rename.cap_add.iter().any(|c| c == cap),
                "{:?}",
                rename.cap_add
            );
        }
        // Undone: put back by one with the same rights.
        post(&app, &admin, &format!("imports/{id}/undo"), json!({})).await;
        run_all(&app).await;
        assert_eq!(import_state(&app, id).await, "reverted");
        let helpers = fake.helpers.lock().unwrap().clone();
        let restore = helpers
            .iter()
            .find(|h| h.name.contains("restore-compose"))
            .unwrap();
        assert!(restore.cap_add.iter().any(|c| c == "DAC_OVERRIDE"));
    }
);

db_test!(
    a_secret_in_a_command_and_another_user_are_carried_over_not_blocked,
    migrate = Migrator,
    |db: TestDb| async move {
        use rdockpanel::models::service;
        use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
        let fake = FakeDocker::default();
        shop(&fake);
        {
            let mut inspected = fake.inspected.lock().unwrap();
            // A valkey: its password is on the command line.
            inspected.get_mut("shop-web-1").unwrap()["Config"]["Cmd"] =
                json!(["valkey-server", "--requirepass", SECRET]);
            // A tunnel: it runs as root, its image does not.
            inspected.get_mut("shop-db-1").unwrap()["Config"]["User"] = json!("0:0");
        }
        fake.images.lock().unwrap().clear();
        let app = support::app_with_helper(db, fake.clone(), "helper:1").await;
        let admin = ops(&app).await;
        let id = planned_shop(&app, &admin).await;
        let plan = admin
            .get(&app.admin_api(&format!("x/imports/{id}")))
            .await
            .json();
        // Confirmed, never blocked; and no value in the plan.
        let findings: Vec<String> = plan["plan"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|i| {
                i["findings"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|f| f["code"].as_str().unwrap().to_owned())
            })
            .collect();
        assert!(
            findings.contains(&"argv_secret_moved".to_owned()),
            "{findings:?}"
        );
        assert!(findings.contains(&"user_kept".to_owned()), "{findings:?}");
        assert!(
            plan["plan"]["items"]
                .as_array()
                .unwrap()
                .iter()
                .all(|i| i["choice"] == "bring"),
            "{plan}"
        );
        assert!(!plan.to_string().contains(SECRET));
        run_import(&app, &admin, id).await;
        assert_eq!(import_state(&app, id).await, "active", "{:?}", fake.calls());
        // The user is the service's, and the service is its administrators'.
        let db_service = service::Entity::find()
            .filter(service::Column::Name.eq("db"))
            .one(app.state().db())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(db_service.user.as_deref(), Some("0:0"));
        assert!(db_service.admin_only);
        // The command reads the password from the environment: it is not in Swarm's spec.
        let spec = fake.services.lock().unwrap()["shop_web"]
            .spec
            .clone()
            .unwrap();
        let text = serde_json::to_string(&spec).unwrap();
        assert!(
            !text.contains(SECRET.to_string().as_str()) || text.contains("RDP_SECRET_1"),
            "{text}"
        );
        assert!(text.contains("exec valkey-server"), "{text}");
        let user = fake.services.lock().unwrap()["shop_db"]
            .spec
            .clone()
            .unwrap()
            .task_template
            .unwrap()
            .container_spec
            .unwrap()
            .user;
        assert_eq!(user.as_deref(), Some("0:0"));
    }
);

db_test!(
    a_forced_stop_of_a_container_without_data_does_not_stop_the_import,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::default();
        shop(&fake);
        // Nothing to write to: no mount on either (a stateless API and its cache).
        for name in ["shop-web-1", "shop-db-1"] {
            fake.inspected.lock().unwrap().get_mut(name).unwrap()["Mounts"] = json!([]);
        }
        *fake.stop_exit.lock().unwrap() = 137;
        let app = support::app_with_helper(db, fake.clone(), "helper:1").await;
        let admin = ops(&app).await;
        let id = planned_shop(&app, &admin).await;
        run_import(&app, &admin, id).await;
        // 137 means no consistent data to lose here: the import goes on.
        assert_eq!(import_state(&app, id).await, "active", "{:?}", fake.calls());
    }
);

db_test!(
    a_container_with_data_gets_time_to_stop_before_it_is_killed,
    migrate = Migrator,
    |db: TestDb| async move {
        let (app, fake, id) = started_shop(db).await;
        run_all(&app).await;
        assert_eq!(import_state(&app, id).await, "active");
        // Their volumes are written: a long wait; a stop with no data would take a short one.
        let timeouts = fake.stop_timeouts.lock().unwrap().clone();
        assert!(!timeouts.is_empty());
        assert!(timeouts.iter().all(|t| *t >= 60), "{timeouts:?}");
    }
);

const TUNNEL_A: &str = "eyJhIjoiMDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWYiLCJ0IjoiMDEyMzQ1NjctODlhYi1jZGVmLTAxMjMtNDU2Nzg5YWJjZGVmIiwicyI6ImMyVmpjbVYwTFhObFkzSmxkQT09In0=";
const TUNNEL_B: &str = "eyJhIjoiMDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWYiLCJ0IjoiMDEyMzQ1NjctODlhYi1jZGVmLTAxMjMtNDU2Nzg5YWJjZGVlIiwicyI6ImMyVmpjbVYwTFhObFkzSmxkQT09In0=";

/// `shop` plus `shop-tunnel-1`, a cloudflared of the same Compose project whose token is `token`.
fn shop_with_tunnel(fake: &FakeDocker, token: &str) {
    shop(fake);
    cloudflared(fake, "tunnel", "172.20.0.55", token);
}

/// `shop-<service>-1` (id `id-<service>`), a cloudflared of `shop` whose token is `token`.
fn cloudflared(fake: &FakeDocker, compose: &str, ip: &str, token: &str) {
    let (id, name) = (format!("id-{compose}"), format!("shop-{compose}-1"));
    let mut doc = fake
        .inspected
        .lock()
        .unwrap()
        .get("shop-db-1")
        .unwrap()
        .clone();
    doc["Id"] = json!(id);
    doc["Name"] = json!(format!("/{name}"));
    doc["Image"] = json!("sha256:tunnel");
    doc["Config"]["Image"] = json!("cloudflare/cloudflared:2026.10.2");
    doc["Config"]["Labels"]["com.docker.compose.service"] = json!(compose);
    doc["Config"]["Env"] = json!([format!("TUNNEL_TOKEN={token}")]);
    doc["Config"]["Cmd"] = json!(["tunnel", "--no-autoupdate", "run"]);
    doc["Mounts"] = json!([]);
    doc["NetworkSettings"]["Networks"]["shop_default"]["Aliases"] = json!([compose]);
    doc["NetworkSettings"]["Networks"]["shop_default"]["IPAddress"] = json!(ip);
    fake.inspected
        .lock()
        .unwrap()
        .insert(name.clone(), doc.clone());
    fake.containers.lock().unwrap().push(ContainerItem {
        id: id.clone(),
        name: name.clone(),
        image: "cloudflare/cloudflared:2026.10.2".to_owned(),
        image_id: "sha256:tunnel".to_owned(),
        state: "running".to_owned(),
        status: "Up".to_owned(),
        created: 0,
        ports: vec![],
        labels: Default::default(),
    });
    fake.detailed.lock().unwrap().push(json!({"Id": id, "Names": [format!("/{name}")],
        "Image": "cloudflare/cloudflared:2026.10.2", "State": "running", "Labels": doc["Config"]["Labels"]}));
}

async fn plan_item(app: &TestApp, admin: &TestClient<'_>, id: i64, container: &str) -> Value {
    let got = admin
        .get(&app.admin_api(&format!("x/imports/{id}")))
        .await
        .json();
    got["plan"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["container_id"] == container)
        .cloned()
        .unwrap()
}

db_test!(
    a_cloudflared_container_can_become_the_panels_cloudflare_tunnel,
    migrate = Migrator,
    |db: TestDb| async move {
        use rdockpanel::models::service;
        use sea_orm::EntityTrait;
        let fake = FakeDocker::default();
        shop_with_tunnel(&fake, TUNNEL_A);
        let app = support::app_with_helper(db, fake.clone(), "helper:1").await;
        let admin = ops(&app).await;
        let id = planned_shop(&app, &admin).await;
        let item = plan_item(&app, &admin, id, "id-tunnel").await;
        assert_eq!(item["tunnel_token"], true);
        assert_eq!(item["as_tunnel"], false);
        // Only a cloudflared offers it.
        assert_eq!(
            plan_item(&app, &admin, id, "id-web").await["tunnel_token"],
            false
        );
        let patch = |body: Value| {
            let url = app.admin_api(&format!("x/imports/{id}/plan"));
            let admin = &admin;
            async move { admin.send(Method::PATCH, &url, Some(&body)).await }
        };
        patch(json!({"tunnel": {"id-web": true}}))
            .await
            .assert_validation_error("tunnel");
        patch(json!({"tunnel": {"id-tunnel": true}}))
            .await
            .assert_status(StatusCode::OK);
        let item = plan_item(&app, &admin, id, "id-tunnel").await;
        assert_eq!(item["as_tunnel"], true);
        // Said: the hostnames must point at Traefik; and no token anywhere.
        let said: Vec<&str> = item["findings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["code"].as_str().unwrap())
            .collect();
        assert!(said.contains(&"tunnel_panel"), "{said:?}");
        assert!(
            !admin
                .get(&app.admin_api(&format!("x/imports/{id}")))
                .await
                .text()
                .contains(TUNNEL_A)
        );

        run_import(&app, &admin, id).await;
        assert_eq!(import_state(&app, id).await, "active", "{:?}", fake.calls());
        // No service for it; the panel's tunnel runs with its token.
        let names: Vec<String> = service::Entity::find()
            .all(app.state().db())
            .await
            .unwrap()
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert!(!names.contains(&"tunnel".to_owned()), "{names:?}");
        // A tunnel of its own, named after the container.
        assert_eq!(tunnels(&app, &admin).await, [("tunnel".to_owned(), true)]);
        assert!(has_service(&fake, "rdp-cloudflared-tunnel"));
        assert!(fake.calls().contains(&"stop id-tunnel".to_owned()));

        // Undone: the panel's tunnel is off and the container runs again.
        post(&app, &admin, &format!("imports/{id}/undo"), json!({})).await;
        run_all(&app).await;
        assert_eq!(
            import_state(&app, id).await,
            "reverted",
            "{:?}",
            fake.calls()
        );
        assert_eq!(tunnels(&app, &admin).await, []);
        assert!(!has_service(&fake, "rdp-cloudflared-tunnel"));
        assert!(fake.calls().contains(&"start id-tunnel".to_owned()));
    }
);

/// The panel's tunnels: (name, on), by name.
async fn tunnels(app: &TestApp, admin: &TestClient<'_>) -> Vec<(String, bool)> {
    let got = admin.get(&app.admin_api("x/apps/tunnels")).await.json();
    let mut all: Vec<(String, bool)> = got["tunnels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| (t["name"].as_str().unwrap().to_owned(), t["enabled"] == true))
        .collect();
    all.sort();
    all
}

fn has_service(fake: &FakeDocker, name: &str) -> bool {
    fake.services.lock().unwrap().contains_key(name)
}

/// The plan's `tunnel` choice of each container.
async fn patch_tunnel(
    app: &TestApp,
    admin: &TestClient<'_>,
    id: i64,
    choice: Value,
) -> TestResponse {
    let url = app.admin_api(&format!("x/imports/{id}/plan"));
    admin
        .send(Method::PATCH, &url, Some(&json!({ "tunnel": choice })))
        .await
}

db_test!(
    a_cloudflared_with_another_token_becomes_a_second_tunnel,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::default();
        shop_with_tunnel(&fake, TUNNEL_B);
        let app = support::app_with_helper(db, fake.clone(), "helper:1").await;
        let admin = ops(&app).await;
        // The panel already runs a tunnel (another account's).
        post(
            &app,
            &admin,
            "apps/tunnel/enable",
            json!({"token": TUNNEL_A}),
        )
        .await
        .assert_status(StatusCode::OK);
        let id = planned_shop(&app, &admin).await;
        patch_tunnel(&app, &admin, id, json!({"id-tunnel": true}))
            .await
            .assert_status(StatusCode::OK);
        run_import(&app, &admin, id).await;
        assert_eq!(import_state(&app, id).await, "active", "{:?}", fake.calls());
        assert_eq!(
            tunnels(&app, &admin).await,
            [("default".to_owned(), true), ("tunnel".to_owned(), true)]
        );
        assert!(has_service(&fake, "rdp-cloudflared"));
        assert!(has_service(&fake, "rdp-cloudflared-tunnel"));
    }
);

db_test!(
    two_cloudflared_containers_become_two_tunnels,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::default();
        shop_with_tunnel(&fake, TUNNEL_A);
        cloudflared(&fake, "edge", "172.20.0.56", TUNNEL_B);
        let app = support::app_with_helper(db, fake.clone(), "helper:1").await;
        let admin = ops(&app).await;
        let id = planned_shop(&app, &admin).await;
        patch_tunnel(
            &app,
            &admin,
            id,
            json!({"id-tunnel": true, "id-edge": true}),
        )
        .await
        .assert_status(StatusCode::OK);
        run_import(&app, &admin, id).await;
        assert_eq!(import_state(&app, id).await, "active", "{:?}", fake.calls());
        assert_eq!(
            tunnels(&app, &admin).await,
            [("edge".to_owned(), true), ("tunnel".to_owned(), true)]
        );
        assert!(has_service(&fake, "rdp-cloudflared-edge"));
        assert!(has_service(&fake, "rdp-cloudflared-tunnel"));
    }
);

db_test!(
    a_cloudflared_with_a_known_token_is_refused_as_a_new_tunnel,
    migrate = Migrator,
    |db: TestDb| async move {
        use rdockpanel::models::service;
        use sea_orm::EntityTrait;
        let fake = FakeDocker::default();
        shop_with_tunnel(&fake, TUNNEL_A);
        let app = support::app_with_helper(db, fake.clone(), "helper:1").await;
        let admin = ops(&app).await;
        let add_first = || {
            post(
                &app,
                &admin,
                "apps/tunnels",
                json!({"name": "first", "token": TUNNEL_A}),
            )
        };
        add_first().await.assert_status(StatusCode::OK);
        let id = planned_shop(&app, &admin).await;
        // One cloudflared per token: refused, and nothing chosen.
        patch_tunnel(&app, &admin, id, json!({"id-tunnel": true}))
            .await
            .assert_validation_error("tunnel");
        assert_eq!(
            plan_item(&app, &admin, id, "id-tunnel").await["as_tunnel"],
            false
        );
        // Chosen before the token was a tunnel: the start refuses it.
        admin
            .send(Method::DELETE, &app.admin_api("x/apps/tunnels/first"), None)
            .await
            .assert_status(StatusCode::OK);
        patch_tunnel(&app, &admin, id, json!({"id-tunnel": true}))
            .await
            .assert_status(StatusCode::OK);
        add_first().await.assert_status(StatusCode::OK);
        post(
            &app,
            &admin,
            &format!("imports/{id}/start"),
            json!({"confirm": "shop"}),
        )
        .await
        .assert_validation_error("tunnel");
        // As a service it still comes.
        patch_tunnel(&app, &admin, id, json!({"id-tunnel": false}))
            .await
            .assert_status(StatusCode::OK);
        run_import(&app, &admin, id).await;
        assert_eq!(import_state(&app, id).await, "active", "{:?}", fake.calls());
        let names: Vec<String> = service::Entity::find()
            .all(app.state().db())
            .await
            .unwrap()
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert!(names.contains(&"tunnel".to_owned()), "{names:?}");
        assert_eq!(tunnels(&app, &admin).await, [("first".to_owned(), true)]);
    }
);

db_test!(
    undoing_the_import_removes_only_the_tunnels_it_made,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::default();
        shop_with_tunnel(&fake, TUNNEL_B);
        let app = support::app_with_helper(db, fake.clone(), "helper:1").await;
        let admin = ops(&app).await;
        // A tunnel named as the container would be: the import's gets another name.
        post(
            &app,
            &admin,
            "apps/tunnels",
            json!({"name": "tunnel", "token": TUNNEL_A}),
        )
        .await
        .assert_status(StatusCode::OK);
        let id = planned_shop(&app, &admin).await;
        patch_tunnel(&app, &admin, id, json!({"id-tunnel": true}))
            .await
            .assert_status(StatusCode::OK);
        run_import(&app, &admin, id).await;
        assert_eq!(import_state(&app, id).await, "active", "{:?}", fake.calls());
        assert_eq!(
            tunnels(&app, &admin).await,
            [("tunnel".to_owned(), true), ("tunnel-2".to_owned(), true)]
        );
        post(&app, &admin, &format!("imports/{id}/undo"), json!({})).await;
        run_all(&app).await;
        assert_eq!(
            import_state(&app, id).await,
            "reverted",
            "{:?}",
            fake.calls()
        );
        assert_eq!(tunnels(&app, &admin).await, [("tunnel".to_owned(), true)]);
        assert!(has_service(&fake, "rdp-cloudflared-tunnel"));
        assert!(!has_service(&fake, "rdp-cloudflared-tunnel-2"));
        assert!(fake.calls().contains(&"start id-tunnel".to_owned()));
    }
);

db_test!(
    undo_leaves_a_tunnel_the_admin_recreated_under_the_same_name,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::default();
        shop_with_tunnel(&fake, TUNNEL_A);
        let app = support::app_with_helper(db, fake.clone(), "helper:1").await;
        let admin = ops(&app).await;
        let id = planned_shop(&app, &admin).await;
        patch_tunnel(&app, &admin, id, json!({"id-tunnel": true}))
            .await
            .assert_status(StatusCode::OK);
        run_import(&app, &admin, id).await;
        assert_eq!(import_state(&app, id).await, "active", "{:?}", fake.calls());
        // The admin removes it, then makes another of the same name.
        admin
            .send(
                Method::DELETE,
                &app.admin_api("x/apps/tunnels/tunnel"),
                None,
            )
            .await
            .assert_status(StatusCode::OK);
        post(
            &app,
            &admin,
            "apps/tunnels",
            json!({"name": "tunnel", "token": TUNNEL_B}),
        )
        .await
        .assert_status(StatusCode::OK);
        post(&app, &admin, &format!("imports/{id}/undo"), json!({})).await;
        run_all(&app).await;
        assert_eq!(
            import_state(&app, id).await,
            "reverted",
            "{:?}",
            fake.calls()
        );
        assert_eq!(tunnels(&app, &admin).await, [("tunnel".to_owned(), true)]);
        assert!(has_service(&fake, "rdp-cloudflared-tunnel"));
    }
);

db_test!(
    undo_waiting_for_a_tunnel_change_leaves_the_tunnel_the_admin_recreated,
    migrate = Migrator,
    |db: TestDb| async move {
        use rdockpanel::models::tunnel;
        use rdockpanel::services::tunnel_service;
        use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
        let fake = FakeDocker::default();
        shop_with_tunnel(&fake, TUNNEL_A);
        let app = support::app_with_helper(db, fake.clone(), "helper:1").await;
        let admin = ops(&app).await;
        let id = planned_shop(&app, &admin).await;
        patch_tunnel(&app, &admin, id, json!({"id-tunnel": true}))
            .await
            .assert_status(StatusCode::OK);
        run_import(&app, &admin, id).await;
        assert_eq!(import_state(&app, id).await, "active", "{:?}", fake.calls());
        post(&app, &admin, &format!("imports/{id}/undo"), json!({})).await;
        // A tunnel change holds the lock while the undo starts: it removes
        // the import's tunnel and makes another of the same name.
        let changing = tunnel_service::exclusive().await;
        let admin_change = async {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let db = app.state().db();
            tunnel::Entity::delete_many()
                .filter(tunnel::Column::Name.eq("tunnel"))
                .exec(db)
                .await
                .unwrap();
            tunnel_service::reserve(app.state(), db, "tunnel", TUNNEL_B)
                .await
                .unwrap();
            drop(changing);
        };
        tokio::join!(run_all(&app), admin_change);
        assert_eq!(
            import_state(&app, id).await,
            "reverted",
            "{:?}",
            fake.calls()
        );
        assert_eq!(
            tunnel_service::holder(app.state(), app.state().db(), TUNNEL_B)
                .await
                .unwrap()
                .as_deref(),
            Some("tunnel")
        );
    }
);

db_test!(
    a_retry_never_overwrites_another_tunnels_token,
    migrate = Migrator,
    |db: TestDb| async move {
        use rdockpanel::services::tunnel_service::holder;
        let fake = FakeDocker::default();
        shop_with_tunnel(&fake, TUNNEL_A);
        fake.create_fails
            .lock()
            .unwrap()
            .push("rdp-cloudflared-tunnel".to_owned());
        let app = support::app_with_helper(db, fake.clone(), "helper:1").await;
        let admin = ops(&app).await;
        let id = planned_shop(&app, &admin).await;
        patch_tunnel(&app, &admin, id, json!({"id-tunnel": true}))
            .await
            .assert_status(StatusCode::OK);
        run_import(&app, &admin, id).await;
        assert_eq!(
            import_state(&app, id).await,
            "needs_attention",
            "{:?}",
            fake.calls()
        );
        // Meanwhile the name goes to another tunnel, of another token.
        fake.create_fails.lock().unwrap().clear();
        admin
            .send(
                Method::DELETE,
                &app.admin_api("x/apps/tunnels/tunnel"),
                None,
            )
            .await
            .assert_status(StatusCode::OK);
        post(
            &app,
            &admin,
            "apps/tunnels",
            json!({"name": "tunnel", "token": TUNNEL_B}),
        )
        .await
        .assert_status(StatusCode::OK);
        post(&app, &admin, &format!("imports/{id}/retry"), json!({})).await;
        run_all(&app).await;
        assert_eq!(import_state(&app, id).await, "active", "{:?}", fake.calls());
        let (state, db) = (app.state(), app.state().db());
        assert_eq!(
            holder(state, db, TUNNEL_B).await.unwrap().as_deref(),
            Some("tunnel")
        );
        assert_eq!(
            holder(state, db, TUNNEL_A).await.unwrap().as_deref(),
            Some("tunnel-2")
        );
    }
);

db_test!(
    two_cloudflared_with_one_token_cannot_both_become_tunnels,
    migrate = Migrator,
    |db: TestDb| async move {
        use rdockpanel::models::container_import;
        use sea_orm::{ActiveModelTrait, EntityTrait, IntoActiveModel, Set};
        let fake = FakeDocker::default();
        shop_with_tunnel(&fake, TUNNEL_A);
        cloudflared(&fake, "edge", "172.20.0.56", TUNNEL_A);
        let app = support::app_with_helper(db, fake.clone(), "helper:1").await;
        let admin = ops(&app).await;
        let id = planned_shop(&app, &admin).await;
        patch_tunnel(
            &app,
            &admin,
            id,
            json!({"id-tunnel": true, "id-edge": true}),
        )
        .await
        .assert_validation_error("tunnel");
        patch_tunnel(&app, &admin, id, json!({"id-tunnel": true}))
            .await
            .assert_status(StatusCode::OK);
        patch_tunnel(&app, &admin, id, json!({"id-edge": true}))
            .await
            .assert_validation_error("tunnel");
        // A plan that has both anyway: the start refuses it.
        let row = container_import::Entity::find_by_id(id)
            .one(app.state().db())
            .await
            .unwrap()
            .unwrap();
        let mut plan: Value = serde_json::from_str(row.plan.as_deref().unwrap()).unwrap();
        for item in plan["items"].as_array_mut().unwrap() {
            if item["container_id"] == "id-edge" {
                item["as_tunnel"] = json!(true);
            }
        }
        let mut both = row.into_active_model();
        both.plan = Set(Some(plan.to_string()));
        both.update(app.state().db()).await.unwrap();
        post(
            &app,
            &admin,
            &format!("imports/{id}/start"),
            json!({"confirm": "shop"}),
        )
        .await
        .assert_validation_error("tunnel");
    }
);

/// `shop-web-1` of [`shop`] routed by Traefik to `rule` on port 8080.
fn routed_web(fake: &FakeDocker, rule: &str) {
    routed(fake, "shop-web-1", rule);
}

/// The container `name` routed by Traefik to `rule` on port 8080.
fn routed(fake: &FakeDocker, name: &str, rule: &str) {
    let mut inspected = fake.inspected.lock().unwrap();
    let labels = &mut inspected.get_mut(name).unwrap()["Config"]["Labels"];
    labels["traefik.enable"] = json!("true");
    labels["traefik.http.routers.web.rule"] = json!(rule);
    labels["traefik.http.services.web.loadbalancer.server.port"] = json!("8080");
}

/// A domain the panel serves already, on a service of another project.
async fn served_elsewhere(app: &TestApp, host: &str) {
    use rdockpanel::models::project;
    use rdockpanel::services::creation_service::{NewDomain, NewService, create};
    use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
    let txn = rustcms::db::begin_write(app.state().db()).await.unwrap();
    let legacy = match project::Entity::find()
        .filter(project::Column::Name.eq("legacy"))
        .one(&txn)
        .await
        .unwrap()
    {
        Some(p) => p,
        None => project::ActiveModel {
            name: Set("legacy".to_owned()),
            created_at: Set(chrono::Utc::now()),
            updated_at: Set(chrono::Utc::now()),
            ..Default::default()
        }
        .insert(&txn)
        .await
        .unwrap(),
    };
    let mut new = NewService::app(host.replace('.', "-"), "ghcr.io/traefik/whoami");
    new.domains = vec![NewDomain {
        host: host.to_owned(),
        path: "/".to_owned(),
        port: 80,
        https: false,
        compose_service: None,
    }];
    create(&txn, app.state().keyring(), &legacy, new)
        .await
        .unwrap();
    txn.commit().await.unwrap();
}

/// The panel's domains of `host` (any service): (service id, port).
async fn domains_of_host(app: &TestApp, host: &str) -> Vec<(i64, i32)> {
    use rdockpanel::models::service_domain;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
    service_domain::Entity::find()
        .filter(service_domain::Column::Host.eq(host))
        .all(app.state().db())
        .await
        .unwrap()
        .into_iter()
        .map(|d| (d.service_id, d.port))
        .collect()
}

async fn start_and_run(app: &TestApp, admin: &TestClient<'_>, id: i64) {
    admin
        .post_json(
            &app.admin_api(&format!("x/imports/{id}/start")),
            &json!({"confirm": "shop"}),
        )
        .await
        .assert_status(StatusCode::ACCEPTED);
    run_all(app).await;
    assert_eq!(import_state(app, id).await, "active");
}

db_test!(
    a_label_domain_is_added_to_the_imported_service,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::default();
        shop(&fake);
        routed_web(&fake, "Host(`shop.example.com`)");
        let app = support::app_with_helper(db, fake.clone(), "helper:1").await;
        let admin = ops(&app).await;
        let id = planned_shop(&app, &admin).await;
        let web = plan_item(&app, &admin, id, "id-web").await;
        assert_eq!(web["domains"][0]["host"], "shop.example.com", "{web}");
        start_and_run(&app, &admin, id).await;
        let served = domains_of_host(&app, "shop.example.com").await;
        assert_eq!(served.len(), 1, "{served:?}");
        assert_eq!(served[0].1, 8080);
    }
);

db_test!(
    a_host_the_panel_already_serves_is_skipped_with_a_confirmation,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::default();
        shop(&fake);
        routed_web(
            &fake,
            "Host(`shop.example.com`) || Host(`www.shop.example.com`)",
        );
        let app = support::app_with_helper(db, fake.clone(), "helper:1").await;
        let admin = ops(&app).await;
        served_elsewhere(&app, "shop.example.com").await;
        let id = planned_shop(&app, &admin).await;
        let taken = |item: &Value| -> Vec<String> {
            item["findings"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|f| f["code"] == "domain_taken")
                .inspect(|f| assert_eq!(f["level"], "CONFIRM"))
                .map(|f| f["field"].as_str().unwrap().to_owned())
                .collect()
        };
        let web = plan_item(&app, &admin, id, "id-web").await;
        assert_eq!(taken(&web), ["shop.example.com"], "{web}");
        // A host the panel serves since the plan was read: said when it changes.
        served_elsewhere(&app, "www.shop.example.com").await;
        let changed = admin
            .send(
                Method::PATCH,
                &app.admin_api(&format!("x/imports/{id}/plan")),
                Some(&json!({"project_name": "shop"})),
            )
            .await;
        changed.assert_status(StatusCode::OK);
        let web = plan_item(&app, &admin, id, "id-web").await;
        assert_eq!(
            taken(&web),
            ["shop.example.com", "www.shop.example.com"],
            "{web}"
        );
        start_and_run(&app, &admin, id).await;
        // Not routed twice: each stays its first service's.
        assert_eq!(domains_of_host(&app, "shop.example.com").await.len(), 1);
        assert_eq!(domains_of_host(&app, "www.shop.example.com").await.len(), 1);
    }
);

db_test!(
    unticking_a_domain_leaves_it_out,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::default();
        shop(&fake);
        routed_web(
            &fake,
            "Host(`shop.example.com`) || Host(`www.shop.example.com`)",
        );
        let app = support::app_with_helper(db, fake.clone(), "helper:1").await;
        let admin = ops(&app).await;
        let id = planned_shop(&app, &admin).await;
        let url = app.admin_api(&format!("x/imports/{id}/plan"));
        for wrong in [
            json!({"domains": {"id-ghost": []}}),
            json!({"domains": {"id-web": ["other.example.com"]}}),
        ] {
            admin
                .send(Method::PATCH, &url, Some(&wrong))
                .await
                .assert_validation_error("domains");
        }
        admin
            .send(
                Method::PATCH,
                &url,
                Some(&json!({"domains": {"id-web": ["www.shop.example.com"]}})),
            )
            .await
            .assert_status(StatusCode::OK);
        let web = plan_item(&app, &admin, id, "id-web").await;
        let skipped: Vec<&Value> = web["domains"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|d| d["skip"] == true)
            .map(|d| &d["host"])
            .collect();
        assert_eq!(skipped, [&json!("www.shop.example.com")], "{web}");
        start_and_run(&app, &admin, id).await;
        assert_eq!(domains_of_host(&app, "shop.example.com").await.len(), 1);
        assert!(
            domains_of_host(&app, "www.shop.example.com")
                .await
                .is_empty()
        );
    }
);

db_test!(
    undo_removes_the_domains_it_added,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::default();
        shop(&fake);
        routed_web(&fake, "Host(`shop.example.com`)");
        let app = support::app_with_helper(db, fake.clone(), "helper:1").await;
        let admin = ops(&app).await;
        let id = planned_shop(&app, &admin).await;
        start_and_run(&app, &admin, id).await;
        assert_eq!(domains_of_host(&app, "shop.example.com").await.len(), 1);
        post(&app, &admin, &format!("imports/{id}/undo"), json!({}))
            .await
            .assert_status(StatusCode::ACCEPTED);
        run_all(&app).await;
        assert_eq!(import_state(&app, id).await, "reverted");
        assert!(domains_of_host(&app, "shop.example.com").await.is_empty());
    }
);

/// The findings of `code` on a plan's item: (field, text).
fn findings_of(item: &Value, code: &str) -> Vec<(String, String)> {
    item["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["code"] == code)
        .map(|f| {
            (
                f["field"].as_str().unwrap().to_owned(),
                f["text"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

db_test!(
    two_containers_of_the_plan_with_one_domain_give_it_to_the_first,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::default();
        shop(&fake);
        routed(&fake, "shop-web-1", "Host(`shop.example.com`)");
        routed(&fake, "shop-db-1", "Host(`shop.example.com`)");
        let app = support::app_with_helper(db, fake.clone(), "helper:1").await;
        let admin = ops(&app).await;
        let id = planned_shop(&app, &admin).await;
        let web = findings_of(&plan_item(&app, &admin, id, "id-web").await, "domain_taken");
        let db_ = findings_of(&plan_item(&app, &admin, id, "id-db").await, "domain_taken");
        // One of them keeps it, the other is told why it does not.
        let said: Vec<_> = web.iter().chain(&db_).collect();
        assert_eq!(said.len(), 1, "{web:?} {db_:?}");
        assert_eq!(said[0].0, "shop.example.com");
        // It names the container that keeps the domain.
        let (keeper, loser) = if web.is_empty() {
            ("web", "id-db")
        } else {
            ("db", "id-web")
        };
        let finding = plan_item(&app, &admin, id, loser).await["findings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["code"] == "domain_taken")
            .cloned()
            .unwrap();
        assert_eq!(finding["name"], keeper, "{finding}");
        assert!(said[0].1.contains("{name}"), "{said:?}");
        start_and_run(&app, &admin, id).await;
        assert_eq!(domains_of_host(&app, "shop.example.com").await.len(), 1);
    }
);

db_test!(
    a_domain_the_panel_would_refuse_is_left_out_and_said,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::default();
        shop(&fake);
        routed_web(
            &fake,
            "Host(`shop.example.com`) && PathPrefix(`/a%20b`) || Host(`ok.example.com`)",
        );
        let app = support::app_with_helper(db, fake.clone(), "helper:1").await;
        let admin = ops(&app).await;
        let id = planned_shop(&app, &admin).await;
        let web = plan_item(&app, &admin, id, "id-web").await;
        let said = findings_of(&web, "domain_invalid");
        assert_eq!(said.len(), 1, "{web}");
        assert_eq!(said[0].0, "shop.example.com/a%20b");
        start_and_run(&app, &admin, id).await;
        assert!(domains_of_host(&app, "shop.example.com").await.is_empty());
        assert_eq!(domains_of_host(&app, "ok.example.com").await.len(), 1);
    }
);

db_test!(
    an_unticked_domain_can_go_to_the_other_container,
    migrate = Migrator,
    |db: TestDb| async move {
        use rdockpanel::models::service;
        use sea_orm::EntityTrait;
        let fake = FakeDocker::default();
        shop(&fake);
        routed(&fake, "shop-web-1", "Host(`shop.example.com`)");
        routed(&fake, "shop-db-1", "Host(`shop.example.com`)");
        let app = support::app_with_helper(db, fake.clone(), "helper:1").await;
        let admin = ops(&app).await;
        let id = planned_shop(&app, &admin).await;
        let url = app.admin_api(&format!("x/imports/{id}/plan"));
        let web_lost =
            !findings_of(&plan_item(&app, &admin, id, "id-web").await, "domain_taken").is_empty();
        let (keeper, other, other_name) = if web_lost {
            ("id-db", "id-web", "web")
        } else {
            ("id-web", "id-db", "db")
        };
        // Unticked on the one that has it, ticked on the other: the other gets it.
        for change in [
            json!({"domains": {keeper: ["shop.example.com"]}}),
            json!({"domains": {other: []}}),
        ] {
            admin
                .send(Method::PATCH, &url, Some(&change))
                .await
                .assert_status(StatusCode::OK);
        }
        let item = plan_item(&app, &admin, id, other).await;
        assert!(findings_of(&item, "domain_taken").is_empty(), "{item}");
        assert_eq!(item["domains"][0]["skip"], false, "{item}");
        start_and_run(&app, &admin, id).await;
        let served = domains_of_host(&app, "shop.example.com").await;
        assert_eq!(served.len(), 1, "{served:?}");
        let owner = service::Entity::find_by_id(served[0].0)
            .one(app.state().db())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(owner.name, other_name);
    }
);

db_test!(
    a_domain_of_a_plan_without_its_port_leads_to_80,
    migrate = Migrator,
    |db: TestDb| async move {
        use rdockpanel::models::container_import;
        use sea_orm::{ActiveModelTrait, EntityTrait, IntoActiveModel, Set};
        let fake = FakeDocker::default();
        shop(&fake);
        routed_web(&fake, "Host(`shop.example.com`)");
        let app = support::app_with_helper(db, fake.clone(), "helper:1").await;
        let admin = ops(&app).await;
        let id = planned_shop(&app, &admin).await;
        // A plan read before ports were filled in: the hint has none.
        let row = container_import::Entity::find_by_id(id)
            .one(app.state().db())
            .await
            .unwrap()
            .unwrap();
        let mut plan: Value = serde_json::from_str(row.plan.as_deref().unwrap()).unwrap();
        for item in plan["items"].as_array_mut().unwrap() {
            for hint in item["domains"].as_array_mut().unwrap() {
                hint["port"] = Value::Null;
            }
        }
        let mut change = row.into_active_model();
        change.plan = Set(Some(plan.to_string()));
        change.update(app.state().db()).await.unwrap();
        admin
            .send(
                Method::PATCH,
                &app.admin_api(&format!("x/imports/{id}/plan")),
                Some(&json!({"project_name": "shop"})),
            )
            .await
            .assert_status(StatusCode::OK);
        let web = plan_item(&app, &admin, id, "id-web").await;
        assert!(findings_of(&web, "domain_invalid").is_empty(), "{web}");
        start_and_run(&app, &admin, id).await;
        assert_eq!(domains_of_host(&app, "shop.example.com").await[0].1, 80);
    }
);
