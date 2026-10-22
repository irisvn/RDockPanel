//! The `Networks` page (`/admin/p/networks`): networks: create and remove. Drawn by
//! `resources/admin/src/rdockpanel/docker/VolumesNetworks.tsx` (React), which calls the routes of
//! `routes/admin.rs`.

use crate::policies::permissions;
use rustcms::admin::{Ctx, PageView, PanelPage, PanelPageMeta};
use rustcms::{AppResult, async_trait};
use serde_json::{Value, json};

pub struct NetworksPage;

#[async_trait]
impl PanelPage for NetworksPage {
    const SLUG: &'static str = "networks";

    fn meta() -> PanelPageMeta {
        PanelPageMeta::new("Networks")
            .icon("network")
            .group("Docker")
            .sort(13)
            .permission(permissions::DOCKER_VIEW)
    }

    fn view() -> PageView {
        PageView::Script("rdockpanel/networks")
    }

    /// What the script may offer (the routes check again).
    async fn data(ctx: &Ctx) -> AppResult<Value> {
        Ok(
            json!({ "can": { permissions::DOCKER_MANAGE: ctx.can(permissions::DOCKER_MANAGE), permissions::SYSTEM_MANAGE: ctx.can(permissions::SYSTEM_MANAGE) } }),
        )
    }
}
