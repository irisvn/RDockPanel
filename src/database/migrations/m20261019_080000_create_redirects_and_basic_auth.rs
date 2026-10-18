//! A service's redirects (Traefik `redirectRegex`) and the users of its HTTP
//! basic authentication (Traefik `basicAuth`, bcrypt hashes).

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
                .table(ServiceRedirects::Table)
                .col(
                    ColumnDef::new(ServiceRedirects::Id)
                        .big_integer()
                        .not_null()
                        .auto_increment()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(ServiceRedirects::ServiceId)
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ServiceRedirects::Regex)
                        .string_len(500)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ServiceRedirects::Replacement)
                        .string_len(500)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ServiceRedirects::Permanent)
                        .boolean()
                        .not_null()
                        .default(false),
                )
                .col(
                    ColumnDef::new(ServiceRedirects::Enabled)
                        .boolean()
                        .not_null()
                        .default(true),
                )
                .col(
                    ColumnDef::new(ServiceRedirects::CreatedAt)
                        .instant_now(manager)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ServiceRedirects::UpdatedAt)
                        .instant_now(manager)
                        .not_null(),
                )
                .foreign_key(
                    ForeignKey::create()
                        .from(ServiceRedirects::Table, ServiceRedirects::ServiceId)
                        .to(Services::Table, Services::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;
        create_index(
            manager,
            ServiceRedirects::Table,
            Index::create()
                .name("service_redirects_service_index")
                .table(ServiceRedirects::Table)
                .col(ServiceRedirects::ServiceId)
                .to_owned(),
        )
        .await?;
        create_table(
            manager,
            Table::create()
                .table(ServiceBasicAuth::Table)
                .col(
                    ColumnDef::new(ServiceBasicAuth::Id)
                        .big_integer()
                        .not_null()
                        .auto_increment()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(ServiceBasicAuth::ServiceId)
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ServiceBasicAuth::Username)
                        .string_len(64)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ServiceBasicAuth::PasswordHash)
                        .string_len(255)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ServiceBasicAuth::CreatedAt)
                        .instant_now(manager)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ServiceBasicAuth::UpdatedAt)
                        .instant_now(manager)
                        .not_null(),
                )
                .foreign_key(
                    ForeignKey::create()
                        .from(ServiceBasicAuth::Table, ServiceBasicAuth::ServiceId)
                        .to(Services::Table, Services::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;
        create_index(
            manager,
            ServiceBasicAuth::Table,
            Index::create()
                .name("service_basic_auth_user_unique")
                .table(ServiceBasicAuth::Table)
                .col(ServiceBasicAuth::ServiceId)
                .col(ServiceBasicAuth::Username)
                .unique()
                .to_owned(),
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        drop_table(manager, ServiceBasicAuth::Table).await?;
        drop_table(manager, ServiceRedirects::Table).await
    }
}

#[derive(DeriveIden)]
enum ServiceRedirects {
    Table,
    Id,
    ServiceId,
    Regex,
    Replacement,
    Permanent,
    Enabled,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
#[sea_orm(iden = "service_basic_auth")]
enum ServiceBasicAuth {
    Table,
    Id,
    ServiceId,
    Username,
    PasswordHash,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Services {
    Table,
    Id,
}
