//! A deployment asked as "Force rebuild": built without the builder's
//! cache, its tasks replaced even when nothing changed.

use rustcms::async_trait;
use rustcms::migration::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait]
impl MigrationTrait for Migration {
    fn use_transaction(&self) -> Option<bool> {
        Some(true)
    }

    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        add_column(
            manager,
            Deployments::Table,
            &mut ColumnDef::new(Deployments::ForceRebuild)
                .boolean()
                .not_null()
                .default(false)
                .to_owned(),
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        drop_column(manager, Deployments::Table, Deployments::ForceRebuild).await
    }
}

#[derive(DeriveIden)]
enum Deployments {
    Table,
    ForceRebuild,
}
