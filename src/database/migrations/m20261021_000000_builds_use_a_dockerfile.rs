//! Nixpacks is gone (0.5.5): a service that built with it builds with the
//! repository's Dockerfile (its next build says so when there is none).

use rustcms::async_trait;
use rustcms::migration::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "UPDATE services SET builder = 'dockerfile' WHERE builder = 'nixpacks'",
            )
            .await?;
        Ok(())
    }

    /// Nothing to undo: a Dockerfile build works before this too.
    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}
