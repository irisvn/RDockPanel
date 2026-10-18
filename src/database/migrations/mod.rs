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
            // rustcms:migrations
        ];
        migrations.extend(app);
        migrations
    }
}
