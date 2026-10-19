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
        // A service's maintenance page (Traefik routes its domains here).
        .get(
            "/maintenance/{name}",
            Access::public(),
            controllers::trigger_controller::maintenance,
        )
        // GitHub's webhook (auto deploy on push): its signature is the proof.
        .post(
            "/github/{id}",
            Access::public(),
            controllers::trigger_controller::github,
        )
        // The public API: a token of Settings → API tokens, within its
        // scopes and its user's permissions (docs/api.md).
        .get(
            "/me",
            Access::authenticated(),
            controllers::api_v1_controller::me,
        )
        .get(
            "/projects",
            Access::authenticated(),
            controllers::api_v1_controller::projects,
        )
        .get(
            "/services/{id}",
            Access::authenticated(),
            controllers::api_v1_controller::show,
        )
        .post(
            "/services/{id}/deploy",
            Access::authenticated(),
            controllers::api_v1_controller::deploy,
        )
        .post(
            "/services/{id}/restart",
            Access::authenticated(),
            controllers::api_v1_controller::restart,
        )
        .get(
            "/services/{id}/env",
            Access::authenticated(),
            controllers::api_v1_controller::env,
        )
        .get(
            "/services/{id}/domains",
            Access::authenticated(),
            controllers::api_v1_controller::domains,
        )
        .get(
            "/deployments/{id}",
            Access::authenticated(),
            controllers::api_v1_controller::deployment,
        )
    // rustcms:api-routes
}

async fn ping() -> Json<serde_json::Value> {
    Json(json!({ "pong": true }))
}
