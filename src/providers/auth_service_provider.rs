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
}
