//! Authentication and permissions: admin sessions under `/admin`, API tokens
//! elsewhere. Declare permissions beyond each resource's own here.

use rustcms::prelude::*;

pub fn provider(api: &ApiRoutes) -> AuthProvider {
    // The API's permissions come from its routes.
    AuthProvider::new().api_permissions(api)
}
