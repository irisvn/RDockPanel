//! The `Domains` page (`/admin/p/domains`), like EasyPanel's: every
//! service's domains, where they lead and whether they are served as
//! configured. A React page of the dashboard's bundle:
//! `resources/admin/src/rdockpanel/domains/`.

use rustcms::admin::{Ctx, PageView, PanelPage, PanelPageMeta};
use rustcms::{AppResult, async_trait};
use serde_json::{Value, json};

pub struct DomainsPage;

#[async_trait]
impl PanelPage for DomainsPage {
    const SLUG: &'static str = "domains";

    fn meta() -> PanelPageMeta {
        PanelPageMeta::new("Domains")
            .icon("globe")
            .sort(1)
            // A role's, or a member's (`apps.access`): their projects' domains.
            .permission_any(["service_domains.view_any", "apps.access"])
    }

    fn view() -> PageView {
        PageView::Script("rdockpanel/domains")
    }

    async fn data(ctx: &Ctx) -> AppResult<Value> {
        Ok(json!({ "user": ctx.principal.id }))
    }
}
