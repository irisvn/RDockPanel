//! Where a service's image comes from: an image (as before), a Git
//! repository built by the panel (its Dockerfile, or Nixpacks), or a
//! Dockerfile written in the panel; a deploy key for private repositories
//! (sealed); the deployment trigger's token (its hash).

use rustcms::async_trait;
use rustcms::migration::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait]
impl MigrationTrait for Migration {
    /// Schema change and migration record commit together, also on SQLite
    /// (whose default is no transaction), so a failed run can be retried.
    fn use_transaction(&self) -> Option<bool> {
        Some(true)
    }

    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let columns = [
            ColumnDef::new(Services::Source)
                .string_len(16)
                .not_null()
                .default("image")
                .to_owned(),
            ColumnDef::new(Services::GitUrl)
                .string_len(512)
                .null()
                .to_owned(),
            ColumnDef::new(Services::GitBranch)
                .string_len(255)
                .null()
                .to_owned(),
            ColumnDef::new(Services::GitPath)
                .string_len(255)
                .null()
                .to_owned(),
            ColumnDef::new(Services::Builder)
                .string_len(16)
                .not_null()
                .default("dockerfile")
                .to_owned(),
            ColumnDef::new(Services::DockerfilePath)
                .string_len(255)
                .null()
                .to_owned(),
            ColumnDef::new(Services::Dockerfile)
                .text()
                .null()
                .to_owned(),
            ColumnDef::new(Services::DeployKey).text().null().to_owned(),
            ColumnDef::new(Services::DeployKeyPublic)
                .string_len(255)
                .null()
                .to_owned(),
            ColumnDef::new(Services::TriggerHash)
                .string_len(64)
                .null()
                .to_owned(),
        ];
        for mut column in columns {
            add_column(manager, Services::Table, &mut column).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for column in [
            Services::TriggerHash,
            Services::DeployKeyPublic,
            Services::DeployKey,
            Services::Dockerfile,
            Services::DockerfilePath,
            Services::Builder,
            Services::GitPath,
            Services::GitBranch,
            Services::GitUrl,
            Services::Source,
        ] {
            drop_column(manager, Services::Table, column).await?;
        }
        Ok(())
    }
}

#[derive(DeriveIden)]
enum Services {
    Table,
    Source,
    GitUrl,
    GitBranch,
    GitPath,
    Builder,
    DockerfilePath,
    Dockerfile,
    DeployKey,
    DeployKeyPublic,
    TriggerHash,
}
