//! Deployments: read only, and rolled back where the service may be
//! deployed.

use rustcms::admin::*;
use rustcms::async_trait;

use super::{admits, may, parent_ok};
use crate::admin::int;
use crate::policies::permissions;
use crate::services::deployment_service::SUCCEEDED;

/// A deployment that succeeded running a recorded image (not a stop or a
/// destroy).
fn can_return_to(record: &Data) -> bool {
    let field = |name: &str| record.get(name).and_then(|v| v.as_str());
    field("status") == Some(SUCCEEDED)
        && field("image_digest").is_some_and(|image| !image.is_empty())
        && !matches!(field("trigger"), Some("stop" | "destroy"))
}

/// Whether a deployment's snapshot adds capabilities or sysctls.
fn privileged(record: &Data) -> bool {
    let spec = match record.get("spec") {
        Some(serde_json::Value::String(text)) => {
            serde_json::from_str(text).unwrap_or(serde_json::Value::Null)
        }
        Some(value) => value.clone(),
        None => return true,
    };
    let set = |field: &str| match &spec[field] {
        serde_json::Value::Array(a) => !a.is_empty(),
        serde_json::Value::Object(o) => !o.is_empty(),
        _ => false,
    };
    set("cap_add") || set("sysctls") || set("groups")
}

/// Viewing needs the permissions; nobody writes through the panel; rolling
/// back needs `services.deploy`, to a deployment that succeeded running a
/// recorded image (not a stop or a destroy).
pub struct DeploymentPolicy;

#[async_trait]
impl Policy for DeploymentPolicy {
    async fn allows(&self, ctx: &Ctx, ability: &str, record: Option<&Data>) -> bool {
        // With a record: in its service's project; without: anywhere.
        let can = |permission: &'static str| async move {
            match record {
                Some(_) => may(ctx, permission, record).await,
                None => admits(ctx, permission).await,
            }
        };
        match ability {
            "view_any" => can("deployments.view_any").await,
            "view" => can("deployments.view").await,
            "rollback" => {
                can("deployments.view").await
                    && can(permissions::SERVICES_DEPLOY).await
                    && record.is_none_or(can_return_to)
                    // Back to added capabilities or sysctls: as setting them.
                    && (record.is_none_or(|r| !privileged(r))
                        || ctx.can(permissions::SYSTEM_MANAGE))
                    // Nor while the service is privileged now.
                    && match record.and_then(|r| int(r, "service_id")) {
                        Some(id) => parent_ok(ctx, &ctx.conn(), id).await,
                        None => true,
                    }
            }
            _ => false,
        }
    }

    async fn admits(&self, ctx: &Ctx, ability: &str) -> bool {
        self.allows(ctx, ability, None).await
    }

    fn view_depends_on_record(&self) -> bool {
        false
    }
}
