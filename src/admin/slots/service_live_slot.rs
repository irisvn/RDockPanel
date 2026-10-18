//! Beside a service: its state, whether its configuration changed since the
//! last deployment, and its latest deployment (followed live by the script
//! `resources/admin/js/rdockpanel/service-live.js`).

use rustcms::admin::{Ctx, Envelope, PageView, PanelSlot, Placement, SlotMeta};
use rustcms::{AppError, AppResult, async_trait};
use sea_orm::EntityTrait;
use serde_json::{Value, json};

use crate::models::service;
use crate::services::deploy_service::Snapshot;
use crate::services::deployment_service::{
    has_pending_changes, latest, snapshot_without_env, summary,
};

pub struct ServiceLiveSlot;

#[async_trait]
impl PanelSlot for ServiceLiveSlot {
    const NAME: &'static str = "service-live";

    fn placement() -> Placement {
        Placement::RecordAside("services")
    }

    fn meta() -> SlotMeta {
        SlotMeta::new()
            .label("Deployment")
            .permission("services.view")
    }

    fn view() -> PageView {
        PageView::Script("rdockpanel/service-live")
    }

    /// The service is one the user may view (the panel checked).
    async fn data(ctx: &Ctx, record: &Envelope) -> AppResult<Value> {
        let id: i64 = record.key.parse().map_err(|_| AppError::NotFound)?;
        let row = service::Entity::find_by_id(id)
            .one(&ctx.db)
            .await?
            .ok_or(AppError::NotFound)?;
        let snapshot: Snapshot = snapshot_without_env(&ctx.db, &row).await?;
        // The deployment (its log, error, image) needs its own permission.
        let latest = if ctx.can("deployments.view") {
            latest(&ctx.db, id).await?
        } else {
            None
        };
        Ok(json!({
            "id": row.id.to_string(),
            "service": row.name,
            "swarm_name": snapshot.service_name(),
            "status": row.status,
            "desired_state": row.desired_state,
            "deployed_at": row.deployed_at,
            "busy": row.current_deployment_id.is_some(),
            "pending": has_pending_changes(&ctx.db, &row, ctx.keyring()?).await?,
            "zero_downtime": row.zero_downtime,
            "zero_downtime_possible": snapshot.zero_downtime_possible(),
            "latest": latest.as_ref().map(summary),
            "can": {
                "deployments": ctx.can("deployments.view"),
                "deploy": ctx.can("services.deploy"),
                "destroy": ctx.can("services.destroy"),
            },
        }))
    }
}
