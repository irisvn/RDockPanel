//! The application's service providers, in order (like `bootstrap/providers.php`).

use rustcms::prelude::*;

use crate::providers;

pub fn register(app: &mut AppBuilder, api: &ApiRoutes) {
    app.provider(providers::auth_service_provider::provider(api));
    app.provider(providers::admin_panel_provider::provider());
    app.provider(providers::app_service_provider::AppServiceProvider);
}
