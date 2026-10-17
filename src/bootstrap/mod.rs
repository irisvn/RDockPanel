//! Assembles the application (like Laravel's `bootstrap/app.php`).

mod providers;

use rustcms::prelude::*;

/// The application for `config`: its providers, then its routes.
pub fn build(config: Config) -> AppBuilder {
    let mut app = AppBuilder::new(config);
    let api = crate::routes::api::routes();
    let admin = crate::routes::admin::routes();
    providers::register(&mut app, &api, &admin);
    // Mounted and listed by `rustcms route:list` from the same declarations.
    api.register(&mut app);
    admin.register(&mut app);
    app
}
