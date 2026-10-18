//! A service's Linux capabilities (added, dropped), sysctls and supplemental
//! groups (as
//! EasyPanel's Advanced → Deploy).

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
        for column in [
            Services::CapAdd,
            Services::CapDrop,
            Services::Sysctls,
            Services::Groups,
        ] {
            add_column(
                manager,
                Services::Table,
                &mut ColumnDef::new(column).text().null().to_owned(),
            )
            .await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for column in [
            Services::Groups,
            Services::Sysctls,
            Services::CapDrop,
            Services::CapAdd,
        ] {
            drop_column(manager, Services::Table, column).await?;
        }
        Ok(())
    }
}

#[derive(DeriveIden)]
enum Services {
    Table,
    CapAdd,
    CapDrop,
    Sysctls,
    Groups,
}
