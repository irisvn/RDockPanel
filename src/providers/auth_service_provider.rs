//! Authentication and permissions: admin sessions under `/admin`, API tokens
//! elsewhere. Declare permissions beyond each resource's own here.

use rustcms::auth::{ADMIN_GUARD, PermissionArea, PermissionMeta, PermissionPreset};
use rustcms::prelude::*;

/// The Roles page's areas for the panel's own permissions (resources keep
/// theirs: the matrix).
const AREAS: &[(&str, &str, i32)] = &[
    ("apps", "Apps and deployments", 10),
    ("docker", "Docker", 20),
    ("server", "Server", 30),
];

/// Each permission beyond the resources' own: its area, label, description.
const META: &[(&str, &str, &str, &str)] = &[
    (
        "apps.access",
        "apps",
        "Open the apps pages",
        "Dashboard, Domains and Actions (a project's members need it)",
    ),
    (
        "services.deploy",
        "apps",
        "Deploy services",
        "Deploy, start, stop, restart and roll back",
    ),
    (
        "services.destroy",
        "apps",
        "Destroy services",
        "Remove a service's Swarm service and its routes",
    ),
    (
        "services.logs",
        "apps",
        "Read service logs",
        "A service's live logs, without Docker's pages",
    ),
    (
        "services.exec",
        "apps",
        "Open service terminals",
        "A shell in a service's running container",
    ),
    (
        "services.secrets",
        "apps",
        "Read environment values",
        "Show environment variables in clear (each read is audited)",
    ),
    (
        "projects.members",
        "apps",
        "Choose project members",
        "Add, change and remove a project's members",
    ),
    (
        "backups.run",
        "apps",
        "Run backups now",
        "Start a database backup by hand",
    ),
    (
        "backups.restore",
        "apps",
        "Restore backups",
        "Replace a database's data with a backup",
    ),
    (
        "docker.view",
        "docker",
        "See Docker",
        "Containers, images, volumes and networks",
    ),
    (
        "docker.manage",
        "docker",
        "Manage Docker",
        "Start, stop, remove and prune Docker objects",
    ),
    (
        "docker.logs",
        "docker",
        "Read container logs",
        "Any container's logs",
    ),
    (
        "docker.exec",
        "docker",
        "Open container terminals",
        "A shell in any container",
    ),
    (
        "server.view",
        "server",
        "See the server",
        "CPU, memory, disk and network, live",
    ),
    (
        "system.manage",
        "system",
        "Manage the panel",
        "Settings, Traefik, certificates, the panel's own containers, reboot",
    ),
];

pub fn provider(api: &ApiRoutes, admin: &AdminRoutes) -> AuthProvider {
    catalog(declared(api, admin))
}

/// How the Roles page shows the panel's permissions: areas, labels, and
/// presets matching a project's roles (applied globally: every project).
fn catalog(mut auth: AuthProvider) -> AuthProvider {
    for (id, label, sort) in AREAS {
        auth = auth.permission_area(id, PermissionArea::new(label).sort(*sort));
    }
    for (name, area, label, description) in META {
        auth = auth.permission_meta(
            ADMIN_GUARD,
            name,
            PermissionMeta::new()
                .label(label)
                .description(description)
                .area(area),
        );
    }
    let presets = [
        (
            "viewer",
            "Apps viewer",
            "Sees every project and its services",
        ),
        (
            "developer",
            "Apps developer",
            "Changes, deploys and runs every project's services",
        ),
        (
            "admin",
            "Apps administrator",
            "Also removes services, restores databases, chooses members",
        ),
    ];
    for (role, label, description) in presets {
        let mut names = vec![crate::services::access_service::APPS_ACCESS];
        names.extend(crate::services::access_service::role_permissions(role));
        auth = auth.permission_preset(
            role,
            PermissionPreset::new(label)
                .description(description)
                .permissions(names),
        );
    }
    auth.permission_preset(
        "monitor",
        PermissionPreset::new("Monitor")
            .description("Sees the server and Docker, changes nothing")
            .permissions(["server.view", "docker.view", "docker.logs"]),
    )
}

fn declared(api: &ApiRoutes, admin: &AdminRoutes) -> AuthProvider {
    // Route permissions come from the routes; `system.manage` (the panel's
    // own Docker objects) is checked inside the Docker handlers.
    AuthProvider::new()
        .api_permissions(api)
        .admin_permissions(admin)
        .permission(
            rustcms::auth::ADMIN_GUARD,
            crate::http::controllers::docker_controller::SYSTEM_PERMISSION,
            "system",
        )
        // Deploying, starting, stopping, restarting and rolling back services; destroying them.
        .permission(rustcms::auth::ADMIN_GUARD, "services.deploy", "services")
        .permission(rustcms::auth::ADMIN_GUARD, "services.destroy", "services")
        // Running a database backup by hand; restoring one (replaces its data).
        .permission(
            rustcms::auth::ADMIN_GUARD,
            crate::http::controllers::backups_controller::RUN_PERMISSION,
            "database_backups",
        )
        .permission(
            rustcms::auth::ADMIN_GUARD,
            crate::http::controllers::backups_controller::RESTORE_PERMISSION,
            "database_backups",
        )
        // A service's logs and its terminal (without Docker's pages); a
        // project's members.
        .permission(rustcms::auth::ADMIN_GUARD, "services.logs", "services")
        .permission(rustcms::auth::ADMIN_GUARD, "services.exec", "services")
        .permission(rustcms::auth::ADMIN_GUARD, "projects.members", "projects")
        // Opening the apps pages (Dashboard, Domains, Actions) and nothing
        // else: given to a project's members (their projects are theirs).
        .permission(
            rustcms::auth::ADMIN_GUARD,
            crate::services::access_service::APPS_ACCESS,
            "projects",
        )
        // Reading environment values in clear (audited); writing needs `services.update`.
        .permission(
            rustcms::auth::ADMIN_GUARD,
            crate::http::controllers::apps_controller::SECRETS_PERMISSION,
            "services",
        )
}
