//! With `PANEL_TEST_SWARM=1`: database services on a real Swarm, reached by
//! an app of their project through their internal host and generated
//! credentials (`PANEL_TEST_ALL_DATABASES=1` adds MySQL, MariaDB, MongoDB).

use std::process::Command;
use std::time::Duration;

use rdockpanel::database::migrations::Migrator;
use rdockpanel::services::deployment_service::{self, Timings};
use rdockpanel::services::docker_service::{BollardDocker, Docker};
use rustcms::axum::http::StatusCode;
use rustcms_testing::{TestApp, TestClient, TestDb, db_test};
use serde_json::{Value, json};

use crate::deployments::{action, create, last_deployment};
use crate::support;

const REAL: Timings = Timings {
    poll: Duration::from_secs(1),
    settle: Duration::from_secs(4),
    timeout: Duration::from_secs(300),
};

fn docker_cli(args: &[&str]) -> std::process::Output {
    Command::new("docker").args(args).output().expect("docker")
}

/// Runs `script` (sh) in a container of the Swarm service `name`.
fn exec(name: &str, script: &str) -> std::process::Output {
    let ps = docker_cli(&[
        "ps",
        "-q",
        "--filter",
        &format!("label=com.docker.swarm.service.name={name}"),
    ]);
    let container = String::from_utf8_lossy(&ps.stdout)
        .lines()
        .next()
        .unwrap_or_default()
        .to_owned();
    assert!(!container.is_empty(), "no container of {name}");
    docker_cli(&["exec", &container, "sh", "-c", script])
}

async fn deploy(app: &TestApp, admin: &TestClient<'_>, id: i64) {
    action(app, admin, "services", "deploy", &id.to_string())
        .await
        .assert_status(StatusCode::OK);
    let queued = last_deployment(app, id).await;
    deployment_service::run(app.state(), queued.id, REAL)
        .await
        .unwrap();
    let done = last_deployment(app, id).await;
    assert_eq!(
        done.status,
        "succeeded",
        "{:?}\n{}",
        done.error,
        done.log.unwrap_or_default()
    );
}

async fn new_service(
    app: &TestApp,
    admin: &TestClient<'_>,
    project: &str,
    body: Value,
) -> (i64, Value) {
    let made = admin
        .post_json(
            &app.admin_api(&format!("x/apps/projects/{project}/services")),
            &body,
        )
        .await;
    made.assert_status(StatusCode::OK);
    let id = made.json()["id"].as_i64().unwrap();
    let page = admin
        .get(&app.admin_api(&format!("x/apps/services/{id}")))
        .await
        .json();
    let mut facts = page["database"].clone();
    if !facts.is_null() {
        let secret = admin
            .get(&app.admin_api(&format!("x/apps/services/{id}/credentials")))
            .await
            .json();
        facts["password"] = secret["password"].clone();
    }
    (id, facts)
}

db_test!(
    databases_on_a_real_swarm,
    migrate = Migrator,
    |db: TestDb| async move {
        if std::env::var_os("PANEL_TEST_SWARM").is_none() {
            return;
        }
        let _swarm = support::swarm_lock().await;
        let name = format!("db{}", support::run_id(&db));
        let app = support::app_with(
            db,
            BollardDocker::connect().expect("docker client"),
            support::scratch_traefik(),
        )
        .await;
        let user = support::user_with(&app, "admin", &[]).await;
        app.make_super_admin(&user).await;
        let admin = app.acting_as(&user).await;
        let project = create(&app, &admin, "projects", json!({"name": name})).await;

        let mut engines = vec![("postgres", "postgres:17"), ("redis", "redis:7")];
        if std::env::var_os("PANEL_TEST_ALL_DATABASES").is_some() {
            engines.extend([
                ("mysql", "mysql:9"),
                ("mariadb", "mariadb:11"),
                ("mongo", "mongo:8"),
            ]);
        }
        let mut made = vec![];
        let checks = std::panic::AssertUnwindSafe(async {
            for (engine, image) in &engines {
                let (id, facts) = new_service(
                    &app,
                    &admin,
                    &project,
                    json!({"name": engine, "kind": engine}),
                )
                .await;
                made.push(id);
                deploy(&app, &admin, id).await;
                // A client of the same project (the engine's image, idle).
                let client = format!("{engine}-client");
                let (cid, _) = new_service(
                    &app,
                    &admin,
                    &project,
                    json!({"name": client, "kind": "app", "image": image}),
                )
                .await;
                made.push(cid);
                admin
                    .send(
                        rustcms::axum::http::Method::PATCH,
                        &app.admin_api(&format!("resources/services/records/{cid}")),
                        Some(&json!({"command": "sh -c 'sleep 3600'", "zero_downtime": false})),
                    )
                    .await
                    .assert_status(StatusCode::OK);
                deploy(&app, &admin, cid).await;
                let host = facts["host"].as_str().unwrap();
                let password = facts["password"].as_str().unwrap();
                let user = facts["user"].as_str().unwrap();
                let database = facts["database"].as_str().unwrap_or_default();
                let script = match *engine {
                    "postgres" => format!(
                        "PGPASSWORD='{password}' psql -h {host} -U {user} -d {database} -tAc 'select 1'"
                    ),
                    "redis" => {
                        format!("redis-cli -h {host} -a '{password}' --no-auth-warning ping")
                    }
                    "mysql" => format!(
                        "mysql -h {host} -u {user} -p'{password}' {database} -N -e 'select 1'"
                    ),
                    "mariadb" => format!(
                        "mariadb -h {host} -u {user} -p'{password}' {database} -N -e 'select 1'"
                    ),
                    _ => format!(
                        "mongosh --quiet 'mongodb://{user}:{password}@{host}:27017/?authSource=admin' --eval 'db.adminCommand({{ping:1}}).ok'"
                    ),
                };
                let mut answer = String::new();
                let mut ok = false;
                for _ in 0..30 {
                    let out = exec(
                        &format!("{project_name}_{client}", project_name = name),
                        &script,
                    );
                    answer = format!(
                        "{}{}",
                        String::from_utf8_lossy(&out.stdout),
                        String::from_utf8_lossy(&out.stderr)
                    );
                    if out.status.success() {
                        ok = true;
                        break;
                    }
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
                // The client succeeded, and said what the engine answers.
                let expected = if *engine == "redis" { "PONG" } else { "1" };
                assert!(
                    ok && answer.lines().any(|l| l.trim() == expected),
                    "{engine}: {answer}"
                );
            }
        });
        let outcome = futures_util::FutureExt::catch_unwind(checks).await;
        let docker = app.state().service::<Docker>().unwrap();
        for id in made {
            action(&app, &admin, "services", "destroy", &id.to_string()).await;
            let destroy = last_deployment(&app, id).await;
            let _ = deployment_service::run(app.state(), destroy.id, REAL).await;
        }
        for _ in 0..30 {
            if docker
                .engine()
                .remove_network(&format!("rdp_{name}"))
                .await
                .is_ok()
            {
                break;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        // Its volumes (Destroy keeps them): once their containers are gone.
        for (engine, _) in &engines {
            for _ in 0..30 {
                if docker
                    .engine()
                    .remove_volume(&format!("rdp_{name}_{engine}_data"))
                    .await
                    .is_ok()
                {
                    break;
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
        if let Err(panic) = outcome {
            std::panic::resume_unwind(panic);
        }
    }
);
