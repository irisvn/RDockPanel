//! The JSON API under `/api/v1`, authenticated with API tokens
//! (`Authorization: Bearer rcms_…`, see `rustcms token:create`). Every route
//! declares its access; `rustcms make:api <Model>` adds controller routes here.

use rustcms::prelude::*;

#[allow(unused_imports)]
use crate::http::controllers;

/// Every API route, with the access it requires.
pub fn routes() -> ApiRoutes {
    ApiRoutes::new("/api/v1")
        .get("/ping", Access::public(), ping)
        // A service's deployment trigger: its token is the secret.
        .post(
            "/deploy/{id}",
            Access::public(),
            controllers::trigger_controller::deploy,
        )
    // rustcms:api-routes
}

async fn ping() -> Json<serde_json::Value> {
    Json(json!({ "pong": true }))
}
