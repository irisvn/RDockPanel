//! The `Volumes` page (`/admin/p/volumes`): volumes: create and remove. Drawn by
//! `resources/admin/js/rdockpanel/volumes.js`, which calls the routes of
//! `routes/admin.rs`.

use rustcms::admin::{Ctx, PageView, PanelPage, PanelPageMeta};
use rustcms::{AppResult, async_trait};
use serde_json::{Value, json};

pub struct VolumesPage;

#[async_trait]
impl PanelPage for VolumesPage {
    const SLUG: &'static str = "volumes";

    fn meta() -> PanelPageMeta {
        PanelPageMeta::new("Volumes")
            .icon("hard-drive")
            .group("Docker")
            .sort(12)
            .permission("docker.view")
    }

    fn view() -> PageView {
        PageView::Script("rdockpanel/volumes")
    }

    /// What the script may offer (the routes check again).
    async fn data(ctx: &Ctx) -> AppResult<Value> {
        Ok(
            json!({ "can": { "docker.manage": ctx.can("docker.manage"), "system.manage": ctx.can("system.manage") } }),
        )
    }
}
