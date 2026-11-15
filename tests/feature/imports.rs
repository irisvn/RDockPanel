//! Importing existing containers (docs/import.md).

use rdockpanel::database::migrations::Migrator;
use rustcms_testing::{TestDb, db_test};
use sea_orm::ConnectionTrait;

use crate::support::{self, fake_docker::FakeDocker};

db_test!(
    the_import_tables_are_the_panels_own,
    migrate = Migrator,
    |db: TestDb| async move {
        // RustCMS has an `imports` table (CSV files): a table of the same name
        // would be skipped by `create_table` (if not exists) without a word.
        let app = support::app_with_docker(db, FakeDocker::default()).await;
        let db = app.state().db();
        for query in [
            "SELECT state, lease_token, plan FROM container_imports",
            "SELECT choice, original_restart FROM container_import_items",
            "SELECT panel_volume, copied_at FROM container_import_volumes",
            "SELECT container_id, volume FROM container_import_claims",
            "SELECT backup_path, sha256 FROM container_import_files",
            "SELECT admin_only FROM services",
            "SELECT link_target FROM service_mounts",
        ] {
            db.execute_unprepared(query)
                .await
                .unwrap_or_else(|e| panic!("{query}: {e}"));
        }
    }
);
