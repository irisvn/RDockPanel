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
    // The homepage (public: nothing of this installation on it).
    app.routes(|router| {
        use crate::http::controllers::home_controller as home;
        use rustcms::axum::routing::get;
        router
            .route("/", get(home::page))
            .route("/home/home.css", get(home::css))
            .route("/home/brand.css", get(home::brand))
            .route("/home/favicon.svg", get(home::favicon))
            .route("/home/home.js", get(home::script))
            .route("/home/img/{name}", get(home::image))
    });
    app
}
