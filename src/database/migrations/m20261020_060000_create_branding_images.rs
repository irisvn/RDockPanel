//! Settings → Branding's images (logo, dark logo, favicon): one row per
//! kind, the file's bytes (PNG, WebP or ICO, at most 512 KiB: MySQL's BLOB
//! holds 64 KiB, so MEDIUMBLOB there).

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
        let mut data = ColumnDef::new(BrandingImages::Data);
        match manager.get_database_backend() {
            sea_orm_migration::sea_orm::DatabaseBackend::MySql => {
                data.custom(Alias::new("MEDIUMBLOB"))
            }
            _ => data.blob(),
        };
        create_table(
            manager,
            Table::create()
                .table(BrandingImages::Table)
                .col(
                    ColumnDef::new(BrandingImages::Kind)
                        .string_len(16)
                        .not_null()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(BrandingImages::Mime)
                        .string_len(32)
                        .not_null(),
                )
                .col(data.not_null())
                .col(
                    ColumnDef::new(BrandingImages::UpdatedAt)
                        .instant_now(manager)
                        .not_null(),
                )
                .to_owned(),
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        drop_table(manager, BrandingImages::Table).await
    }
}

#[derive(DeriveIden)]
enum BrandingImages {
    Table,
    Kind,
    Mime,
    Data,
    UpdatedAt,
}
