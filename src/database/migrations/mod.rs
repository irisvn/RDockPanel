//! Migrations, one per change (`rustcms make:migration <name>` adds them here).

use rustcms::migration::{MigrationTrait, MigratorTrait};

// rustcms:migration-mods

pub struct Migrator;

impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        let mut migrations = rustcms::migration::framework();
        let app: Vec<Box<dyn MigrationTrait>> = vec![
            // rustcms:migrations
        ];
        migrations.extend(app);
        migrations
    }
}
