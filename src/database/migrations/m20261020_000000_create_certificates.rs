//! Certificates uploaded to the panel (a PEM chain and its key, the key
//! sealed), served by Traefik for the names they cover, and the one a domain
//! chooses (none: its resolver's). A certificate in use cannot be deleted
//! (checked by the panel).

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
        create_table(
            manager,
            Table::create()
                .table(Certificates::Table)
                .col(
                    ColumnDef::new(Certificates::Id)
                        .big_integer()
                        .not_null()
                        .auto_increment()
                        .primary_key(),
                )
                .col(ColumnDef::new(Certificates::Name).string_len(64).not_null())
                .col(ColumnDef::new(Certificates::Chain).text().not_null())
                .col(ColumnDef::new(Certificates::Key).text().not_null())
                .col(ColumnDef::new(Certificates::Names).text().not_null())
                .col(
                    ColumnDef::new(Certificates::NotAfter)
                        .instant(manager)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Certificates::Fingerprint)
                        .string_len(64)
                        .not_null(),
                )
                .col(ColumnDef::new(Certificates::NotifiedDays).integer().null())
                .col(
                    ColumnDef::new(Certificates::CreatedAt)
                        .instant_now(manager)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Certificates::UpdatedAt)
                        .instant_now(manager)
                        .not_null(),
                )
                .to_owned(),
        )
        .await?;
        create_index(
            manager,
            Certificates::Table,
            Index::create()
                .name("certificates_fingerprint_unique")
                .table(Certificates::Table)
                .col(Certificates::Fingerprint)
                .unique()
                .to_owned(),
        )
        .await?;
        add_column(
            manager,
            ServiceDomains::Table,
            &mut ColumnDef::new(ServiceDomains::CertificateId)
                .big_integer()
                .null()
                .to_owned(),
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        drop_column(
            manager,
            ServiceDomains::Table,
            ServiceDomains::CertificateId,
        )
        .await?;
        drop_table(manager, Certificates::Table).await
    }
}

#[derive(DeriveIden)]
enum Certificates {
    Table,
    Id,
    Name,
    Chain,
    Key,
    Names,
    NotAfter,
    Fingerprint,
    NotifiedDays,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum ServiceDomains {
    Table,
    CertificateId,
}
