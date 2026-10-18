//! With `PANEL_TEST_SWARM=1` and `scripts/dev-swarm.sh`'s Traefik: services
//! reached through their domains, an image change under load without a
//! failed request, HTTPS with its redirect, and projects that cannot reach
//! each other.

use std::process::Command;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use rdockpanel::database::migrations::Migrator;
use rdockpanel::models::deployment;
use rdockpanel::services::deployment_service::{self, Timings};
use rdockpanel::services::docker_service::{BollardDocker, Docker};
use rdockpanel::services::traefik_service::Traefik;
use rustcms::axum::http::StatusCode;
use rustcms_testing::{TestApp, TestClient, TestDb, db_test};
use sea_orm::EntityTrait;
use serde_json::json;

use crate::deployments::{action, create, last_deployment, records};
use crate::support;

const HTTP: &str = "127.0.0.1:8080";

const REAL: Timings = Timings {
    poll: Duration::from_secs(1),
    settle: Duration::from_secs(4),
    timeout: Duration::from_secs(180),
};

/// `curl`'s status code for `url` (`000`: no answer).
fn curl(args: &[&str]) -> String {
    let out = Command::new("curl")
        .args([
            "-s",
            "-o",
            "/dev/null",
            "-w",
            "%{http_code}",
            "--max-time",
            "3",
        ])
        .args(args)
        .output()
        .expect("curl");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn get(host: &str) -> String {
    curl(&["-H", &format!("Host: {host}"), &format!("http://{HTTP}/")])
}

fn docker_cli(args: &[&str]) -> std::process::Output {
    Command::new("docker").args(args).output().expect("docker")
}

/// Waits until `host` answers 200 through Traefik (it restarts when it
/// joins a network, and reads new files within a second or two).
async fn until_routed(host: &str) {
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(90) {
        if get(host) == "200" {
            return;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    panic!("{host} is not routed: {}", get(host));
}

/// Runs action `trigger` on service `id`, then its deployment; it succeeded.
async fn step(app: &TestApp, admin: &TestClient<'_>, trigger: &str, id: i64) -> deployment::Model {
    action(app, admin, "services", trigger, &id.to_string())
        .await
        .assert_status(StatusCode::OK);
    let queued = last_deployment(app, id).await;
    deployment_service::run(app.state(), queued.id, REAL)
        .await
        .unwrap();
    let done = deployment::Entity::find_by_id(queued.id)
        .one(app.state().db())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        done.status,
        "succeeded",
        "{trigger}: {:?}\n{}",
        done.error,
        done.log.unwrap_or_default()
    );
    done
}

/// A project with an nginx service `web` (health-checked, zero downtime)
/// and `domains`: the service's id.
async fn nginx(
    app: &TestApp,
    admin: &TestClient<'_>,
    project: &str,
    domains: &[(&str, bool)],
) -> i64 {
    let project = create(app, admin, "projects", json!({"name": project})).await;
    let service = create(
        app,
        admin,
        "services",
        json!({
            "project_id": project, "name": "web", "image": "nginx:1.27-alpine", "replicas": 2,
            "zero_downtime": true, "healthcheck_cmd": "wget -qO- http://127.0.0.1/ >/dev/null",
            "healthcheck_interval_s": 1,
        }),
    )
    .await;
    for (host, https) in domains {
        create(
            app,
            admin,
            "service_domains",
            json!({"service_id": service, "host": host, "path": "/", "port": 80, "https": https}),
        )
        .await;
    }
    service.parse().unwrap()
}

db_test!(
    domains_route_through_traefik,
    migrate = Migrator,
    |db: TestDb| async move {
        if std::env::var_os("PANEL_TEST_SWARM").is_none() {
            return;
        }
        let _swarm = support::swarm_lock().await;
        if !docker_cli(&["service", "inspect", "rdp-traefik"])
            .status
            .success()
        {
            panic!("PANEL_TEST_SWARM needs Traefik: run scripts/dev-swarm.sh");
        }
        // The directory Traefik reads (shared: other files are left alone).
        let traefik = Traefik::new(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("storage/traefik/dynamic"),
            "rdp-traefik",
            false,
        );
        let app = support::app_with(
            db,
            BollardDocker::connect().expect("docker client"),
            traefik,
        )
        .await;
        let user = support::user_with(&app, "admin", &[]).await;
        app.make_super_admin(&user).await;
        let admin = app.acting_as(&user).await;

        let tag = format!("r{}", std::process::id());
        let (a, b) = (format!("{tag}a"), format!("{tag}b"));
        let (host, secure) = (format!("{a}.localhost"), format!("s{a}.localhost"));
        let a_id = nginx(&app, &admin, &a, &[(&host, false), (&secure, true)]).await;
        let b_id = nginx(&app, &admin, &b, &[]).await;

        let checks = std::panic::AssertUnwindSafe(async {
            step(&app, &admin, "deploy", a_id).await;
            step(&app, &admin, "deploy", b_id).await;
            until_routed(&host).await;

            // HTTPS (Traefik's own certificate), and HTTP redirected to it.
            let resolve = format!("{secure}:8443:127.0.0.1");
            let https = curl(&[
                "-k",
                "--resolve",
                &resolve,
                &format!("https://{secure}:8443/"),
            ]);
            assert_eq!(https, "200");
            let redirect = curl(&["-H", &format!("Host: {secure}"), &format!("http://{HTTP}/")]);
            assert!(matches!(redirect.as_str(), "301" | "308"), "{redirect}");

            // Another image under load: no request fails.
            let stop = Arc::new(AtomicBool::new(false));
            let (ok, failed) = (
                Arc::new(AtomicUsize::new(0)),
                Arc::new(Mutex::new(Vec::<String>::new())),
            );
            let load = {
                let (stop, ok, failed, host) =
                    (stop.clone(), ok.clone(), failed.clone(), host.clone());
                std::thread::spawn(move || {
                    while !stop.load(Ordering::Relaxed) {
                        match get(&host).as_str() {
                            "200" => {
                                ok.fetch_add(1, Ordering::Relaxed);
                            }
                            code => failed.lock().unwrap().push(code.to_owned()),
                        }
                        std::thread::sleep(Duration::from_millis(50));
                    }
                })
            };
            let patch = json!({"image": "nginx:1.28-alpine"});
            admin
                .send(
                    rustcms::axum::http::Method::PATCH,
                    &format!("{}/{a_id}", records(&app, "services")),
                    Some(&patch),
                )
                .await
                .assert_status(StatusCode::OK);
            step(&app, &admin, "deploy", a_id).await;
            stop.store(true, Ordering::Relaxed);
            load.join().unwrap();
            let failed = failed.lock().unwrap().clone();
            assert!(
                failed.is_empty(),
                "{} failed of {}: {failed:?}",
                failed.len(),
                ok.load(Ordering::Relaxed) + failed.len()
            );
            assert!(ok.load(Ordering::Relaxed) > 20);

            // Project A reaches its own service, not project B's.
            let container = docker_cli(&[
                "ps",
                "-q",
                "--filter",
                &format!("label=com.docker.swarm.service.name={a}_web"),
            ]);
            let container = String::from_utf8_lossy(&container.stdout)
                .lines()
                .next()
                .expect("a container of A")
                .to_owned();
            let fetch = |target: &str| {
                docker_cli(&[
                    "exec",
                    &container,
                    "wget",
                    "-qO-",
                    "-T",
                    "3",
                    &format!("http://{target}/"),
                ])
                .status
                .success()
            };
            assert!(fetch(&format!("{a}_web")), "A reaches itself");
            assert!(!fetch(&format!("{b}_web")), "A must not reach B");

            // Stopped: not routed any more.
            step(&app, &admin, "stop", a_id).await;
            let started = Instant::now();
            while get(&host) != "404" && started.elapsed() < Duration::from_secs(10) {
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            assert_eq!(get(&host), "404");
        });
        let outcome = futures_util::FutureExt::catch_unwind(checks).await;

        // Cleanup: destroy both, Traefik off their networks, the networks.
        for id in [a_id, b_id] {
            action(&app, &admin, "services", "destroy", &id.to_string()).await;
            let destroy = last_deployment(&app, id).await;
            let _ = deployment_service::run(app.state(), destroy.id, REAL).await;
        }
        let docker = app.state().service::<Docker>().unwrap();
        for project in [&a, &b] {
            let network = format!("rdp_{project}");
            docker_cli(&[
                "service",
                "update",
                "--detach",
                "--network-rm",
                &network,
                "rdp-traefik",
            ]);
            for _ in 0..30 {
                if docker.engine().remove_network(&network).await.is_ok() {
                    break;
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
        if let Err(panic) = outcome {
            std::panic::resume_unwind(panic);
        }
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("storage/traefik/dynamic");
        assert!(!dir.join(format!("{a}_web.yml")).exists());
    }
);
