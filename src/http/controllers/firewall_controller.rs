//! The Firewall page (`/admin/api/x/apps/firewall/...`): the server's
//! listeners and firewall, the ports opened and the blocked addresses (the
//! policy, saved here), and the changes of the host's table (applied, then
//! kept or reverted). `system.manage` (the routes); every change audited.

use rustcms::audit::{self, AuditActor, AuditContext, AuditEntry, RequestMeta};
use rustcms::prelude::*;
use rustcms::{AppError, AppResult, Json, ValidationErrors};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, Set};
use serde::Deserialize;
use serde_json::Value;

use crate::models::{firewall_blocked, firewall_opening};
use crate::services::action_service::Initiator;
use crate::services::docker_service::Docker;
use crate::services::firewall_rules::{self as rules, Cidr, Protocol};
use crate::services::firewall_service;

/// Installed by install.sh (`[panel] installed`).
fn installed() -> bool {
    crate::services::deploy_rules_service::config().installed
}

fn docker(app: &AppState) -> AppResult<std::sync::Arc<Docker>> {
    app.service::<Docker>()
        .ok_or_else(|| AppError::internal("Docker is not configured"))
}

async fn audited(
    app: &AppState,
    auth: &Auth,
    request: RequestMeta,
    event: &str,
    key: String,
    meta: Value,
) {
    let context = AuditContext::new(AuditActor::user(auth.principal()), request);
    let entry = AuditEntry::new(event, "firewall").key(key).meta(meta);
    audit::record_detached(app.db(), &context, entry).await;
}

/// `GET /firewall`.
pub async fn show(State(app): State<AppState>) -> AppResult<Json<Value>> {
    let docker = docker(&app)?;
    Ok(Json(
        firewall_service::report(&app, &docker, installed()).await?,
    ))
}

/// `GET /firewall/preview`: the table the policy loads, what stays closed.
pub async fn preview(State(app): State<AppState>) -> AppResult<Json<Value>> {
    let docker = docker(&app)?;
    Ok(Json(
        firewall_service::preview(&app, &docker, installed()).await?,
    ))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpeningInput {
    pub ports: String,
    pub protocol: String,
    pub target: String,
    #[serde(default)]
    pub anywhere: bool,
    #[serde(default)]
    pub sources_v4: String,
    #[serde(default)]
    pub sources_v6: String,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

/// The opening `input` describes, checked: its ports, protocol, target and
/// canonical sources (each in its family's field).
fn checked(
    input: &OpeningInput,
) -> Result<((u16, u16), Protocol, String, String), ValidationErrors> {
    let mut errors = ValidationErrors::new();
    let ports = rules::parse_ports(&input.ports)
        .map_err(|e| errors.add("ports", e))
        .ok();
    let protocol = match input.protocol.as_str() {
        "tcp" => Some(Protocol::Tcp),
        "udp" => Some(Protocol::Udp),
        _ => {
            errors.add("protocol", "must be tcp or udp");
            None
        }
    };
    if !matches!(input.target.as_str(), "host" | "published") {
        errors.add("target", "must be host or published");
    }
    let mut sources = vec![];
    for (field, text, v4) in [
        ("sources_v4", &input.sources_v4, true),
        ("sources_v6", &input.sources_v6, false),
    ] {
        let list: Vec<&str> = text
            .split([',', '\n', ' '])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        if list.len() > rules::SOURCES_MAX {
            errors.add(field, format!("at most {} entries", rules::SOURCES_MAX));
        }
        for item in list {
            match Cidr::parse(item) {
                Ok(c) if c.is_v4() == v4 => sources.push(c),
                Ok(_) => errors.add(
                    field,
                    format!("{item} is not an IPv{} network", if v4 { 4 } else { 6 }),
                ),
                Err(e) => errors.add(field, e),
            }
        }
    }
    if !input.anywhere && sources.is_empty() {
        errors.add("sources_v4", "list who may connect, or choose anywhere");
    }
    if input.note.as_ref().is_some_and(|n| n.chars().count() > 200) {
        errors.add("note", "is at most 200 characters");
    }
    if let (Some(ports), Some(protocol)) = (ports, protocol)
        && let Some(p) = rules::overlaps_always_open(ports, protocol, &[22])
    {
        errors.add(
            "ports",
            format!("{p}/tcp is always open (SSH or the web): nothing to open"),
        );
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    sources.sort();
    sources.dedup();
    let list = if input.anywhere {
        String::new()
    } else {
        sources
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",")
    };
    Ok((
        ports.expect("checked"),
        protocol.expect("checked"),
        input.target.clone(),
        list,
    ))
}

/// `POST /firewall/openings`.
pub async fn create_opening(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Json(input): Json<OpeningInput>,
) -> AppResult<Json<Value>> {
    let (ports, protocol, target, sources) = checked(&input)?;
    if firewall_opening::Entity::find().count(app.db()).await? as usize >= rules::OPENINGS_MAX {
        return Err(ValidationErrors::field(
            "ports",
            format!("at most {} openings", rules::OPENINGS_MAX),
        )
        .into());
    }
    let now = chrono::Utc::now();
    let row = firewall_opening::ActiveModel {
        port_start: Set(i32::from(ports.0)),
        port_end: Set(i32::from(ports.1)),
        protocol: Set(protocol.as_str().to_owned()),
        target: Set(target),
        anywhere: Set(input.anywhere),
        sources: Set(sources),
        note: Set(input.note.clone().filter(|n| !n.trim().is_empty())),
        enabled: Set(input.enabled.unwrap_or(true)),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(app.db())
    .await?;
    let value = serde_json::to_value(&row).unwrap_or(Value::Null);
    audited(
        &app,
        &auth,
        request,
        "firewall.opening.create",
        row.id.to_string(),
        value.clone(),
    )
    .await;
    Ok(Json(value))
}

/// `PATCH /firewall/openings/{id}`: the whole opening again.
pub async fn update_opening(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path(id): Path<i64>,
    Json(input): Json<OpeningInput>,
) -> AppResult<Json<Value>> {
    let row = firewall_opening::Entity::find_by_id(id)
        .one(app.db())
        .await?
        .ok_or(AppError::NotFound)?;
    let (ports, protocol, target, sources) = checked(&input)?;
    let enabled = input.enabled.unwrap_or(row.enabled);
    let mut active: firewall_opening::ActiveModel = row.into();
    active.port_start = Set(i32::from(ports.0));
    active.port_end = Set(i32::from(ports.1));
    active.protocol = Set(protocol.as_str().to_owned());
    active.target = Set(target);
    active.anywhere = Set(input.anywhere);
    active.sources = Set(sources);
    active.note = Set(input.note.clone().filter(|n| !n.trim().is_empty()));
    active.enabled = Set(enabled);
    active.updated_at = Set(chrono::Utc::now());
    let row = active.update(app.db()).await?;
    let value = serde_json::to_value(&row).unwrap_or(Value::Null);
    audited(
        &app,
        &auth,
        request,
        "firewall.opening.update",
        id.to_string(),
        value.clone(),
    )
    .await;
    Ok(Json(value))
}

/// `DELETE /firewall/openings/{id}`.
pub async fn delete_opening(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path(id): Path<i64>,
) -> AppResult<Json<Value>> {
    let gone = firewall_opening::Entity::delete_by_id(id)
        .exec(app.db())
        .await?;
    if gone.rows_affected == 0 {
        return Err(AppError::NotFound);
    }
    audited(
        &app,
        &auth,
        request,
        "firewall.opening.delete",
        id.to_string(),
        json!({}),
    )
    .await;
    Ok(Json(json!({ "deleted": id })))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockInput {
    pub cidr: String,
    #[serde(default)]
    pub note: Option<String>,
}

/// `POST /firewall/blocked`.
pub async fn create_blocked(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Json(input): Json<BlockInput>,
) -> AppResult<Json<Value>> {
    let cidr = Cidr::parse(&input.cidr).map_err(|e| ValidationErrors::field("cidr", e))?;
    if cidr.prefix == 0 {
        return Err(ValidationErrors::field("cidr", "would block everyone").into());
    }
    let text = cidr.to_string();
    if firewall_blocked::Entity::find()
        .filter(firewall_blocked::Column::Cidr.eq(&text))
        .one(app.db())
        .await?
        .is_some()
    {
        return Err(ValidationErrors::field("cidr", "is blocked already").into());
    }
    if firewall_blocked::Entity::find().count(app.db()).await? as usize >= rules::BLOCKED_MAX {
        return Err(ValidationErrors::field(
            "cidr",
            format!("at most {} addresses", rules::BLOCKED_MAX),
        )
        .into());
    }
    if input.note.as_ref().is_some_and(|n| n.chars().count() > 200) {
        return Err(ValidationErrors::field("note", "is at most 200 characters").into());
    }
    let row = firewall_blocked::ActiveModel {
        cidr: Set(text),
        note: Set(input.note.filter(|n| !n.trim().is_empty())),
        created_at: Set(chrono::Utc::now()),
        ..Default::default()
    }
    .insert(app.db())
    .await?;
    let value = serde_json::to_value(&row).unwrap_or(Value::Null);
    audited(
        &app,
        &auth,
        request,
        "firewall.blocked.create",
        row.id.to_string(),
        value.clone(),
    )
    .await;
    Ok(Json(value))
}

/// `DELETE /firewall/blocked/{id}`.
pub async fn delete_blocked(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path(id): Path<i64>,
) -> AppResult<Json<Value>> {
    let gone = firewall_blocked::Entity::delete_by_id(id)
        .exec(app.db())
        .await?;
    if gone.rows_affected == 0 {
        return Err(AppError::NotFound);
    }
    audited(
        &app,
        &auth,
        request,
        "firewall.blocked.delete",
        id.to_string(),
        json!({}),
    )
    .await;
    Ok(Json(json!({ "deleted": id })))
}

/// `POST /firewall/apply`: loads the saved policy, to be kept in time.
pub async fn apply(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
) -> AppResult<Json<Value>> {
    let docker = docker(&app)?;
    let by = Initiator::user(auth.principal(), request);
    Ok(Json(
        firewall_service::apply(&app, &docker, &by, installed()).await?,
    ))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tx {
    pub txid: String,
}

fn txid(input: &Tx) -> Result<&str, ValidationErrors> {
    let t = input.txid.as_str();
    if t.is_empty()
        || t.len() > 32
        || !t
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
    {
        return Err(ValidationErrors::field(
            "txid",
            "is not a change of this panel",
        ));
    }
    Ok(t)
}

/// `POST /firewall/keep`.
pub async fn keep(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Json(input): Json<Tx>,
) -> AppResult<Json<Value>> {
    let docker = docker(&app)?;
    let by = Initiator::user(auth.principal(), request);
    Ok(Json(
        firewall_service::keep(&app, &docker, &by, txid(&input)?, installed()).await?,
    ))
}

/// `POST /firewall/revert`.
pub async fn revert(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Json(input): Json<Tx>,
) -> AppResult<Json<Value>> {
    let docker = docker(&app)?;
    let by = Initiator::user(auth.principal(), request);
    Ok(Json(
        firewall_service::revert(&app, &docker, &by, txid(&input)?, installed()).await?,
    ))
}

/// `POST /firewall/disable`.
pub async fn disable(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
) -> AppResult<Json<Value>> {
    let docker = docker(&app)?;
    let by = Initiator::user(auth.principal(), request);
    Ok(Json(
        firewall_service::disable(&app, &docker, &by, installed()).await?,
    ))
}
