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
use crate::services::build_service::{inside, valid_git_url, valid_ref};
use crate::services::database_service::{Credentials, Engine, random_password};
use crate::services::deploy_rules_service::{
    config, valid_container_path, valid_image_reference, valid_name, valid_port, valid_route_path,
    valid_volume_name,
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
    /// A Compose service's container it leads to.
    pub compose_service: Option<String>,
}

/// A redirect of a new service (Traefik's `redirectRegex`).
#[derive(Debug, Clone)]
pub struct NewRedirect {
    pub regex: String,
    pub replacement: String,
    pub permanent: bool,
}

/// A Compose service's repository: its URL, branch, the Compose project's
/// folder and file.
#[derive(Debug, Clone)]
pub struct NewComposeGit {
    pub url: String,
    pub branch: Option<String>,
    pub path: Option<String>,
    pub file: Option<String>,
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

/// How a new app's image is built (instead of pulled).
#[derive(Debug, Clone)]
pub struct NewBuild {
    /// `git` or `dockerfile`.
    pub source: String,
    pub git_url: Option<String>,
    pub git_branch: Option<String>,
    pub git_path: Option<String>,
    /// `dockerfile` (the only builder).
    pub builder: String,
    pub dockerfile_path: Option<String>,
    pub dockerfile: Option<String>,
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
    /// An app built by the panel (its `image` then empty).
    pub build: Option<NewBuild>,
    /// A Compose service's file (`kind` `compose`)…
    pub compose: Option<String>,
    /// …or its repository's.
    pub compose_git: Option<NewComposeGit>,
    /// Linux capabilities added and dropped, and sysctls (`key=value` lines).
    pub cap_add: Vec<String>,
    pub cap_drop: Vec<String>,
    pub sysctls: BTreeMap<String, String>,
    pub groups: Vec<String>,
    /// Its basic authentication: (user, password); hashed when made.
    pub basic_auth: Vec<(String, String)>,
    pub redirects: Vec<NewRedirect>,
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
            build: None,
            compose: None,
            compose_git: None,
            basic_auth: vec![],
            redirects: vec![],
            cap_add: vec![],
            cap_drop: vec![],
            sysctls: BTreeMap::new(),
            groups: vec![],
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
        "app" | "compose" => None,
        kind => match Engine::parse(kind) {
            Some(engine) => Some(engine),
            None => {
                errors.add(
                    "kind",
                    "must be app, compose, postgres, mysql, mariadb, mongo or redis",
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
    if new.kind == "compose" {
        if let Some(text) = new.compose.as_deref().filter(|t| !t.trim().is_empty())
            && let Err(e) =
                crate::services::compose_service::prepare(text, &project.name, &new.name)
        {
            errors.add("compose", e);
        }
        if new.build.is_some() {
            errors.add(
                "source",
                "a Compose service's repository is set on its Source tab",
            );
        }
        if let Some(git) = &new.compose_git {
            if !valid_git_url(&git.url) {
                errors.add("git_url", "must be an https or ssh repository URL");
            }
            if git
                .branch
                .as_deref()
                .is_some_and(|b| !b.is_empty() && !valid_ref(b))
            {
                errors.add("git_branch", "is not a branch, tag or commit");
            }
            for path in [&git.path, &git.file].into_iter().flatten() {
                if inside(path).is_none() {
                    errors.add("git_path", "must stay inside the repository");
                }
            }
        }
        for domain in &new.domains {
            if !domain
                .compose_service
                .as_deref()
                .is_some_and(crate::services::deploy_service::valid_compose_service)
            {
                errors.add(
                    "domains",
                    format!("{} names no Compose service", domain.host),
                );
            }
        }
    }
    match &new.build {
        None if new.kind == "compose" => {}
        None if !valid_image_reference(&image) => {
            errors.add("image", "is not an image reference (e.g. nginx:1.27)");
        }
        Some(_) if engine.is_some() => {
            errors.add("source", "a database runs its engine's image");
        }
        Some(build) => match build.source.as_str() {
            "git" => {
                if !build.git_url.as_deref().is_some_and(valid_git_url) {
                    errors.add("git_url", "must be an https or ssh repository URL");
                }
                if build
                    .git_branch
                    .as_deref()
                    .is_some_and(|b| !b.is_empty() && !valid_ref(b))
                {
                    errors.add("git_branch", "is not a branch, tag or commit");
                }
                for path in [&build.git_path, &build.dockerfile_path]
                    .into_iter()
                    .flatten()
                {
                    if inside(path).is_none() {
                        errors.add("git_path", "must stay inside the repository");
                    }
                }
                if build.builder != "dockerfile" {
                    errors.add("builder", "must be dockerfile");
                }
            }
            "dockerfile" => {
                if !build
                    .dockerfile
                    .as_deref()
                    .is_some_and(|d| !d.trim().is_empty() && d.len() <= 65_535)
                {
                    errors.add("dockerfile", "is required (at most 64 KB)");
                }
            }
            _ => errors.add("source", "must be image, git or dockerfile"),
        },
        None => {}
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
        if !crate::services::deploy_rules_service::valid_domain_host(&domain.host) {
            errors.add("domains", format!("{} is not a hostname", domain.host));
        }
        if crate::services::deploy_rules_service::is_panel_host(&domain.host) {
            errors.add(
                "domains",
                format!("{} is the panel's own domain", domain.host),
            );
        }
        if !valid_route_path(&domain.path) {
            errors.add("domains", format!("{} is not a path", domain.path));
        }
        if !valid_port(i64::from(domain.port)) {
            errors.add("domains", "a port is 1-65535");
        }
        if domain.https
            && domain.host.starts_with("*.")
            && !crate::services::deploy_rules_service::config()
                .cert_resolver
                .is_empty()
        {
            errors.add(
                "domains",
                format!("{}: a wildcard's certificate needs a DNS resolver (choose one on the domain after)", domain.host),
            );
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
    for caps in [&new.cap_add, &new.cap_drop] {
        if !crate::services::deploy_service::valid_capabilities(&caps.join(",")) {
            errors.add("capabilities", "must be capability names like NET_ADMIN");
        }
    }
    if !new
        .groups
        .iter()
        .all(|g| crate::services::deploy_service::valid_group(g))
    {
        errors.add("groups", "must be group names or ids");
    }
    let lines: Vec<String> = new
        .sysctls
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect();
    if crate::services::deploy_service::sysctls(Some(&lines.join("\n"))).is_none() {
        errors.add("sysctls", "must be key=value pairs");
    }
    for (user, password) in &new.basic_auth {
        if !crate::services::deploy_service::valid_auth_user(user) {
            errors.add("basic_auth", format!("{user:?} is not a user name"));
        }
        if password.is_empty() || password.len() > 72 {
            errors.add(
                "basic_auth",
                format!("{user}: a password is 1-72 characters"),
            );
        }
    }
    for redirect in &new.redirects {
        if let Err(e) =
            crate::services::deploy_service::check_redirect(&redirect.regex, &redirect.replacement)
        {
            errors.add("redirects", format!("{}: {e}", redirect.regex));
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
        image: Set(if new.build.is_some() {
            String::new()
        } else {
            image
        }),
        source: Set(match (&new.build, &new.compose_git) {
            (Some(b), _) => b.source.clone(),
            (None, Some(_)) => "git".to_owned(),
            (None, None) => "image".to_owned(),
        }),
        git_url: Set(new
            .build
            .as_ref()
            .and_then(|b| b.git_url.clone())
            .or_else(|| new.compose_git.as_ref().map(|g| g.url.clone()))),
        git_branch: Set(new
            .build
            .as_ref()
            .and_then(|b| b.git_branch.clone())
            .or_else(|| new.compose_git.as_ref().and_then(|g| g.branch.clone()))),
        git_path: Set(new
            .build
            .as_ref()
            .and_then(|b| b.git_path.clone())
            .or_else(|| new.compose_git.as_ref().and_then(|g| g.path.clone()))),
        builder: Set(new
            .build
            .as_ref()
            .map_or_else(|| "dockerfile".to_owned(), |b| b.builder.clone())),
        dockerfile_path: Set(new.build.as_ref().and_then(|b| b.dockerfile_path.clone())),
        dockerfile: Set(new.build.as_ref().and_then(|b| b.dockerfile.clone())),
        compose: Set(new.compose.clone().filter(|t| !t.trim().is_empty())),
        cap_add: Set((!new.cap_add.is_empty()).then(|| new.cap_add.join(", "))),
        cap_drop: Set((!new.cap_drop.is_empty()).then(|| new.cap_drop.join(", "))),
        groups: Set((!new.groups.is_empty()).then(|| new.groups.join(", "))),
        sysctls: Set((!new.sysctls.is_empty()).then(|| {
            new.sysctls
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("\n")
        })),
        compose_file: Set(new.compose_git.as_ref().and_then(|g| g.file.clone())),
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
        // One certificate per host: the host's other HTTPS domains use the default too.
        if let Some((_, message)) = crate::services::cert_service::check_domain_tls(
            txn,
            None,
            &domain.host.to_lowercase(),
            domain.https,
            None,
            None,
        )
        .await?
        {
            return Err(rustcms::ValidationErrors::field(
                "domains",
                format!("{}: {message}", domain.host),
            )
            .into());
        }
        service_domain::ActiveModel {
            service_id: Set(row.id),
            host: Set(domain.host.to_lowercase()),
            path: Set(domain.path.clone()),
            port: Set(i32::from(domain.port)),
            https: Set(domain.https),
            compose_service: Set(domain.compose_service.clone()),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        }
        .insert(txn)
        .await
        .map_err(|e| taken(e, "domains", format!("{} is already routed", domain.host)))?;
    }
    for (user, password) in &new.basic_auth {
        let hash = bcrypt::hash(password, 10).map_err(AppError::internal)?;
        crate::models::service_basic_auth::ActiveModel {
            service_id: Set(row.id),
            username: Set(user.clone()),
            password_hash: Set(hash),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        }
        .insert(txn)
        .await
        .map_err(|e| taken(e, "basic_auth", format!("{user} is given twice")))?;
    }
    for redirect in &new.redirects {
        crate::models::service_redirect::ActiveModel {
            service_id: Set(row.id),
            regex: Set(redirect.regex.clone()),
            replacement: Set(redirect.replacement.clone()),
            permanent: Set(redirect.permanent),
            enabled: Set(true),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        }
        .insert(txn)
        .await?;
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
