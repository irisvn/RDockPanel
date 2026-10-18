//! The panel's own JSON endpoints, on the admin session, under
//! `/admin/api/x` (script pages call them with the session's CSRF token).
//! Every route declares the permission it requires; `system.manage` is
//! checked per object in the handlers (the panel's own containers, …).

use rustcms::prelude::*;

use crate::http::controllers::{
    apps_controller as apps, deploy_controller as deploy, docker_controller as docker,
    server_controller as server,
};

/// Every admin route, with the access it requires.
pub fn routes() -> AdminRoutes {
    let view = || AdminAccess::can("docker.view");
    let manage = || AdminAccess::can("docker.manage");
    AdminRoutes::scope()
        .nest(
            "/docker",
            AdminRoutes::scope()
                .get("/info", view(), docker::info)
                .get("/containers", view(), docker::containers)
                .get("/containers/{id}", view(), docker::container)
                .delete("/containers/{id}", manage(), docker::remove_container)
                .post(
                    "/containers/{id}/{action}",
                    manage(),
                    docker::container_action,
                )
                .get(
                    "/containers/{id}/logs",
                    AdminAccess::can("docker.logs"),
                    docker::logs,
                )
                .events(
                    "/containers/{id}/logs/follow",
                    AdminAccess::can("docker.logs"),
                    docker::follow_logs,
                )
                .events("/containers/{id}/stats", view(), docker::live_stats)
                .socket(
                    "/containers/{id}/exec",
                    AdminAccess::can("docker.exec"),
                    docker::exec,
                )
                .get("/images", view(), docker::images)
                .post("/images/pull", manage(), docker::pull_image)
                .delete("/images/{id}", manage(), docker::remove_image)
                .post("/prune", manage(), docker::prune)
                .get("/actions/{id}", view(), docker::action)
                .get("/volumes", view(), docker::volumes)
                .post("/volumes", manage(), docker::create_volume)
                .delete("/volumes/{name}", manage(), docker::remove_volume)
                .get("/networks", view(), docker::networks)
                .post("/networks", manage(), docker::create_network)
                .delete("/networks/{id}", manage(), docker::remove_network),
        )
        .nest(
            "/server",
            AdminRoutes::scope()
                .get("/info", AdminAccess::can("server.view"), server::info)
                .get("/samples", AdminAccess::can("server.view"), server::samples)
                .events("/live", AdminAccess::can("server.view"), server::live),
        )
        .nest(
            "/apps",
            AdminRoutes::scope()
                .get(
                    "/overview",
                    AdminAccess::can("projects.view_any"),
                    apps::overview,
                )
                .get(
                    "/services/{id}",
                    AdminAccess::can("services.view"),
                    apps::service,
                )
                .get(
                    "/deployments/{id}",
                    AdminAccess::can("deployments.view"),
                    apps::deployment,
                )
                .get(
                    "/services/{id}/env",
                    AdminAccess::can(apps::SECRETS_PERMISSION),
                    apps::env,
                )
                .put(
                    "/services/{id}/env",
                    AdminAccess::can("services.update"),
                    apps::put_env,
                ),
        )
        .nest(
            "/deploy",
            AdminRoutes::scope().events(
                "/deployments/{id}/log",
                AdminAccess::can("deployments.view"),
                deploy::log,
            ),
        )
}
