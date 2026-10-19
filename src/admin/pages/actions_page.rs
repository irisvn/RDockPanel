//! The `Actions` page (`/admin/p/actions`), like EasyPanel's: what the panel
//! did lately (deployments, backups, Docker actions) with their logs. A React
//! page of the dashboard's bundle: `resources/admin/src/rdockpanel/actions/`.

use rustcms::admin::{Ctx, PageView, PanelPage, PanelPageMeta};
use rustcms::{AppResult, async_trait};
use serde_json::{Value, json};

pub struct ActionsPage;

#[async_trait]
impl PanelPage for ActionsPage {
    const SLUG: &'static str = "actions";

    fn meta() -> PanelPageMeta {
        PanelPageMeta::new("Actions")
            .icon("history")
            .sort(1)
            .permission("projects.view_any")
    }

    fn view() -> PageView {
        PageView::Script("rdockpanel/actions")
    }

    async fn data(ctx: &Ctx) -> AppResult<Value> {
        Ok(json!({ "user": ctx.principal.id }))
    }
}
