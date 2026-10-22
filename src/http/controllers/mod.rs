//! Request handlers, one module per model (`rustcms make:api <Model>`).

pub mod actions_controller;
pub mod api_v1_controller;
pub mod apps_controller;
pub mod backups_controller;
pub mod branding_controller;
pub mod deploy_controller;
pub mod docker_controller;
pub mod domains_controller;
pub mod firewall_controller;
pub mod git_controller;
pub mod home_controller;
pub mod maintenance_controller;
pub mod members_controller;
pub mod notifications_controller;
pub mod server_controller;
pub mod system_controller;
pub mod templates_controller;
pub mod tokens_controller;
pub mod traefik_controller;
pub mod trigger_controller;
pub mod tunnel_controller;
// rustcms:controller-mods
