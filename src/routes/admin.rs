//! The panel's own JSON endpoints, on the admin session, under
//! `/admin/api/x` (script pages call them with the session's CSRF token).
//! Every route declares the permission it requires; `system.manage` is
//! checked per object in the handlers (the panel's own containers, …).

use rustcms::prelude::*;

use crate::http::controllers::{
    actions_controller as actions, apps_controller as apps, backups_controller as backups,
    deploy_controller as deploy, docker_controller as docker, domains_controller as domains,
    git_controller as git, members_controller as members,
    notifications_controller as notifications, server_controller as server,
    system_controller as system, templates_controller as templates, traefik_controller as traefik,
};

/// Every admin route, with the access it requires.
pub fn routes() -> AdminRoutes {
    // A project's: any signed-in user, then the handler checks the
    // permission in the record's project (a role's, or a project role's:
    // `access_service`).
    let member = AdminAccess::authenticated;
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
                .get("/overview", member(), apps::overview)
                .get("/domains", member(), domains::index)
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
                .get("/panel", AdminAccess::can("system.manage"), traefik::panel)
                .get("/projects/{id}/members", member(), members::index)
                .post("/projects/{id}/members", member(), members::add)
                .patch("/projects/{id}/members/{user}", member(), members::update)
                .delete("/projects/{id}/members/{user}", member(), members::remove)
                .get("/users", member(), members::users)
                .events("/services/{id}/logs/{tail}", member(), docker::service_logs)
                .socket(
                    "/services/{id}/exec/{container}",
                    member(),
                    docker::service_exec,
                )
                .get(
                    "/git/providers",
                    AdminAccess::can("system.manage"),
                    git::index,
                )
                .post(
                    "/git/providers",
                    AdminAccess::can("system.manage"),
                    git::create,
                )
                .patch(
                    "/git/providers/{id}",
                    AdminAccess::can("system.manage"),
                    git::update,
                )
                .delete(
                    "/git/providers/{id}",
                    AdminAccess::can("system.manage"),
                    git::delete,
                )
                .post(
                    "/git/providers/{id}/test",
                    AdminAccess::can("system.manage"),
                    git::test,
                )
                .get("/services/{id}/github", member(), git::for_service)
                .get(
                    "/services/{id}/github/{provider}/repos",
                    member(),
                    git::repositories,
                )
                .get(
                    "/services/{id}/github/{provider}/branches",
                    member(),
                    git::branches,
                )
                .get(
                    "/system/status",
                    AdminAccess::can("system.manage"),
                    system::status,
                )
                .events(
                    "/system/logs/{tail}",
                    AdminAccess::can("system.manage"),
                    system::logs,
                )
                .post(
                    "/system/restart",
                    AdminAccess::can("system.manage"),
                    system::restart,
                )
                .post(
                    "/system/reboot",
                    AdminAccess::can("system.manage"),
                    system::reboot,
                )
                .post(
                    "/system/update",
                    AdminAccess::can("system.manage"),
                    system::check_update,
                )
                .post(
                    "/system/ip",
                    AdminAccess::can("system.manage"),
                    system::refresh_ip,
                )
                .post(
                    "/system/prune/{kind}",
                    AdminAccess::can("system.manage"),
                    system::prune,
                )
                .post(
                    "/panel/check",
                    AdminAccess::can("system.manage"),
                    traefik::check_panel,
                )
                .get(
                    "/traefik/status",
                    AdminAccess::can("system.manage"),
                    traefik::status,
                )
                .post(
                    "/traefik/restart",
                    AdminAccess::can("system.manage"),
                    traefik::restart,
                )
                .put(
                    "/traefik/middlewares",
                    AdminAccess::can("system.manage"),
                    traefik::save_middlewares,
                )
                .events(
                    "/traefik/logs/{tail}",
                    AdminAccess::can("system.manage"),
                    traefik::logs,
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
                .get("/services/{id}", member(), apps::service)
                .post("/projects/{id}/services", member(), apps::create_service)
                .get("/templates", member(), templates::list)
                .post(
                    "/templates/refresh",
                    AdminAccess::can("services.create"),
                    templates::refresh,
                )
                .get("/templates/{slug}", member(), templates::show)
                .get("/templates/{slug}/logo", member(), templates::logo)
                .post(
                    "/projects/{id}/templates/{slug}",
                    member(),
                    templates::create,
                )
                .post("/projects/{id}/schema", member(), templates::from_schema)
                .get("/deployments/{id}", member(), apps::deployment)
                .post("/services/{id}/deploy-key", member(), apps::deploy_key)
                .post("/services/{id}/trigger", member(), apps::trigger)
                .get("/services/{id}/credentials", member(), apps::credentials)
                .get("/services/{id}/env", member(), apps::env)
                .put("/services/{id}/env", member(), apps::put_env)
                // Database backups (configurations: the `database_backups` resource).
                .get("/services/{id}/backups", member(), backups::index)
                .post("/services/{id}/restore", member(), backups::restore)
                .post("/backups/{id}/run", member(), backups::run)
                .get("/backup-runs/{id}", member(), backups::show_run)
                .get("/services/{id}/stats", member(), apps::stats)
                .get("/services/{id}/errors", member(), apps::errors)
                .post("/services/{id}/basic-auth", member(), apps::add_basic_auth)
                .delete(
                    "/services/{id}/basic-auth/{user}",
                    member(),
                    apps::remove_basic_auth,
                )
                .post(
                    "/services/{id}/github-webhook",
                    member(),
                    apps::github_webhook,
                )
                .post("/scripts/{id}/run", member(), apps::run_script)
                .get("/actions", member(), actions::index)
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
            AdminRoutes::scope().events("/deployments/{id}/log", member(), deploy::log),
        )
}
