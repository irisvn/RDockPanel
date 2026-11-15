//! `tunnels` table: the panel's Cloudflare Tunnels, one cloudflared each
//! (`tunnel_service`). The token is sealed with `APP_KEY` (context
//! `tunnels.token`) and never shown.

use rustcms::crypto::{Context, Encrypted};
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "tunnels")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    /// `default` (the panel's first), or 1-40 lowercase letters, digits and dashes.
    pub name: String,
    #[sea_orm(column_type = "Text")]
    pub token: Encrypted<String>,
    pub enabled: bool,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

/// Where `token` is sealed.
pub fn token_context() -> Context {
    Context::column("tunnels", "token")
}

/// Every tunnel, the first made first.
pub async fn all<C: ConnectionTrait>(db: &C) -> Result<Vec<Model>, DbErr> {
    use sea_orm::QueryOrder;
    Entity::find().order_by_asc(Column::Id).all(db).await
}

/// The tunnel named `name`.
pub async fn named<C: ConnectionTrait>(db: &C, name: &str) -> Result<Option<Model>, DbErr> {
    Entity::find().filter(Column::Name.eq(name)).one(db).await
}
