//! Assembles the application (like Laravel's `bootstrap/app.php`).

mod providers;

use rustcms::prelude::*;

/// The application for `config`: its providers, then its API routes.
pub fn build(config: Config) -> AppBuilder {
    let mut app = AppBuilder::new(config);
    let api = crate::routes::api::routes();
    providers::register(&mut app, &api);
    // Mounted and listed by `rustcms route:list` from the same declarations.
    api.register(&mut app);
    app
}
