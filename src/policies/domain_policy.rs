//! A service's domains: as its other records, and a wildcard or a domain
//! with its own certificate kept to the panel's administrators.

use rustcms::admin::*;
use rustcms::async_trait;

use crate::admin::{is_set, text};
use crate::policies::permissions;

/// The service's records' policy, and a wildcard changed or removed by the
/// panel's administrators only.
pub struct DomainPolicy;

#[async_trait]
impl Policy for DomainPolicy {
    async fn allows(&self, ctx: &Ctx, ability: &str, record: Option<&Data>) -> bool {
        let base = crate::policies::child_policy::ChildPolicy("service_domains");
        if !base.allows(ctx, ability, record).await {
            return false;
        }
        // A wildcard, or a domain with its own certificate resolver: changed
        // or removed by the panel's administrators only.
        matches!(ability, "view" | "view_any")
            || !(record
                .and_then(|r| text(r, "host"))
                .is_some_and(|h| h.starts_with("*."))
                || record
                    .is_some_and(|r| is_set(r, "cert_resolver_id") || is_set(r, "certificate_id")))
            || ctx.can(permissions::SYSTEM_MANAGE)
    }

    async fn admits(&self, ctx: &Ctx, ability: &str) -> bool {
        crate::policies::child_policy::ChildPolicy("service_domains")
            .admits(ctx, ability)
            .await
    }

    fn view_depends_on_record(&self) -> bool {
        false
    }
}
