//! Certificate resolvers: the panel's administrators' only.

use rustcms::admin::*;
use rustcms::async_trait;

use crate::admin::{int, text};
use crate::policies::permissions;

/// `system.manage` for everything (they change Traefik), and no delete while
/// a domain or a deployed route uses the resolver.
pub struct ResolverPolicy;

#[async_trait]
impl Policy for ResolverPolicy {
    async fn allows(&self, ctx: &Ctx, ability: &str, record: Option<&Data>) -> bool {
        if !ctx.can(permissions::SYSTEM_MANAGE) {
            return false;
        }
        match (ability, record) {
            ("delete", Some(r)) => {
                let (Some(id), Some(name)) = (int(r, "id"), text(r, "name")) else {
                    return false;
                };
                crate::services::cert_service::in_use(&ctx.conn(), id, name)
                    .await
                    .is_ok_and(|used| !used)
            }
            _ => true,
        }
    }

    async fn admits(&self, ctx: &Ctx, _ability: &str) -> bool {
        ctx.can(permissions::SYSTEM_MANAGE)
    }

    fn view_depends_on_record(&self) -> bool {
        false
    }
}
