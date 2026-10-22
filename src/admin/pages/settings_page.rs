//! The `Settings` page (`/admin/p/settings`), like EasyPanel's: storage
//! providers for backups (notifications next). A React page of the same
//! bundle as the dashboard: `resources/admin/src/rdockpanel/settings/`.

use crate::policies::permissions;
use rustcms::admin::{Ctx, PageView, PanelPage, PanelPageMeta};
use rustcms::{AppResult, async_trait};
use serde_json::{Value, json};

pub struct SettingsPage;

#[async_trait]
impl PanelPage for SettingsPage {
    const SLUG: &'static str = "settings";

    fn meta() -> PanelPageMeta {
        PanelPageMeta::new("Settings")
            .icon("settings")
            .sort(4)
            .permission_any([
                "storage_providers.view_any",
                "notification_channels.view_any",
                permissions::SYSTEM_MANAGE,
                // Their own API tokens (Settings → API tokens).
                "projects.view_any",
                permissions::APPS_ACCESS,
            ])
    }

    fn view() -> PageView {
        PageView::Script("rdockpanel/settings")
    }

    /// Who the page was opened for (writes are bound to them).
    async fn data(ctx: &Ctx) -> AppResult<Value> {
        Ok(json!({ "user": ctx.principal.id }))
    }
}
