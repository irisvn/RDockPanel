//! `services.user`: the user a service's container runs as (`root`, `1000:1000`),
//! when it is not its image's. Only an import sets it (a container that ran
//! as another user than its image's): the form cannot.

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
            Services::Table,
            &mut ColumnDef::new(Services::RunUser)
                .string_len(64)
                .null()
                .to_owned(),
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        drop_column(manager, Services::Table, Services::RunUser).await
    }
}

#[derive(DeriveIden)]
enum Services {
    Table,
    #[sea_orm(iden = "run_user")]
    RunUser,
}
