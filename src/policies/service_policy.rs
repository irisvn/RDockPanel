//! Services: the permissions, globally or in the service's project.

use rustcms::admin::*;
use rustcms::async_trait;

use crate::admin::int;
use crate::models::service::NEW;

/// The permissions, and no delete once deployed (Destroy first).
pub struct ServicePolicy;

#[async_trait]
impl Policy for ServicePolicy {
    async fn allows(&self, ctx: &Ctx, ability: &str, record: Option<&Data>) -> bool {
        let permission = format!("services.{ability}");
        let allowed = match record {
            Some(_) => crate::policies::may(ctx, &permission, record).await,
            None => crate::policies::admits(ctx, &permission).await,
        };
        if !allowed {
            return false;
        }
        // A privileged service (added capabilities, sysctls or groups):
        // acted on by the panel's administrators only.
        if !matches!(ability, "view" | "view_any")
            && let Some(id) = record.and_then(|r| int(r, "id"))
            && !crate::policies::parent_ok(ctx, &ctx.conn(), id).await
        {
            return false;
        }
        // Deployed, deploying, or left by a failed first run: Destroy first.
        !(ability == "delete"
            && record.is_some_and(|r| r.get("status").and_then(|s| s.as_str()) != Some(NEW)))
    }

    async fn admits(&self, ctx: &Ctx, ability: &str) -> bool {
        crate::policies::admits(ctx, &format!("services.{ability}")).await
    }

    fn view_depends_on_record(&self) -> bool {
        false
    }
}
