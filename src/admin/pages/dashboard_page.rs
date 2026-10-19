//! The `Dashboard` page (`/admin/p/dashboard`): projects and their services,
//! like EasyPanel's, and each service's page by sub-path
//! (`<project>/<service>/<tab>`). A React page (host API v2):
//! `resources/admin/src/rdockpanel/dashboard.tsx`, built to
//! `resources/admin/js/rdockpanel/dashboard.js`. It reads
//! `/admin/api/x/apps/...` and writes through the resources' API.

use rustcms::admin::{Ctx, PageView, PanelPage, PanelPageMeta};
use rustcms::{AppResult, async_trait};
use serde_json::{Value, json};

pub struct DashboardPage;

#[async_trait]
impl PanelPage for DashboardPage {
    const SLUG: &'static str = "dashboard";

    fn meta() -> PanelPageMeta {
        PanelPageMeta::new("Dashboard")
            .icon("dashboard")
            // The server's metrics with `server.view`, the projects with theirs.
            .permission_any(["projects.view_any", "server.view"])
    }

    fn view() -> PageView {
        PageView::Script("rdockpanel/dashboard")
    }

    /// Who the page was opened for: its writes are refused for anyone else
    /// (signed in as another user meanwhile). The routes check again.
    async fn data(ctx: &Ctx) -> AppResult<Value> {
        Ok(json!({
            "user": ctx.principal.id,
            // Shown at its foot: RDockPanel's version and commit, RustCMS Core's.
            "versions": crate::services::system_service::versions(),
        }))
    }
}
