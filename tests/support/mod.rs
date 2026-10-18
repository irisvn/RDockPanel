//! Helpers shared by the test suites.

#![allow(dead_code)]

pub mod fake_docker;

use rdockpanel::bootstrap;
use rustcms_testing::{TestApp, TestDb};

/// The application on `db` (migrated by `db_test!`).
pub async fn app(db: TestDb) -> TestApp {
    TestApp::new(bootstrap::build, db).await
}

/// The application on `db` with `docker` as its Docker engine.
pub async fn app_with_docker(db: TestDb, docker: fake_docker::FakeDocker) -> TestApp {
    use rdockpanel::services::docker_service::Docker;
    TestApp::new(
        move |config| {
            let mut app = bootstrap::build(config);
            app.bind(Docker::new(docker));
            app
        },
        db,
    )
    .await
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
