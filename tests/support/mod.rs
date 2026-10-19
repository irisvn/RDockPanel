//! Helpers shared by the test suites.

#![allow(dead_code)]

pub mod fake_docker;

use rdockpanel::bootstrap;
use rustcms_testing::{TestApp, TestDb};

/// The application on `db` (migrated by `db_test!`), its Traefik routes in
/// a directory of its own.
pub async fn app(db: TestDb) -> TestApp {
    let traefik = scratch_traefik();
    TestApp::new(
        move |config| {
            let mut app = bootstrap::build(config);
            app.bind(traefik);
            app
        },
        db,
    )
    .await
}

/// Traefik's routes in a new temporary directory (never the development
/// one: an app's boot rewrites its directory from its own database).
pub fn scratch_traefik() -> rdockpanel::services::traefik_service::Traefik {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("rdp-routes-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    rdockpanel::services::traefik_service::Traefik::new(dir, "rdp-traefik", true)
}

/// The application on `db` with `docker` as its Docker engine, and Traefik's
/// routes in a directory of its own ([`traefik_dir`]).
/// The engine runs Traefik (`rdp-traefik`, labelled, on one network), as
/// `scripts/dev-swarm.sh` does.
pub async fn app_with_docker(db: TestDb, docker: fake_docker::FakeDocker) -> TestApp {
    let spec = serde_json::json!({
        "Name": "rdp-traefik",
        "Labels": {"rdockpanel.system": "true"},
        "TaskTemplate": {
            "ContainerSpec": {"Image": "traefik:v3.5", "Args": ["--providers.file.directory=/etc/traefik/dynamic"]},
            "Networks": [{"Target": "ingress-extra"}],
        },
    });
    docker.services.lock().unwrap().insert(
        "rdp-traefik".to_owned(),
        rdockpanel::services::docker_service::SwarmService {
            spec: Some(serde_json::from_value(spec).unwrap()),
            ..Default::default()
        },
    );
    app_with(db, docker, scratch_traefik()).await
}

/// The application with `docker` and `traefik`.
pub async fn app_with(
    db: TestDb,
    docker: impl rdockpanel::services::docker_service::DockerApi + 'static,
    traefik: rdockpanel::services::traefik_service::Traefik,
) -> TestApp {
    use rdockpanel::services::docker_service::Docker;
    TestApp::new(
        move |config| {
            let mut app = bootstrap::build(config);
            app.bind(Docker::new(docker));
            app.bind(traefik);
            app
        },
        db,
    )
    .await
}

/// Held by the tests that change a real Swarm's networks: Swarm's address
/// allocator rejects tasks ("Pool overlaps") while networks come and go in
/// parallel.
pub async fn swarm_lock() -> tokio::sync::MutexGuard<'static, ()> {
    static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    static SWEPT: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();
    let guard = LOCK.lock().await;
    SWEPT
        .get_or_init(|| async {
            let _ = tokio::task::spawn_blocking(sweep_dead_runs).await;
        })
        .await;
    guard
}

/// The Swarm objects of test runs that died before their cleanup (killed,
/// crashed). Each run that takes `swarm_lock` leaves a marker named after its
/// process; the objects of a marked process no longer running are removed
/// (their names carry its pid: `e<pid>`, `d<pid>`, `db<pid>`, `t<pid>`,
/// `r<pid>a`, `r<pid>b`), then its marker. Nothing unmarked is touched.
fn sweep_dead_runs() {
    use std::process::Command;
    let runs = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("swarm-runs");
    let _ = std::fs::create_dir_all(&runs);
    // Dead only when the system says there is no such process.
    let dead = |pid: u32| {
        Command::new("kill")
            .args(["-0", &pid.to_string()])
            .output()
            .is_ok_and(|o| {
                !o.status.success()
                    && String::from_utf8_lossy(&o.stderr).contains("No such process")
            })
    };
    let mut gone: Vec<(std::path::PathBuf, u32)> = vec![];
    for entry in std::fs::read_dir(&runs).into_iter().flatten().flatten() {
        if let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>()
            && pid != std::process::id()
            && dead(pid)
        {
            gone.push((entry.path(), pid));
        }
    }
    let marker = runs.join(std::process::id().to_string());
    let _ = std::fs::write(&marker, "");
    if gone.is_empty() {
        return;
    }
    let owned = |name: &str| {
        let name = name.strip_prefix("rdp_").unwrap_or(name);
        gone.iter().any(|(_, pid)| {
            [
                format!("e{pid}"),
                format!("d{pid}"),
                format!("db{pid}"),
                format!("t{pid}"),
                format!("r{pid}a"),
                format!("r{pid}b"),
            ]
            .iter()
            .any(|p| {
                name == p
                    || name
                        .strip_prefix(p.as_str())
                        .is_some_and(|r| r.starts_with('_'))
            })
        })
    };
    let docker = |args: &[&str]| Command::new("docker").args(args).output().ok();
    // A listing that failed keeps the markers: nothing was cleaned up.
    let listed = std::cell::Cell::new(true);
    let list = |args: &[&str]| -> Vec<String> {
        let out = docker(args).filter(|o| o.status.success());
        listed.set(listed.get() && out.is_some());
        out.map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter(|n| owned(n))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
    };
    // Removed, or already gone.
    let removed = |args: &[&str]| {
        docker(args).is_some_and(|o| {
            let err = String::from_utf8_lossy(&o.stderr);
            o.status.success() || err.contains("No such") || err.contains("not found")
        })
    };
    for s in list(&["service", "ls", "--format", "{{.Name}}"]) {
        removed(&["service", "rm", &s]);
    }
    // Routes of the development Traefik (the routing test writes there).
    let routes = concat!(env!("CARGO_MANIFEST_DIR"), "/storage/traefik/dynamic");
    for entry in std::fs::read_dir(routes).into_iter().flatten().flatten() {
        let file = entry.file_name().to_string_lossy().into_owned();
        if file.strip_suffix(".yml").is_some_and(owned) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    let mut networks = list(&["network", "ls", "--format", "{{.Name}}"]);
    // Traefik leaves the networks it is on (without waiting for it).
    let traefik = docker(&[
        "service",
        "inspect",
        "rdp-traefik",
        "--format",
        "{{json .Spec.TaskTemplate.Networks}}",
    ])
    .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
    .unwrap_or_default();
    for n in &networks {
        let id = docker(&["network", "inspect", "--format", "{{.Id}}", n])
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
            .unwrap_or_default();
        if !id.is_empty() && traefik.contains(&id) {
            removed(&[
                "service",
                "update",
                "--detach",
                "--network-rm",
                n,
                "rdp-traefik",
            ]);
        }
    }
    let mut volumes = list(&["volume", "ls", "-q"]);
    // Once their containers are gone.
    for _ in 0..30 {
        networks.retain(|n| !removed(&["network", "rm", n]));
        volumes.retain(|v| !removed(&["volume", "rm", v]));
        if networks.is_empty() && volumes.is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    if listed.get() && networks.is_empty() && volumes.is_empty() {
        for (path, _) in &gone {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Where the app writes Traefik's routes.
pub fn traefik_dir(app: &TestApp) -> std::path::PathBuf {
    app.state()
        .service::<rdockpanel::services::traefik_service::Traefik>()
        .expect("traefik")
        .dir
        .clone()
}

/// A user holding `permissions` (admin guard) through a role of that name.
pub async fn user_with(
    app: &TestApp,
    name: &str,
    permissions: &[&str],
) -> rustcms::models::user::Model {
    use rustcms::auth::{ADMIN_GUARD, Actor, AuthState, RbacService};
    use rustcms_testing::{Factory, UserFactory};
    let auth = app.state().service::<AuthState>().expect("auth");
    let rbac = RbacService::new(app.state().db(), &auth.resolver);
    rbac.sync_permissions(&auth.permissions.all())
        .await
        .unwrap();
    let user = UserFactory::new()
        .name(name)
        .email(format!("{name}@example.test"))
        .create(app.state().db())
        .await;
    if !permissions.is_empty() {
        let permissions: Vec<String> = permissions.iter().map(|p| (*p).to_owned()).collect();
        let role = rbac
            .create_role(Actor::System, ADMIN_GUARD, name, &permissions)
            .await
            .unwrap();
        rbac.assign_role(Actor::System, user.id, role.id)
            .await
            .unwrap();
    }
    user
}
