//! `certificates` table: certificates uploaded to the panel. Their chain
//! (public), their private key sealed with `APP_KEY` (context
//! `certificates.key`), and what the panel read from the chain: the names it
//! covers, its expiry and its SHA-256 fingerprint.

use rustcms::crypto::{Context, Encrypted};
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "certificates")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub name: String,
    /// The PEM chain: the certificate, then its intermediates.
    #[sea_orm(column_type = "Text")]
    pub chain: String,
    #[sea_orm(column_type = "Text")]
    pub key: Encrypted<String>,
    /// The DNS names it covers (its subject alternative names), comma-separated.
    #[sea_orm(column_type = "Text")]
    pub names: String,
    pub not_after: DateTimeUtc,
    /// SHA-256 of the certificate (DER), lowercase hex.
    pub fingerprint: String,
    /// The expiry warning last sent (days left), so each is sent once.
    pub notified_days: Option<i32>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

/// Where `key` is sealed.
pub fn key_context() -> Context {
    Context::column("certificates", "key")
}

impl Model {
    /// The names it covers.
    pub fn name_list(&self) -> Vec<&str> {
        self.names.split(',').filter(|n| !n.is_empty()).collect()
    }
}
