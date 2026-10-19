//! The `Images` page (`/admin/p/images`): images: pull, remove, prune. Drawn by
//! `resources/admin/js/rdockpanel/images.js`, which calls the routes of
//! `routes/admin.rs`.

use rustcms::admin::{Ctx, PageView, PanelPage, PanelPageMeta};
use rustcms::{AppResult, async_trait};
use serde_json::{Value, json};

pub struct ImagesPage;

#[async_trait]
impl PanelPage for ImagesPage {
    const SLUG: &'static str = "images";

    fn meta() -> PanelPageMeta {
        PanelPageMeta::new("Images")
            .icon("image")
            .group("Docker")
            .sort(11)
            .permission("docker.view")
    }

    fn view() -> PageView {
        PageView::Script("rdockpanel/images")
    }

    /// What the script may offer (the routes check again).
    async fn data(ctx: &Ctx) -> AppResult<Value> {
        Ok(
            json!({ "can": { "docker.manage": ctx.can("docker.manage"), "system.manage": ctx.can("system.manage") } }),
        )
    }
}
