//! The policy of a service's records (domains, ports, mounts, redirects,
//! scripts, backups).

use rustcms::admin::*;
use rustcms::async_trait;

use super::{admits, may, parent_ok};
use crate::admin::int;

/// A service's records: their permissions (`<slug>.<ability>`, globally or
/// in the service's project), and a privileged service's changed by its
/// administrators only.
pub struct ChildPolicy(pub &'static str);

#[async_trait]
impl Policy for ChildPolicy {
    async fn allows(&self, ctx: &Ctx, ability: &str, record: Option<&Data>) -> bool {
        let permission = format!("{}.{ability}", self.0);
        let allowed = match record {
            Some(_) => may(ctx, &permission, record).await,
            None => admits(ctx, &permission).await,
        };
        if !allowed {
            return false;
        }
        if matches!(ability, "view" | "view_any") {
            return true;
        }
        match record.and_then(|r| int(r, "service_id")) {
            Some(id) => parent_ok(ctx, &ctx.conn(), id).await,
            None => true,
        }
    }

    async fn admits(&self, ctx: &Ctx, ability: &str) -> bool {
        admits(ctx, &format!("{}.{ability}", self.0)).await
    }

    fn view_depends_on_record(&self) -> bool {
        false
    }
}
