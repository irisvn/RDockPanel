//! Traefik in the apps interface (`/admin/api/x/apps/traefik`, Settings →
//! Certificates): the certificate resolvers, whether Traefik runs them, and
//! "Apply". The panel's administrators' only (`system.manage`): it changes
//! Traefik. The resolvers themselves are written through their resource
//! (`cert_resolvers`).

use rustcms::audit::RequestMeta;
use rustcms::prelude::*;
use rustcms::{AppError, AppResult, Json};
use serde_json::Value;

use crate::models::cert_resolver::{self, DNS_PROVIDERS};
use crate::services::action_service::{self, Initiator};
use crate::services::cert_service::{self, applied, plan, plan_key};
use crate::services::docker_service::Docker;
use crate::services::traefik_service::Traefik;

fn services(app: &AppState) -> AppResult<(std::sync::Arc<Docker>, std::sync::Arc<Traefik>)> {
    let docker = app
        .service::<Docker>()
        .ok_or_else(|| AppError::internal("Docker is not bound"))?;
    let traefik = app
        .service::<Traefik>()
        .ok_or_else(|| AppError::internal("Traefik is not bound"))?;
    Ok((docker, traefik))
}

/// The resolvers (never their tokens), the DNS providers, the default, and
/// whether Traefik runs them (`null`: no Traefik service to manage).
pub async fn resolvers(State(app): State<AppState>) -> AppResult<Json<Value>> {
    use sea_orm::{EntityTrait, QueryOrder};
    let db = app.db();
    let rows = cert_resolver::Entity::find()
        .order_by_asc(cert_resolver::Column::Name)
        .all(db)
        .await?;
    let mut list = Vec::new();
    for r in &rows {
        list.push(json!({
            "id": r.id,
            "name": r.name,
            "kind": r.kind,
            "provider": r.provider,
            "email": r.email,
            "staging": r.staging,
            "has_token": r.token.is_some(),
            "in_use": cert_service::in_use(db, r.id, &r.name).await?,
        }));
    }
    let resolvers = cert_service::resolvers(db, app.keyring())
        .await
        .map_err(AppError::internal)?;
    let plan = plan(&resolvers, &plan_key(app.keyring()));
    let (docker, traefik) = services(&app)?;
    // Docker out of reach: unknown, not an error of the page.
    let applied = applied(&docker, &traefik, &plan).await.ok().flatten();
    Ok(Json(json!({
        "resolvers": list,
        "providers": DNS_PROVIDERS
            .iter()
            .map(|p| json!({ "name": p.name, "label": p.label }))
            .collect::<Vec<_>>(),
        "default": crate::services::deploy_rules_service::config().cert_resolver,
        "applied": applied,
    })))
}

/// Gives Traefik the resolvers (Swarm restarts it: a few seconds without
/// routing). Logged as an action.
pub async fn apply(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
) -> AppResult<Json<Value>> {
    let resolvers = cert_service::resolvers(app.db(), app.keyring())
        .await
        .map_err(|e| rustcms::ValidationErrors::field("resolvers", e))?;
    let plan = plan(&resolvers, &plan_key(app.keyring()));
    let (docker, traefik) = services(&app)?;
    if traefik.service.is_empty() {
        return Err(rustcms::ValidationErrors::field(
            "resolvers",
            "Traefik is not a Swarm service of the panel's",
        )
        .into());
    }
    let by = Initiator::user(auth.principal(), request);
    let changed = action_service::run(
        app.db(),
        &by,
        "traefik.resolvers",
        &traefik.service,
        cert_service::apply(&docker, &traefik, &plan),
    )
    .await?;
    Ok(Json(json!({ "changed": changed, "applied": true })))
}
