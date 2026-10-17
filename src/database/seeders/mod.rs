//! Seeders: `rustcms db:seed` runs [`DatabaseSeeder`], which calls the others
//! (`rustcms make:seeder <Name>` adds them here). Seeders may run more than
//! once: find rows before creating them.

use rustcms::database::Seeder;
use rustcms::{AppState, async_trait};

// rustcms:seeder-mods

pub struct DatabaseSeeder;

#[async_trait]
impl Seeder for DatabaseSeeder {
    fn name(&self) -> &'static str {
        "database"
    }

    async fn run(&self, state: &AppState) -> anyhow::Result<()> {
        rustcms::database::call(
            state,
            &[
                // rustcms:seeder-calls
            ],
        )
        .await
    }
}
