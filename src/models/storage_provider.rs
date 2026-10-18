//! `storage_providers` table: where backups go. `local` (a directory of the
//! panel's server) or `s3` (any S3-compatible bucket), its secret key sealed
//! with `APP_KEY` (context `storage_providers.secret_key`).

use rustcms::crypto::{Context, Encrypted};
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "storage_providers")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub name: String,
    /// `local` or `s3`.
    pub kind: String,
    /// `local`: an absolute directory.
    pub local_path: Option<String>,
    /// `s3`: the endpoint (`https://s3.eu-west-1.amazonaws.com`, a MinIO's…).
    pub endpoint: Option<String>,
    pub region: Option<String>,
    pub bucket: Option<String>,
    pub access_key: Option<String>,
    #[sea_orm(column_type = "Text", nullable)]
    pub secret_key: Option<Encrypted<String>>,
    /// `https://endpoint/bucket/…` rather than `https://bucket.endpoint/…`.
    pub path_style: bool,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

/// Where `secret_key` is sealed.
pub fn secret_key_context() -> Context {
    Context::column("storage_providers", "secret_key")
}
