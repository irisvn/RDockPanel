//! Traefik in the apps interface (`/admin/api/x/apps/traefik`, Settings →
//! Certificates): the certificate resolvers, whether Traefik runs them, and
//! "Apply". The panel's administrators' only (`system.manage`): it changes
//! Traefik. The resolvers themselves are written through their resource
//! (`cert_resolvers`).

use rustcms::audit::RequestMeta;
use rustcms::auth::{EventStream, LiveSession};
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
    // An HTTP resolver: its challenges answered through the tunnel too.
    crate::services::tunnel_service::sync_panel_route(&app).await;
    Ok(Json(json!({ "changed": changed, "applied": true })))
}

/// The uploaded certificates (never their keys): names covered, expiry, in use.
pub async fn certificates(State(app): State<AppState>) -> AppResult<Json<Value>> {
    use sea_orm::{EntityTrait, QueryOrder};
    let db = app.db();
    let now = chrono::Utc::now();
    let mut list = Vec::new();
    for c in crate::models::certificate::Entity::find()
        .order_by_asc(crate::models::certificate::Column::Name)
        .all(db)
        .await?
    {
        list.push(json!({
            "id": c.id,
            "name": c.name,
            "names": c.name_list(),
            "not_after": c.not_after,
            "days_left": crate::services::certificate_service::days_left(c.not_after, now),
            "fingerprint": c.fingerprint,
            "in_use": crate::services::certificate_service::in_use(db, c.id).await?,
        }));
    }
    Ok(Json(json!({ "certificates": list })))
}

/// An upload: a name, the PEM chain and its key.
#[derive(Debug, serde::Deserialize)]
pub struct Upload {
    #[serde(default)]
    pub name: String,
    pub chain: String,
    pub key: String,
}

/// Reads and checks an upload: (name, parsed).
fn read_upload(
    upload: &Upload,
) -> AppResult<(String, crate::services::certificate_service::Parsed)> {
    let parsed =
        crate::services::certificate_service::parse(&upload.chain, &upload.key, chrono::Utc::now())
            .map_err(|e| {
                let field = if e.contains("key") { "key" } else { "chain" };
                rustcms::ValidationErrors::field(field, e)
            })?;
    let name = upload.name.trim();
    let name = if name.is_empty() {
        parsed.names[0].clone()
    } else if name.chars().count() > 64 {
        return Err(rustcms::ValidationErrors::field("name", "is 64 characters at most").into());
    } else {
        name.to_owned()
    };
    Ok((name, parsed))
}

/// Gives Traefik the certificates (their files), the action logged.
async fn publish(app: &AppState, auth: &Auth, request: RequestMeta, target: &str) -> AppResult<()> {
    let (_, traefik) = services(app)?;
    let by = Initiator::user(auth.principal(), request);
    action_service::run(app.db(), &by, "traefik.certificates", target, async {
        crate::services::certificate_service::sync(app.db(), app.keyring(), &traefik)
            .await
            .map_err(AppError::internal)
    })
    .await
}

/// A certificate's change, recorded in its own transaction (never its key).
async fn audited(
    txn: &sea_orm::DatabaseTransaction,
    auth: &Auth,
    request: RequestMeta,
    event: &str,
    row: &crate::models::certificate::Model,
) -> AppResult<()> {
    use rustcms::audit::{AuditActor, AuditContext, AuditEntry};
    let entry = AuditEntry::new(event, "certificates")
        .key(row.id.to_string())
        .meta(json!({ "name": row.name, "names": row.names, "not_after": row.not_after }));
    let context = AuditContext::new(AuditActor::user(auth.principal()), request);
    rustcms::audit::record(txn, &context, entry).await?;
    Ok(())
}

fn taken(err: sea_orm::DbErr) -> AppError {
    let text = err.to_string().to_ascii_lowercase();
    if text.contains("unique") || text.contains("duplicate") {
        rustcms::ValidationErrors::field("chain", "this certificate is already uploaded").into()
    } else {
        err.into()
    }
}

/// Uploads a certificate (its key sealed); Traefik serves it at once.
pub async fn upload_certificate(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Json(upload): Json<Upload>,
) -> AppResult<Json<Value>> {
    use sea_orm::{ActiveModelTrait, Set};
    let (name, parsed) = read_upload(&upload)?;
    let now = chrono::Utc::now();
    let key = rustcms::crypto::Encrypted::seal(
        app.keyring(),
        &crate::models::certificate::key_context(),
        &parsed.key,
    )
    .map_err(AppError::internal)?;
    let txn = rustcms::db::begin_write(app.db()).await?;
    let row = crate::models::certificate::ActiveModel {
        name: Set(name),
        chain: Set(parsed.chain),
        key: Set(key),
        names: Set(parsed.names.join(",")),
        not_after: Set(parsed.not_after),
        fingerprint: Set(parsed.fingerprint),
        notified_days: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(&txn)
    .await
    .map_err(taken)?;
    audited(&txn, &auth, request.clone(), "certificates.create", &row).await?;
    txn.commit().await?;
    publish(&app, &auth, request, &row.name).await?;
    Ok(Json(
        json!({ "id": row.id, "self_signed": parsed.self_signed }),
    ))
}

/// Replaces a certificate (a renewed one): it must still cover every host
/// served with it (its domains', and deployed routes'). No deployment
/// needed: Traefik serves the new one.
pub async fn replace_certificate(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path(id): Path<i64>,
    Json(upload): Json<Upload>,
) -> AppResult<Json<Value>> {
    use sea_orm::{ActiveModelTrait, IntoActiveModel, Set};
    let (name, parsed) = read_upload(&upload)?;
    let key = rustcms::crypto::Encrypted::seal(
        app.keyring(),
        &crate::models::certificate::key_context(),
        &parsed.key,
    )
    .map_err(AppError::internal)?;
    // A write transaction from the start (SQLite: no read lock to upgrade).
    let txn = rustcms::db::begin_write(app.db()).await?;
    let row = crate::services::certificate_service::locked(&txn, id)
        .await?
        .ok_or(AppError::NotFound)?;
    let names: Vec<&str> = parsed.names.iter().map(String::as_str).collect();
    for host in crate::services::certificate_service::hosts_using(&txn, id).await? {
        if !crate::services::certificate_service::covers(&names, &host) {
            return Err(rustcms::ValidationErrors::field(
                "chain",
                format!("it does not cover {host}, served with this certificate"),
            )
            .into());
        }
    }
    let mut active = row.into_active_model();
    if !upload.name.trim().is_empty() {
        active.name = Set(name);
    }
    active.chain = Set(parsed.chain);
    active.key = Set(key);
    active.names = Set(parsed.names.join(","));
    active.not_after = Set(parsed.not_after);
    active.fingerprint = Set(parsed.fingerprint);
    // Renewed: warned again when this one nears its end.
    active.notified_days = Set(None);
    active.updated_at = Set(chrono::Utc::now());
    let row = active.update(&txn).await.map_err(taken)?;
    audited(&txn, &auth, request.clone(), "certificates.replace", &row).await?;
    txn.commit().await?;
    publish(&app, &auth, request, &row.name).await?;
    Ok(Json(
        json!({ "id": row.id, "self_signed": parsed.self_signed }),
    ))
}

/// Deletes a certificate no domain and no deployed route uses.
pub async fn delete_certificate(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path(id): Path<i64>,
) -> AppResult<Json<Value>> {
    use sea_orm::ModelTrait;
    // A write transaction from the start (SQLite: no read lock to upgrade).
    let txn = rustcms::db::begin_write(app.db()).await?;
    let row = crate::services::certificate_service::locked(&txn, id)
        .await?
        .ok_or(AppError::NotFound)?;
    if crate::services::certificate_service::in_use(&txn, id).await? {
        return Err(rustcms::ValidationErrors::field(
            "certificate",
            "a domain uses it (or a deployed one still does: deploy that service first)",
        )
        .into());
    }
    let name = row.name.clone();
    audited(&txn, &auth, request.clone(), "certificates.delete", &row).await?;
    row.delete(&txn).await?;
    txn.commit().await?;
    publish(&app, &auth, request, &name).await?;
    Ok(Json(json!({ "deleted": true })))
}

/// Traefik's state (whitelisted: image, ports, tasks, last update) and the
/// administrators' middlewares (their YAML as saved).
pub async fn status(State(app): State<AppState>) -> AppResult<Json<Value>> {
    let (docker, traefik) = services(&app)?;
    let state = if traefik.service.is_empty() {
        json!({ "found": false })
    } else {
        crate::services::traefik_settings_service::status(&docker, &traefik).await?
    };
    let middlewares = crate::models::panel_setting::get(
        app.db(),
        crate::services::traefik_settings_service::MIDDLEWARES_KEY,
    )
    .await?
    .unwrap_or_default();
    Ok(Json(
        json!({ "traefik": state, "middlewares": middlewares }),
    ))
}

/// Restarts Traefik (a few seconds without routing). Logged as an action.
pub async fn restart(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
) -> AppResult<Json<Value>> {
    let (docker, traefik) = services(&app)?;
    if traefik.service.is_empty() {
        return Err(rustcms::ValidationErrors::field(
            "traefik",
            "Traefik is not a Swarm service of the panel's",
        )
        .into());
    }
    let by = Initiator::user(auth.principal(), request);
    action_service::run(
        app.db(),
        &by,
        "traefik.restart",
        &traefik.service,
        crate::services::traefik_settings_service::restart(&docker, &traefik),
    )
    .await?;
    Ok(Json(json!({ "restarted": true })))
}

/// The administrators' middlewares, as YAML.
#[derive(Debug, serde::Deserialize)]
pub struct Middlewares {
    #[serde(default)]
    pub yaml: String,
}

/// Saves the administrators' middlewares (checked) and writes them for
/// Traefik (no restart). Logged as an action.
pub async fn save_middlewares(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Json(body): Json<Middlewares>,
) -> AppResult<Json<Value>> {
    let parsed = crate::services::traefik_settings_service::parse_middlewares(&body.yaml)
        .map_err(|e| rustcms::ValidationErrors::field("yaml", e))?;
    crate::models::panel_setting::set(
        app.db(),
        crate::services::traefik_settings_service::MIDDLEWARES_KEY,
        &body.yaml,
    )
    .await?;
    let (_, traefik) = services(&app)?;
    let by = Initiator::user(auth.principal(), request);
    action_service::run(app.db(), &by, "traefik.middlewares", "custom", async {
        crate::services::traefik_settings_service::sync_middlewares(app.db(), &traefik)
            .await
            .map_err(AppError::internal)
    })
    .await?;
    let names: Vec<String> = parsed
        .map(|m| m.keys().map(|k| format!("{k}@file")).collect())
        .unwrap_or_default();
    Ok(Json(json!({ "middlewares": names })))
}

/// Traefik's output as it comes (`lines` events): the last `tail` lines first.
pub async fn logs(
    live: LiveSession,
    State(app): State<AppState>,
    Path(tail): Path<u32>,
) -> AppResult<EventStream> {
    let (docker, traefik) = services(&app)?;
    if traefik.service.is_empty() {
        return Err(AppError::NotFound);
    }
    let query = crate::services::docker_service::LogQuery {
        follow: true,
        tail: Some(tail.min(5_000)),
        since: None,
        timestamps: false,
    };
    let lines = docker.engine().service_logs(&traefik.service, query);
    Ok(live.stream(crate::services::live_service::log_events(lines)))
}

/// The panel's domain, Let's Encrypt email and apps' domain, as the
/// installer set them.
pub async fn panel(State(app): State<AppState>) -> AppResult<Json<Value>> {
    let (docker, traefik) = services(&app)?;
    Ok(Json(
        crate::services::panel_domain_service::current(Some(&docker), &traefik).await,
    ))
}

/// Checks new ones (DNS, a taken name) and gives the installer command
/// applying them: the panel never changes them itself.
pub async fn check_panel(
    State(app): State<AppState>,
    Json(wanted): Json<crate::services::panel_domain_service::Wanted>,
) -> AppResult<Json<Value>> {
    Ok(Json(
        crate::services::panel_domain_service::check(app.db(), &wanted).await?,
    ))
}
