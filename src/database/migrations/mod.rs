//! Migrations, one per change (`rustcms make:migration <name>` adds them here).

use rustcms::migration::{MigrationTrait, MigratorTrait};

mod m20261017_060642_create_actions_table;
mod m20261018_084009_create_projects_table;
mod m20261018_084010_create_services_table;
mod m20261018_084011_create_service_domains_table;
mod m20261018_084012_create_service_ports_table;
mod m20261018_084013_create_service_mounts_table;
mod m20261018_084014_create_deployments_table;
mod m20261018_132946_add_secrets_to_services;
mod m20261018_161513_add_content_to_service_mounts;
mod m20261018_163345_add_kind_to_services;
mod m20261018_173253_seal_file_mounts;
mod m20261018_180047_add_source_to_services;
mod m20261019_020000_create_backup_tables;
mod m20261019_040000_create_notification_channels_table;
mod m20261019_060000_add_compose_to_services;
mod m20261019_080000_create_redirects_and_basic_auth;
mod m20261019_100000_add_notes_and_webhook_to_services;
mod m20261019_120000_add_maintenance_to_services;
mod m20261019_140000_create_service_scripts_table;
mod m20261019_160000_add_capabilities_to_services;
mod m20261019_180000_add_middlewares_to_service_domains;
mod m20261019_200000_create_cert_resolvers;
mod m20261020_000000_create_certificates;
mod m20261020_010000_create_panel_settings;
mod m20261020_020000_add_force_rebuild_to_deployments;
mod m20261020_030000_create_git_providers;
mod m20261020_040000_add_events_to_notification_channels;
mod m20261020_050000_create_project_members;
mod m20261020_060000_create_branding_images;
mod m20261021_000000_builds_use_a_dockerfile;
mod m20261022_000000_create_firewall_tables;
mod m20261023_000000_add_rate_limit_to_services;
mod m20261024_000000_add_maintenance_page;
// rustcms:migration-mods

pub struct Migrator;

impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        let mut migrations = rustcms::migration::framework();
        let app: Vec<Box<dyn MigrationTrait>> = vec![
            Box::new(m20261017_060642_create_actions_table::Migration),
            Box::new(m20261018_084009_create_projects_table::Migration),
            Box::new(m20261018_084010_create_services_table::Migration),
            Box::new(m20261018_084011_create_service_domains_table::Migration),
            Box::new(m20261018_084012_create_service_ports_table::Migration),
            Box::new(m20261018_084013_create_service_mounts_table::Migration),
            Box::new(m20261018_084014_create_deployments_table::Migration),
            Box::new(m20261018_132946_add_secrets_to_services::Migration),
            Box::new(m20261018_161513_add_content_to_service_mounts::Migration),
            Box::new(m20261018_163345_add_kind_to_services::Migration),
            Box::new(m20261018_173253_seal_file_mounts::Migration),
            Box::new(m20261018_180047_add_source_to_services::Migration),
            Box::new(m20261019_020000_create_backup_tables::Migration),
            Box::new(m20261019_040000_create_notification_channels_table::Migration),
            Box::new(m20261019_060000_add_compose_to_services::Migration),
            Box::new(m20261019_080000_create_redirects_and_basic_auth::Migration),
            Box::new(m20261019_100000_add_notes_and_webhook_to_services::Migration),
            Box::new(m20261019_120000_add_maintenance_to_services::Migration),
            Box::new(m20261019_140000_create_service_scripts_table::Migration),
            Box::new(m20261019_160000_add_capabilities_to_services::Migration),
            Box::new(m20261019_180000_add_middlewares_to_service_domains::Migration),
            Box::new(m20261019_200000_create_cert_resolvers::Migration),
            Box::new(m20261020_000000_create_certificates::Migration),
            Box::new(m20261020_010000_create_panel_settings::Migration),
            Box::new(m20261020_020000_add_force_rebuild_to_deployments::Migration),
            Box::new(m20261020_030000_create_git_providers::Migration),
            Box::new(m20261020_040000_add_events_to_notification_channels::Migration),
            Box::new(m20261020_050000_create_project_members::Migration),
            Box::new(m20261020_060000_create_branding_images::Migration),
            Box::new(m20261021_000000_builds_use_a_dockerfile::Migration),
            Box::new(m20261022_000000_create_firewall_tables::Migration),
            Box::new(m20261023_000000_add_rate_limit_to_services::Migration),
            Box::new(m20261024_000000_add_maintenance_page::Migration),
            // rustcms:migrations
        ];
        migrations.extend(app);
        migrations
    }
}
