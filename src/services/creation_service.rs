//! Creating a service with what it needs, in one transaction: an app from
//! an image (with its environment, domains, mounts, ports), or a database
//! configured for its engine (credentials generated). The apps interface's
//! "+ Service" and EasyPanel templates go through here; the resources'
//! forms remain for editing. Checks are the resources' rules.

use std::collections::BTreeMap;

use chrono::Utc;
use rustcms::crypto::{Encrypted, Keyring};
use rustcms::{AppError, AppResult, ValidationErrors};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseTransaction, EntityTrait, PaginatorTrait, QueryFilter,
    Set,
};

use crate::models::{project, service, service_domain, service_mount, service_port};
use crate::services::database_service::{Credentials, Engine, random_password};
use crate::services::deploy_rules_service::{
    config, valid_container_path, valid_hostname, valid_image_reference, valid_name, valid_port,
    valid_route_path, valid_volume_name,
};
use crate::services::deploy_service::{command_args, valid_env_name};

/// A database's options (each left out: a default, the password generated).
#[derive(Debug, Clone, Default)]
pub struct DatabaseOptions {
    pub database: Option<String>,
    pub user: Option<String>,
    pub password: Option<String>,
    pub root_password: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NewDomain {
    pub host: String,
    pub path: String,
    pub port: u16,
    pub https: bool,
}

#[derive(Debug, Clone)]
pub enum NewMount {
    Volume { name: String, target: String },
    File { content: String, target: String },
}

#[derive(Debug, Clone)]
pub struct NewPort {
    pub published: u16,
    pub target: u16,
    pub protocol: String,
}

/// A service to create.
#[derive(Debug, Clone)]
pub struct NewService {
    pub name: String,
    /// `app` or a database engine (`postgres`, …).
    pub kind: String,
    /// An app's image (required); a database's (its engine's by default).
    pub image: Option<String>,
    pub env: BTreeMap<String, String>,
    pub command: Option<String>,
    pub replicas: i32,
    pub zero_downtime: bool,
    pub domains: Vec<NewDomain>,
    pub mounts: Vec<NewMount>,
    pub ports: Vec<NewPort>,
    pub database: DatabaseOptions,
}

impl NewService {
    /// An app from `image`, with defaults for the rest.
    pub fn app(name: impl Into<String>, image: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: "app".to_owned(),
            image: Some(image.into()),
            env: BTreeMap::new(),
            command: None,
            replicas: 1,
            zero_downtime: true,
            domains: vec![],
            mounts: vec![],
            ports: vec![],
            database: DatabaseOptions::default(),
        }
    }

    /// A database of `engine`, with defaults (credentials generated).
    pub fn database(name: impl Into<String>, engine: Engine) -> Self {
        Self {
            kind: engine.kind().to_owned(),
            image: None,
            zero_downtime: false,
            ..Self::app(name, "")
        }
    }
}

/// Creates `new` in `project` (in `txn`, the caller's). Field errors are
/// a 422 like the resources' forms.
pub async fn create(
    txn: &DatabaseTransaction,
    keyring: &Keyring,
    project: &project::Model,
    new: NewService,
) -> AppResult<service::Model> {
    let mut errors = ValidationErrors::new();
    if !valid_name(&new.name) {
        errors.add(
            "name",
            "must be 1-40 lowercase letters, digits and dashes, starting with a letter",
        );
    } else if service::Entity::find()
        .filter(service::Column::ProjectId.eq(project.id))
        .filter(service::Column::Name.eq(&new.name))
        .count(txn)
        .await?
        > 0
    {
        errors.add("name", "is already taken in this project");
    }
    let engine = match new.kind.as_str() {
        "app" => None,
        kind => match Engine::parse(kind) {
            Some(engine) => Some(engine),
            None => {
                errors.add(
                    "kind",
                    "must be app, postgres, mysql, mariadb, mongo or redis",
                );
                None
            }
        },
    };

    // What the service runs.
    let mut env = new.env.clone();
    let mut command = new.command.clone().filter(|c| !c.trim().is_empty());
    let mut healthcheck: Option<String> = None;
    let mut mounts = new.mounts.clone();
    let image = match engine {
        Some(engine) => {
            let options = &new.database;
            let credentials = Credentials {
                user: options
                    .user
                    .clone()
                    .filter(|u| !u.is_empty())
                    .unwrap_or_else(|| engine.default_user().to_owned()),
                password: options
                    .password
                    .clone()
                    .filter(|p| !p.is_empty())
                    .unwrap_or_else(random_password),
                database: options
                    .database
                    .clone()
                    .filter(|d| !d.is_empty())
                    .unwrap_or_else(|| project.name.clone()),
                root_password: engine.has_root_password().then(|| {
                    options
                        .root_password
                        .clone()
                        .filter(|p| !p.is_empty())
                        .unwrap_or_else(random_password)
                }),
            };
            let setup = engine.setup(new.image.as_deref(), &credentials);
            // The engine's variables; the caller's beside them.
            env.extend(setup.env);
            if command.is_none() {
                command = setup.command;
            }
            healthcheck = Some(setup.healthcheck);
            mounts.insert(
                0,
                NewMount::Volume {
                    name: "data".to_owned(),
                    target: setup.data_path.to_owned(),
                },
            );
            setup.image
        }
        None => new.image.clone().unwrap_or_default().trim().to_owned(),
    };
    if !valid_image_reference(&image) {
        errors.add("image", "is not an image reference (e.g. nginx:1.27)");
    }
    if let Some(command) = &command
        && command_args(command).is_none()
    {
        errors.add("command", "has an unclosed quote");
    }
    if env.keys().any(|name| !valid_env_name(name)) {
        errors.add(
            "env",
            "names must be letters, digits and _, not starting with a digit",
        );
    }
    let replicas = if engine.is_some() {
        1
    } else {
        new.replicas.clamp(0, 100)
    };
    for domain in &new.domains {
        if !valid_hostname(&domain.host) {
            errors.add("domains", format!("{} is not a hostname", domain.host));
        }
        if !valid_route_path(&domain.path) {
            errors.add("domains", format!("{} is not a path", domain.path));
        }
        if !valid_port(i64::from(domain.port)) {
            errors.add("domains", "a port is 1-65535");
        }
    }
    let mut targets = std::collections::BTreeSet::new();
    for mount in &mounts {
        let target = match mount {
            NewMount::Volume { target, .. } | NewMount::File { target, .. } => target,
        };
        if !targets.insert(target.clone()) {
            errors.add("mounts", format!("{target} is mounted twice"));
        }
    }
    for mount in &mounts {
        let (target, ok) = match mount {
            NewMount::Volume { name, target } => (target, valid_volume_name(name)),
            NewMount::File { content, target } => (target, content.len() <= 65_535),
        };
        if !ok || !valid_container_path(target) {
            errors.add("mounts", format!("{target} is not a valid mount"));
        }
    }
    let reserved = &config().reserved_ports;
    for port in &new.ports {
        if !valid_port(i64::from(port.published)) || reserved.contains(&port.published) {
            errors.add("ports", format!("{} cannot be published", port.published));
        }
        if !valid_port(i64::from(port.target)) {
            errors.add("ports", "a container port is 1-65535");
        }
        if !matches!(port.protocol.as_str(), "tcp" | "udp") {
            errors.add("ports", "a protocol is tcp or udp");
        }
    }
    if !errors.is_empty() {
        return Err(errors.into());
    }

    let now = Utc::now();
    let sealed = (!env.is_empty())
        .then(|| Encrypted::seal(keyring, &service::env_context(), &env))
        .transpose()?;
    let row = service::ActiveModel {
        project_id: Set(project.id),
        name: Set(new.name.clone()),
        kind: Set(new.kind.clone()),
        image: Set(image),
        command: Set(command),
        replicas: Set(replicas),
        zero_downtime: Set(new.zero_downtime && engine.is_none()),
        healthcheck_cmd: Set(healthcheck),
        healthcheck_interval_s: Set(engine.map(|_| 5)),
        healthcheck_retries: Set(engine.map(|_| 20)),
        env: Set(sealed),
        desired_state: Set("running".to_owned()),
        status: Set(service::NEW.to_owned()),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(txn)
    .await
    .map_err(|e| taken(e, "name", "is already taken in this project".to_owned()))?;
    for domain in &new.domains {
        service_domain::ActiveModel {
            service_id: Set(row.id),
            host: Set(domain.host.to_lowercase()),
            path: Set(domain.path.clone()),
            port: Set(i32::from(domain.port)),
            https: Set(domain.https),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        }
        .insert(txn)
        .await
        .map_err(|e| taken(e, "domains", format!("{} is already routed", domain.host)))?;
    }
    for mount in &mounts {
        let (kind, name, content, target) = match mount {
            NewMount::Volume { name, target } => ("volume", Some(name.clone()), None, target),
            NewMount::File { content, target } => ("file", None, Some(content.clone()), target),
        };
        service_mount::ActiveModel {
            service_id: Set(row.id),
            kind: Set(kind.to_owned()),
            name: Set(name),
            source: Set(None),
            target: Set(target.clone()),
            read_only: Set(kind == "file"),
            sealed_content: Set(content
                .map(|c| Encrypted::seal(keyring, &service_mount::content_context(), &c))
                .transpose()?),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        }
        .insert(txn)
        .await?;
    }
    for port in &new.ports {
        service_port::ActiveModel {
            service_id: Set(row.id),
            published: Set(i32::from(port.published)),
            target: Set(i32::from(port.target)),
            protocol: Set(port.protocol.clone()),
            mode: Set("ingress".to_owned()),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        }
        .insert(txn)
        .await
        .map_err(|e| {
            taken(
                e,
                "ports",
                format!("{} is already published", port.published),
            )
        })?;
    }
    Ok(row)
}

/// A unique key taken: that field's error; any other failure as it is.
fn taken(error: sea_orm::DbErr, field: &str, message: String) -> AppError {
    match error.sql_err() {
        Some(sea_orm::SqlErr::UniqueConstraintViolation(_)) => {
            ValidationErrors::field(field, message).into()
        }
        _ => error.into(),
    }
}

/// A database service's credentials (its environment, opened).
pub fn credentials_of(
    row: &service::Model,
    env: &BTreeMap<String, String>,
) -> Option<(Engine, Credentials)> {
    let engine = Engine::parse(&row.kind)?;
    Some((engine, engine.credentials_of(env)))
}
