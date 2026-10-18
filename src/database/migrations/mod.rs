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
            // rustcms:migrations
        ];
        migrations.extend(app);
        migrations
    }
}
