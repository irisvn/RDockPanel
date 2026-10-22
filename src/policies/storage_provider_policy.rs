//! Storage providers: the permissions, and none deleted while in use.

use rustcms::admin::*;
use rustcms::async_trait;
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};

use crate::admin::int;
use crate::models::database_backup;

/// The permissions, and no delete while a backup uses the provider.
pub struct StorageProviderPolicy;

#[async_trait]
impl Policy for StorageProviderPolicy {
    async fn allows(&self, ctx: &Ctx, ability: &str, record: Option<&Data>) -> bool {
        if !ctx.can(&format!("storage_providers.{ability}")) {
            return false;
        }
        match (ability, record.and_then(|r| int(r, "id"))) {
            ("delete", Some(id)) => database_backup::Entity::find()
                .filter(database_backup::Column::StorageProviderId.eq(id))
                .count(&ctx.conn())
                .await
                .is_ok_and(|n| n == 0),
            _ => true,
        }
    }

    async fn admits(&self, ctx: &Ctx, ability: &str) -> bool {
        ctx.can(&format!("storage_providers.{ability}"))
    }

    fn view_depends_on_record(&self) -> bool {
        false
    }
}
