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
    LOCK.lock().await
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
