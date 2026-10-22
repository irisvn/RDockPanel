//! Permissions beyond each resource's CRUD abilities (declared, with their
//! labels, in `providers/auth_service_provider.rs`).

/// Opening the apps pages, nothing more: a member's (their projects say
/// what they see there).
pub const APPS_ACCESS: &str = "apps.access";
/// Deploying, starting, stopping, restarting and rolling back services.
pub const SERVICES_DEPLOY: &str = "services.deploy";
/// Removing a service's Swarm service and its routes.
pub const SERVICES_DESTROY: &str = "services.destroy";
/// A service's logs, without Docker's pages.
pub const SERVICES_LOGS: &str = "services.logs";
/// A shell in a service's running container.
pub const SERVICES_EXEC: &str = "services.exec";
/// Reading environment values (an audited read): the Environment tab.
pub const SERVICES_SECRETS: &str = "services.secrets";
/// Adding, changing and removing a project's members.
pub const PROJECTS_MEMBERS: &str = "projects.members";
/// Running a backup by hand.
pub const BACKUPS_RUN: &str = "backups.run";
/// Restoring one (it replaces the database's data).
pub const BACKUPS_RESTORE: &str = "backups.restore";
/// Docker's containers, images, volumes and networks.
pub const DOCKER_VIEW: &str = "docker.view";
/// Starting, stopping, removing and pruning them.
pub const DOCKER_MANAGE: &str = "docker.manage";
/// Any container's logs.
pub const DOCKER_LOGS: &str = "docker.logs";
/// A shell in any container.
pub const DOCKER_EXEC: &str = "docker.exec";
/// The server's CPU, memory, disk and network.
pub const SERVER_VIEW: &str = "server.view";
/// The panel's administrators: settings, Traefik, certificates, the panel's
/// own Docker objects, privileged services, reboot.
pub const SYSTEM_MANAGE: &str = "system.manage";
