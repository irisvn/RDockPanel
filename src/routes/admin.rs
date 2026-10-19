//! The panel's own JSON endpoints, on the admin session, under
//! `/admin/api/x` (script pages call them with the session's CSRF token).
//! Every route declares the permission it requires; `system.manage` is
//! checked per object in the handlers (the panel's own containers, …).

use rustcms::prelude::*;

use crate::http::controllers::{
    actions_controller as actions, apps_controller as apps, backups_controller as backups,
    deploy_controller as deploy, docker_controller as docker,
    notifications_controller as notifications, server_controller as server,
    templates_controller as templates, traefik_controller as traefik,
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
                .events(
                    "/containers/{id}/logs/follow/{tail}",
                    AdminAccess::can("docker.logs"),
                    docker::follow_logs_tail,
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
                    "/traefik/resolvers",
                    AdminAccess::can("system.manage"),
                    traefik::resolvers,
                )
                .post(
                    "/traefik/apply",
                    AdminAccess::can("system.manage"),
                    traefik::apply,
                )
                .get(
                    "/certificates",
                    AdminAccess::can("system.manage"),
                    traefik::certificates,
                )
                .post(
                    "/certificates",
                    AdminAccess::can("system.manage"),
                    traefik::upload_certificate,
                )
                .put(
                    "/certificates/{id}",
                    AdminAccess::can("system.manage"),
                    traefik::replace_certificate,
                )
                .delete(
                    "/certificates/{id}",
                    AdminAccess::can("system.manage"),
                    traefik::delete_certificate,
                )
                .get(
                    "/services/{id}",
                    AdminAccess::can("services.view"),
                    apps::service,
                )
                .post(
                    "/projects/{id}/services",
                    AdminAccess::can("services.create"),
                    apps::create_service,
                )
                .get(
                    "/templates",
                    AdminAccess::can("projects.view_any"),
                    templates::list,
                )
                .post(
                    "/templates/refresh",
                    AdminAccess::can("services.create"),
                    templates::refresh,
                )
                .get(
                    "/templates/{slug}",
                    AdminAccess::can("projects.view_any"),
                    templates::show,
                )
                .get(
                    "/templates/{slug}/logo",
                    AdminAccess::can("projects.view_any"),
                    templates::logo,
                )
                .post(
                    "/projects/{id}/templates/{slug}",
                    AdminAccess::can("services.create"),
                    templates::create,
                )
                .post(
                    "/projects/{id}/schema",
                    AdminAccess::can("services.create"),
                    templates::from_schema,
                )
                .get(
                    "/deployments/{id}",
                    AdminAccess::can("deployments.view"),
                    apps::deployment,
                )
                .post(
                    "/services/{id}/deploy-key",
                    AdminAccess::can("services.update"),
                    apps::deploy_key,
                )
                .post(
                    "/services/{id}/trigger",
                    AdminAccess::can("services.deploy"),
                    apps::trigger,
                )
                .get(
                    "/services/{id}/credentials",
                    AdminAccess::can(apps::SECRETS_PERMISSION),
                    apps::credentials,
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
                )
                // Database backups (configurations: the `database_backups` resource).
                .get(
                    "/services/{id}/backups",
                    AdminAccess::can("database_backups.view_any"),
                    backups::index,
                )
                .post(
                    "/services/{id}/restore",
                    AdminAccess::can(backups::RESTORE_PERMISSION),
                    backups::restore,
                )
                .post(
                    "/backups/{id}/run",
                    AdminAccess::can(backups::RUN_PERMISSION),
                    backups::run,
                )
                .get(
                    "/backup-runs/{id}",
                    AdminAccess::can("database_backups.view_any"),
                    backups::show_run,
                )
                .get(
                    "/services/{id}/stats",
                    AdminAccess::can("services.view"),
                    apps::stats,
                )
                .post(
                    "/services/{id}/basic-auth",
                    AdminAccess::can("services.update"),
                    apps::add_basic_auth,
                )
                .delete(
                    "/services/{id}/basic-auth/{user}",
                    AdminAccess::can("services.update"),
                    apps::remove_basic_auth,
                )
                .post(
                    "/services/{id}/github-webhook",
                    AdminAccess::can("services.deploy"),
                    apps::github_webhook,
                )
                .post(
                    "/scripts/{id}/run",
                    AdminAccess::can("docker.exec"),
                    apps::run_script,
                )
                .get(
                    "/actions",
                    AdminAccess::can("projects.view_any"),
                    actions::index,
                )
                .get(
                    "/notification-channels",
                    AdminAccess::can("notification_channels.view_any"),
                    notifications::index,
                )
                .post(
                    "/notification-channels/{id}/test",
                    AdminAccess::can("notification_channels.update"),
                    notifications::test,
                )
                .get(
                    "/storage-providers",
                    AdminAccess::can("storage_providers.view_any"),
                    backups::providers,
                )
                .get(
                    "/storage-providers/{id}/files",
                    AdminAccess::can(backups::RESTORE_PERMISSION),
                    backups::files,
                )
                .post(
                    "/storage-providers/{id}/check",
                    AdminAccess::can("storage_providers.update"),
                    backups::check,
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
