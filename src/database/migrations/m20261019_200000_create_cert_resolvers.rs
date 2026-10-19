//! Certificate resolvers of Traefik the panel manages (Let's Encrypt over
//! HTTP, or over DNS with a provider's token, sealed), and the one a domain
//! chooses (none: the panel's default). A resolver in use cannot be deleted
//! (checked by the panel: SQLite cannot add the constraint to the table).
//! One resolver per DNS provider.

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
                .table(CertResolvers::Table)
                .col(
                    ColumnDef::new(CertResolvers::Id)
                        .big_integer()
                        .not_null()
                        .auto_increment()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(CertResolvers::Name)
                        .string_len(32)
                        .not_null(),
                )
                .col(ColumnDef::new(CertResolvers::Kind).string_len(8).not_null())
                .col(
                    ColumnDef::new(CertResolvers::Provider)
                        .string_len(32)
                        .null(),
                )
                .col(
                    ColumnDef::new(CertResolvers::Email)
                        .string_len(255)
                        .not_null(),
                )
                .col(ColumnDef::new(CertResolvers::Token).text().null())
                .col(
                    ColumnDef::new(CertResolvers::Staging)
                        .boolean()
                        .not_null()
                        .default(false),
                )
                .col(
                    ColumnDef::new(CertResolvers::CreatedAt)
                        .instant_now(manager)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(CertResolvers::UpdatedAt)
                        .instant_now(manager)
                        .not_null(),
                )
                .to_owned(),
        )
        .await?;
        create_index(
            manager,
            CertResolvers::Table,
            Index::create()
                .name("cert_resolvers_name_unique")
                .table(CertResolvers::Table)
                .col(CertResolvers::Name)
                .unique()
                .to_owned(),
        )
        .await?;
        // One resolver per DNS provider (its token is one variable of
        // Traefik's); HTTP ones have none (NULLs never collide).
        create_index(
            manager,
            CertResolvers::Table,
            Index::create()
                .name("cert_resolvers_provider_unique")
                .table(CertResolvers::Table)
                .col(CertResolvers::Provider)
                .unique()
                .to_owned(),
        )
        .await?;
        add_column(
            manager,
            ServiceDomains::Table,
            &mut ColumnDef::new(ServiceDomains::CertResolverId)
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
            ServiceDomains::CertResolverId,
        )
        .await?;
        drop_table(manager, CertResolvers::Table).await
    }
}

#[derive(DeriveIden)]
enum CertResolvers {
    Table,
    Id,
    Name,
    Kind,
    Provider,
    Email,
    Token,
    Staging,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum ServiceDomains {
    Table,
    CertResolverId,
}
