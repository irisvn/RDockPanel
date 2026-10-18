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
                .table(NotificationChannels::Table)
                .col(
                    ColumnDef::new(NotificationChannels::Id)
                        .big_integer()
                        .not_null()
                        .auto_increment()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(NotificationChannels::Name)
                        .string_len(64)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(NotificationChannels::Kind)
                        .string_len(16)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(NotificationChannels::Enabled)
                        .boolean()
                        .not_null()
                        .default(true),
                )
                .col(
                    ColumnDef::new(NotificationChannels::OnDeploy)
                        .boolean()
                        .not_null()
                        .default(true),
                )
                .col(
                    ColumnDef::new(NotificationChannels::OnBackup)
                        .boolean()
                        .not_null()
                        .default(true),
                )
                .col(
                    ColumnDef::new(NotificationChannels::OnDisk)
                        .boolean()
                        .not_null()
                        .default(true),
                )
                .col(
                    ColumnDef::new(NotificationChannels::DiskThreshold)
                        .integer()
                        .not_null()
                        .default(90),
                )
                .col(
                    ColumnDef::new(NotificationChannels::ChatId)
                        .string_len(64)
                        .null(),
                )
                .col(
                    ColumnDef::new(NotificationChannels::SmtpHost)
                        .string_len(255)
                        .null(),
                )
                .col(
                    ColumnDef::new(NotificationChannels::SmtpPort)
                        .integer()
                        .null(),
                )
                .col(
                    ColumnDef::new(NotificationChannels::SmtpUsername)
                        .string_len(255)
                        .null(),
                )
                .col(
                    ColumnDef::new(NotificationChannels::SmtpFrom)
                        .string_len(255)
                        .null(),
                )
                .col(
                    ColumnDef::new(NotificationChannels::EmailTo)
                        .string_len(1024)
                        .null(),
                )
                .col(ColumnDef::new(NotificationChannels::Secret).text().null())
                .col(
                    ColumnDef::new(NotificationChannels::CreatedAt)
                        .instant_now(manager)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(NotificationChannels::UpdatedAt)
                        .instant_now(manager)
                        .not_null(),
                )
                .to_owned(),
        )
        .await?;
        create_index(
            manager,
            NotificationChannels::Table,
            Index::create()
                .name("notification_channels_name_unique")
                .table(NotificationChannels::Table)
                .col(NotificationChannels::Name)
                .unique()
                .to_owned(),
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        drop_table(manager, NotificationChannels::Table).await
    }
}

#[derive(DeriveIden)]
enum NotificationChannels {
    Table,
    Id,
    Name,
    Kind,
    Enabled,
    OnDeploy,
    OnBackup,
    OnDisk,
    DiskThreshold,
    ChatId,
    SmtpHost,
    SmtpPort,
    SmtpUsername,
    SmtpFrom,
    EmailTo,
    Secret,
    CreatedAt,
    UpdatedAt,
}
