//! Business rules shared by the admin panel, the API, jobs and seeders. `rustcms make:service <Name>` adds modules here.

pub mod action_service;
pub mod backup_service;
pub mod build_service;
pub mod creation_service;
pub mod database_service;
pub mod deploy_rules_service;
pub mod deploy_service;
pub mod deployment_service;
pub mod docker_service;
pub mod dotenv;
pub mod live_service;
pub mod placeholders;
pub mod storage_service;
pub mod system_stats_service;
pub mod template_engine;
pub mod template_service;
pub mod traefik_service;
// rustcms:services
