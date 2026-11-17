//! The `Nodes` page (`/admin/p/nodes`): the Swarm's servers: listed, joined,
//! paused, drained, activated and removed. Drawn by
//! `resources/admin/src/rdockpanel/docker/Nodes.tsx` (React), which calls the routes of
//! `routes/admin.rs` (`/docker/nodes`).

use crate::policies::permissions;
use rustcms::admin::{Ctx, PageView, PanelPage, PanelPageMeta};
use rustcms::{AppResult, async_trait};
use serde_json::{Value, json};

pub struct NodesPage;

#[async_trait]
impl PanelPage for NodesPage {
    const SLUG: &'static str = "nodes";

    fn meta() -> PanelPageMeta {
        PanelPageMeta::new("Nodes")
            .icon("server")
            .group("Docker")
            .sort(14)
            .permission(permissions::DOCKER_VIEW)
    }

    fn view() -> PageView {
        PageView::Script("rdockpanel/nodes")
    }

    /// What the script may offer (the routes check again).
    async fn data(ctx: &Ctx) -> AppResult<Value> {
        Ok(json!({ "can": { permissions::SYSTEM_MANAGE: ctx.can(permissions::SYSTEM_MANAGE) } }))
    }
}
