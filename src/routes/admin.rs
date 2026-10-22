//! The panel's own JSON endpoints, on the admin session, under
//! `/admin/api/x` (script pages call them with the session's CSRF token).
//! Every route declares the permission it requires; `system.manage` is
//! checked per object in the handlers (the panel's own containers, …).

use rustcms::prelude::*;

use crate::http::controllers::{
    actions_controller as actions, apps_controller as apps, backups_controller as backups,
    branding_controller as branding, capacity_controller as capacity, deploy_controller as deploy,
    docker_controller as docker, domains_controller as domains, firewall_controller as firewall,
    git_controller as git, home_controller as home, maintenance_controller as maintenance,
    members_controller as members, notifications_controller as notifications,
    server_controller as server, system_controller as system, templates_controller as templates,
    tokens_controller as tokens, traefik_controller as traefik, tunnel_controller as tunnel,
};
use crate::policies::permissions;

/// Every admin route, with the access it requires.
pub fn routes() -> AdminRoutes {
    // A project's: any signed-in user, then the handler checks the
    // permission in the record's project (a role's, or a project role's:
    // `access_service`).
    let member = AdminAccess::authenticated;
    let view = || AdminAccess::can(permissions::DOCKER_VIEW);
    let manage = || AdminAccess::can(permissions::DOCKER_MANAGE);
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
                    AdminAccess::can(permissions::DOCKER_LOGS),
                    docker::logs,
                )
                .events(
                    "/containers/{id}/logs/follow",
                    AdminAccess::can(permissions::DOCKER_LOGS),
                    docker::follow_logs,
                )
                .events(
                    "/containers/{id}/logs/follow/{tail}",
                    AdminAccess::can(permissions::DOCKER_LOGS),
                    docker::follow_logs_tail,
                )
                .events("/containers/{id}/stats", view(), docker::live_stats)
                .socket(
                    "/containers/{id}/exec",
                    AdminAccess::can(permissions::DOCKER_EXEC),
                    docker::exec,
                )
                .get("/images", view(), docker::images)
                .post("/images/pull", manage(), docker::pull_image)
                .delete("/images/{id}", manage(), docker::remove_image)
                .delete("/images", manage(), docker::remove_image_named)
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
                .get(
                    "/info",
                    AdminAccess::can(permissions::SERVER_VIEW),
                    server::info,
                )
                .get(
                    "/samples",
                    AdminAccess::can(permissions::SERVER_VIEW),
                    server::samples,
                )
                .events(
                    "/live",
                    AdminAccess::can(permissions::SERVER_VIEW),
                    server::live,
                ),
        )
        .nest(
            "/apps",
            AdminRoutes::scope()
                .get("/overview", member(), apps::overview)
                .get("/domains", member(), domains::index)
                .get("/domains/dns", member(), domains::dns)
                .get(
                    "/traefik/resolvers",
                    AdminAccess::can(permissions::SYSTEM_MANAGE),
                    traefik::resolvers,
                )
                .post(
                    "/traefik/apply",
                    AdminAccess::can(permissions::SYSTEM_MANAGE),
                    traefik::apply,
                )
                .get(
                    "/panel",
                    AdminAccess::can(permissions::SYSTEM_MANAGE),
                    traefik::panel,
                )
                // The Firewall page: the server's listeners and firewall.
                .get(
                    "/firewall",
                    AdminAccess::can(permissions::SYSTEM_MANAGE),
                    firewall::show,
                )
                .get(
                    "/firewall/preview",
                    AdminAccess::can(permissions::SYSTEM_MANAGE),
                    firewall::preview,
                )
                .post(
                    "/firewall/openings",
                    AdminAccess::can(permissions::SYSTEM_MANAGE),
                    firewall::create_opening,
                )
                .patch(
                    "/firewall/openings/{id}",
                    AdminAccess::can(permissions::SYSTEM_MANAGE),
                    firewall::update_opening,
                )
                .delete(
                    "/firewall/openings/{id}",
                    AdminAccess::can(permissions::SYSTEM_MANAGE),
                    firewall::delete_opening,
                )
                .post(
                    "/firewall/blocked",
                    AdminAccess::can(permissions::SYSTEM_MANAGE),
                    firewall::create_blocked,
                )
                .delete(
                    "/firewall/blocked/{id}",
                    AdminAccess::can(permissions::SYSTEM_MANAGE),
                    firewall::delete_blocked,
                )
                .post(
                    "/firewall/apply",
                    AdminAccess::can(permissions::SYSTEM_MANAGE),
                    firewall::apply,
                )
                .post(
                    "/firewall/keep",
                    AdminAccess::can(permissions::SYSTEM_MANAGE),
                    firewall::keep,
                )
                .post(
                    "/firewall/revert",
                    AdminAccess::can(permissions::SYSTEM_MANAGE),
                    firewall::revert,
                )
                .post(
                    "/firewall/disable",
                    AdminAccess::can(permissions::SYSTEM_MANAGE),
                    firewall::disable,
                )
                // The homepage at `/` (Settings → Panel): on or off.
                .get(
                    "/home",
                    AdminAccess::can(permissions::SYSTEM_MANAGE),
                    home::show,
                )
                .put(
                    "/home",
                    AdminAccess::can(permissions::SYSTEM_MANAGE),
                    home::update,
                )
                // The shared maintenance page (Settings).
                .get(
                    "/settings/maintenance",
                    AdminAccess::can(permissions::SYSTEM_MANAGE),
                    maintenance::show_settings,
                )
                .put(
                    "/settings/maintenance",
                    AdminAccess::can(permissions::SYSTEM_MANAGE),
                    maintenance::update_settings,
                )
                // Branding (Settings): the panel's name, color and images.
                .get(
                    "/branding",
                    AdminAccess::can(permissions::SYSTEM_MANAGE),
                    branding::index,
                )
                .put(
                    "/branding",
                    AdminAccess::can(permissions::SYSTEM_MANAGE),
                    branding::update,
                )
                .put(
                    "/branding/images/{kind}",
                    AdminAccess::can(permissions::SYSTEM_MANAGE),
                    branding::upload,
                )
                .delete(
                    "/branding/images/{kind}",
                    AdminAccess::can(permissions::SYSTEM_MANAGE),
                    branding::remove,
                )
                // Cloudflare Tunnel (Settings).
                .get(
                    "/tunnel",
                    AdminAccess::can(permissions::SYSTEM_MANAGE),
                    tunnel::status,
                )
                .post(
                    "/tunnel/enable",
                    AdminAccess::can(permissions::SYSTEM_MANAGE),
                    tunnel::enable,
                )
                .post(
                    "/tunnel/disable",
                    AdminAccess::can(permissions::SYSTEM_MANAGE),
                    tunnel::disable,
                )
                // Each user's own API tokens.
                .get("/tokens", member(), tokens::index)
                .post("/tokens", member(), tokens::create)
                .delete("/tokens/{id}", member(), tokens::revoke)
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
                    AdminAccess::can(permissions::SYSTEM_MANAGE),
                    git::index,
                )
                .post(
                    "/git/providers",
                    AdminAccess::can(permissions::SYSTEM_MANAGE),
                    git::create,
                )
                .patch(
                    "/git/providers/{id}",
                    AdminAccess::can(permissions::SYSTEM_MANAGE),
                    git::update,
                )
                .delete(
                    "/git/providers/{id}",
                    AdminAccess::can(permissions::SYSTEM_MANAGE),
                    git::delete,
                )
                .post(
                    "/git/providers/{id}/test",
                    AdminAccess::can(permissions::SYSTEM_MANAGE),
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
                    AdminAccess::can(permissions::SYSTEM_MANAGE),
                    system::status,
                )
                .events(
                    "/system/logs/{tail}",
                    AdminAccess::can(permissions::SYSTEM_MANAGE),
                    system::logs,
                )
                .post(
                    "/system/restart",
                    AdminAccess::can(permissions::SYSTEM_MANAGE),
                    system::restart,
                )
                .post(
                    "/system/reboot",
                    AdminAccess::can(permissions::SYSTEM_MANAGE),
                    system::reboot,
                )
                .get(
                    "/system/version",
                    AdminAccess::can(permissions::SYSTEM_MANAGE),
                    system::version,
                )
                .post(
                    "/system/version/check",
                    AdminAccess::can(permissions::SYSTEM_MANAGE),
                    system::check_version,
                )
                .get(
                    "/system/version/notice",
                    AdminAccess::can(permissions::SYSTEM_MANAGE),
                    system::version_notice,
                )
                .post(
                    "/system/version/dismiss",
                    AdminAccess::can(permissions::SYSTEM_MANAGE),
                    system::dismiss_version,
                )
                .post(
                    "/system/version/update",
                    AdminAccess::can(permissions::SYSTEM_MANAGE),
                    system::update,
                )
                .get(
                    "/system/cleanup",
                    AdminAccess::can(permissions::SYSTEM_MANAGE),
                    system::cleanup_preview,
                )
                .post(
                    "/system/cleanup",
                    AdminAccess::can(permissions::SYSTEM_MANAGE),
                    system::cleanup,
                )
                .post(
                    "/system/ip",
                    AdminAccess::can(permissions::SYSTEM_MANAGE),
                    system::refresh_ip,
                )
                .post(
                    "/system/prune/{kind}",
                    AdminAccess::can(permissions::SYSTEM_MANAGE),
                    system::prune,
                )
                .post(
                    "/panel/check",
                    AdminAccess::can(permissions::SYSTEM_MANAGE),
                    traefik::check_panel,
                )
                .get(
                    "/traefik/status",
                    AdminAccess::can(permissions::SYSTEM_MANAGE),
                    traefik::status,
                )
                .post(
                    "/traefik/restart",
                    AdminAccess::can(permissions::SYSTEM_MANAGE),
                    traefik::restart,
                )
                .put(
                    "/traefik/middlewares",
                    AdminAccess::can(permissions::SYSTEM_MANAGE),
                    traefik::save_middlewares,
                )
                .events(
                    "/traefik/logs/{tail}",
                    AdminAccess::can(permissions::SYSTEM_MANAGE),
                    traefik::logs,
                )
                .get(
                    "/certificates",
                    AdminAccess::can(permissions::SYSTEM_MANAGE),
                    traefik::certificates,
                )
                .post(
                    "/certificates",
                    AdminAccess::can(permissions::SYSTEM_MANAGE),
                    traefik::upload_certificate,
                )
                .put(
                    "/certificates/{id}",
                    AdminAccess::can(permissions::SYSTEM_MANAGE),
                    traefik::replace_certificate,
                )
                .delete(
                    "/certificates/{id}",
                    AdminAccess::can(permissions::SYSTEM_MANAGE),
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
                .post("/capacity", member(), capacity::check)
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
                // Its maintenance page: its logo, previews being written.
                .put(
                    "/services/{id}/maintenance-logo",
                    member(),
                    maintenance::upload_logo,
                )
                .delete(
                    "/services/{id}/maintenance-logo",
                    member(),
                    maintenance::remove_logo,
                )
                .post(
                    "/maintenance/preview",
                    member(),
                    maintenance::create_preview,
                )
                .get(
                    "/maintenance/preview/{id}",
                    member(),
                    maintenance::show_preview,
                )
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
                    AdminAccess::can(permissions::BACKUPS_RESTORE),
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
