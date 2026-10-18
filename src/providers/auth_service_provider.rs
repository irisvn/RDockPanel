//! Authentication and permissions: admin sessions under `/admin`, API tokens
//! elsewhere. Declare permissions beyond each resource's own here.

use rustcms::prelude::*;

pub fn provider(api: &ApiRoutes, admin: &AdminRoutes) -> AuthProvider {
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
        // Reading environment values in clear (audited); writing needs `services.update`.
        .permission(
            rustcms::auth::ADMIN_GUARD,
            crate::http::controllers::apps_controller::SECRETS_PERMISSION,
            "services",
        )
}
