//! `container_imports.flags`: what an import has become, apart from where it
//! is (`ingress`: traffic could have reached a new service; `back`: a move is
//! on its way back). Kept apart from `step` (where to go on) so that a
//! failure that waits for the admin forgets neither.

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
            ContainerImports::Table,
            &mut ColumnDef::new(ContainerImports::Flags)
                .string_len(64)
                .null()
                .to_owned(),
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        drop_column(manager, ContainerImports::Table, ContainerImports::Flags).await
    }
}

#[derive(DeriveIden)]
enum ContainerImports {
    Table,
    Flags,
}
