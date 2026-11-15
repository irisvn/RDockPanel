//! Importing existing containers: reading them and writing the plan
//! (docs/import.md). The state machine that stops, copies and
//! deploys comes with the cut-over; here an import is `discovering`, then
//! `planned` (or `discover_failed`).

use std::collections::BTreeMap;

use rustcms::{AppError, AppResult, AppState};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, PaginatorTrait,
    QueryFilter, QuerySelect, Set,
};
use serde_json::{Value, json};

use crate::models::{container_import, container_import_item, container_import_volume};
use crate::services::deploy_rules_service::config;
use crate::services::docker_service::Docker;
use crate::services::import_compose_service as compose;
use crate::services::import_graph_service::{self, Member, PanelFacts};
use crate::services::import_plan_service::{self, Detail};

/// The states in which an import is running: it holds its services and the
/// panel does not update.
pub const RUNNING: [&str; 11] = [
    "queued",
    "preparing",
    "stopping",
    "draining",
    "copying",
    "editing_compose",
    "deploying",
    "observing",
    "undoing",
    "draining_panel",
    "restoring",
];

/// Refuses what would touch a service of an import that runs or waits for
/// the admin (`needs_attention`): a deployment, a backup, a restore. The
/// import's own steps pass `by_import`. Called in the write that would act.
pub async fn refuse_while_importing<C: ConnectionTrait>(
    txn: &C,
    service_id: i64,
    by_import: Option<i64>,
) -> AppResult<()> {
    let items = container_import_item::Entity::find()
        .filter(container_import_item::Column::ServiceId.eq(service_id))
        .all(txn)
        .await?;
    for item in items {
        if Some(item.import_id) == by_import {
            continue;
        }
        let import = container_import::Entity::find_by_id(item.import_id)
            .lock_shared()
            .one(txn)
            .await?;
        if import
            .is_some_and(|i| i.state == "needs_attention" || RUNNING.contains(&i.state.as_str()))
        {
            return Err(AppError::Conflict {
                code: "import_in_progress",
                reason: "importing",
                message: "This service is being imported: wait for the import, or undo it",
            });
        }
    }
    Ok(())
}

/// Refuses an update of the panel while an import runs.
pub async fn refuse_updating_while_importing<C: ConnectionTrait>(txn: &C) -> AppResult<()> {
    let running = container_import::Entity::find()
        .filter(container_import::Column::State.is_in(RUNNING))
        .count(txn)
        .await?;
    if running > 0 {
        return Err(AppError::Conflict {
            code: "import_in_progress",
            reason: "importing",
            message: "Containers are being imported: update the panel when it is done",
        });
    }
    Ok(())
}

/// An import holds its originals (stopped, to undo to) and the volumes it
/// copied until it is `finished` or `reverted`.
async fn holding<C: ConnectionTrait>(txn: &C, import_id: i64) -> AppResult<bool> {
    Ok(container_import::Entity::find_by_id(import_id)
        .one(txn)
        .await?
        .is_some_and(|i| !matches!(i.state.as_str(), "finished" | "reverted")))
}

fn conflict() -> AppError {
    AppError::Conflict {
        code: "import_in_progress",
        reason: "importing",
        message: "An import holds this: finish or undo the import first",
    }
}

/// Refuses a start, stop or removal of a container an import holds
/// (`reference` as the caller wrote it: an id, a short id or a name).
pub async fn guard_container<C: ConnectionTrait>(txn: &C, reference: &str) -> AppResult<()> {
    let items = container_import_item::Entity::find().all(txn).await?;
    for item in items.iter().filter(|i| {
        i.container_id == reference
            || i.container_name == reference
            || (reference.len() >= 4 && i.container_id.starts_with(reference))
    }) {
        if holding(txn, item.import_id).await? {
            return Err(conflict());
        }
    }
    Ok(())
}

/// Refuses the removal of a volume an import copied from or to, until the
/// admin releases it.
pub async fn guard_volume<C: ConnectionTrait>(txn: &C, name: &str) -> AppResult<()> {
    let volumes = container_import_volume::Entity::find()
        .filter(container_import_volume::Column::ReleasedAt.is_null())
        .all(txn)
        .await?;
    for volume in volumes
        .iter()
        .filter(|v| v.original_name == name || v.panel_volume.as_deref() == Some(name))
    {
        let state = match volume.import_id {
            Some(id) => container_import::Entity::find_by_id(id)
                .one(txn)
                .await?
                .map(|i| i.state),
            // Its import is deleted: the volume still holds data.
            None => Some("finished".to_owned()),
        };
        let kept = match state.as_deref() {
            // Undone: the original is the user's again, and the copy theirs to keep or remove.
            Some("reverted") => false,
            // The originals' volumes stay until released; the copies serve the
            // services while the import is not over.
            Some("finished") => volume.original_name == name,
            Some(_) | None => true,
        };
        if kept {
            return Err(conflict());
        }
    }
    Ok(())
}

/// Refuses a prune of containers while an import holds stopped originals.
pub async fn guard_prune_containers<C: ConnectionTrait>(txn: &C) -> AppResult<()> {
    let items = container_import_item::Entity::find().all(txn).await?;
    for item in &items {
        if holding(txn, item.import_id).await? {
            return Err(conflict());
        }
    }
    Ok(())
}

/// What marks the panel's own database: the loopback ports of its URL, and
/// the working directory of a panel that runs from source.
/// Marks the plan's domains the panel serves already (same host and path, any
/// service): skipped, and a finding to confirm on their container. Run when
/// the plan is read and when it changes.
pub async fn mark_taken_domains<C: ConnectionTrait>(db: &C, plan: &mut Value) -> AppResult<()> {
    use crate::models::service_domain;
    let hosts: Vec<String> = plan["items"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|i| i["domains"].as_array().into_iter().flatten())
        .filter_map(|d| d["host"].as_str().map(str::to_owned))
        .collect();
    if hosts.is_empty() {
        return Ok(());
    }
    let served: Vec<(String, String)> = service_domain::Entity::find()
        .filter(service_domain::Column::Host.is_in(hosts))
        .all(db)
        .await?
        .into_iter()
        .map(|d| (d.host, d.path))
        .collect();
    for item in plan["items"].as_array_mut().into_iter().flatten() {
        let mut taken = Vec::new();
        for hint in item["domains"].as_array_mut().into_iter().flatten() {
            let (host, path) = (hint["host"].clone(), hint["path"].clone());
            if served.iter().any(|(h, p)| host == *h && path == *p) {
                hint["skip"] = json!(true);
                taken.push(host);
            }
        }
        let Some(findings) = item["findings"].as_array_mut() else {
            continue;
        };
        findings.retain(|f| f["code"] != "domain_taken");
        for host in taken {
            findings.push(json!({
                "code": "domain_taken",
                "level": "CONFIRM",
                "field": host,
                "text": "the panel already serves this domain on another service: it is not added",
                "how_to_unblock": "Confirm, or remove it from the other service and read again.",
            }));
        }
    }
    Ok(())
}

pub fn panel_facts() -> PanelFacts {
    let database_ports = ["DATABASE_URL", "RUSTCMS_TEST_MYSQL"]
        .iter()
        .filter_map(|name| std::env::var(name).ok())
        .filter_map(|url| port_of(&url))
        .collect();
    let working_dir = (!config().installed)
        .then(|| std::env::current_dir().ok())
        .flatten()
        .map(|p| p.to_string_lossy().into_owned());
    PanelFacts {
        database_ports,
        working_dir,
    }
}

/// The port of a `scheme://[user:pass@]host[:port]/db` URL on this machine.
fn port_of(url: &str) -> Option<u16> {
    let authority = url.split("://").nth(1)?.split('/').next()?;
    let host_port = authority.rsplit('@').next()?;
    let (host, port) = host_port.rsplit_once(':')?;
    matches!(host, "127.0.0.1" | "localhost" | "[::1]")
        .then(|| port.parse().ok())
        .flatten()
}

/// How an import runs. Bound as a service so that tests can bind other
/// timings and a helper image of their own.
#[derive(Clone, Debug)]
pub struct ImportSettings {
    /// The image of the helper containers (one with `cp` and `sh`): `[panel]
    /// helper_image`, else the panel's own (`None`).
    pub helper_image: Option<String>,
    /// How long the new services are watched before the import is `active`.
    pub observe: std::time::Duration,
    /// The longest a wait takes (the originals gone, the old tasks gone).
    pub deadline: std::time::Duration,
    /// Between two looks at Docker.
    pub poll: std::time::Duration,
    /// Where the symlinks of imported folders are made (a host path Docker
    /// can bind): the installer's folder on a Mac, `/var/lib/rdockpanel/links`
    /// on Linux, unless set.
    pub links_dir: Option<std::path::PathBuf>,
    /// How long a step holds the import before another may take it over; a
    /// long wait renews it (a third of it apart).
    pub lease: std::time::Duration,
}

impl Default for ImportSettings {
    fn default() -> Self {
        Self {
            helper_image: None,
            observe: std::time::Duration::from_secs(60),
            deadline: std::time::Duration::from_secs(120),
            poll: std::time::Duration::from_secs(1),
            links_dir: None,
            lease: std::time::Duration::from_secs(90),
        }
    }
}

impl ImportSettings {
    pub fn from_config() -> Self {
        let configured = &config().helper_image;
        Self {
            helper_image: (!configured.is_empty()).then(|| configured.clone()),
            ..Self::default()
        }
    }

    /// Where the symlinks of imported folders are made.
    pub fn links(&self) -> std::path::PathBuf {
        self.links_dir
            .clone()
            .unwrap_or_else(|| match config().host_os {
                crate::services::deploy_rules_service::HostOs::Macos => {
                    config().state_dir.join("links")
                }
                crate::services::deploy_rules_service::HostOs::Linux => {
                    std::path::PathBuf::from("/var/lib/rdockpanel/links")
                }
            })
    }
}

pub fn settings(state: &AppState) -> ImportSettings {
    state
        .service::<ImportSettings>()
        .map(|s| (*s).clone())
        .unwrap_or_default()
}

/// The image of the helper containers (one with `cp` and `sh`): `[panel]
/// helper_image`, else the panel's own.
pub async fn helper_image(state: &AppState, docker: &Docker) -> Option<String> {
    if let Some(image) = settings(state).helper_image {
        return Some(image);
    }
    crate::services::system_service::own_image(docker)
        .await
        .ok()
        .flatten()
        .map(|(reference, _)| reference)
}

fn docker(state: &AppState) -> AppResult<std::sync::Arc<Docker>> {
    state
        .service::<Docker>()
        .ok_or_else(|| AppError::internal("Docker is not configured"))
}

/// Reads the containers `import_id` is about and stores its plan. A failure
/// is the import's `discover_failed` state, with Docker's own message.
pub async fn discover(state: &AppState, import_id: i64) -> AppResult<()> {
    let row = container_import::Entity::find_by_id(import_id)
        .one(state.db())
        .await?
        .ok_or(AppError::NotFound)?;
    let outcome = read(state, &row).await;
    let mut done = row.into_active_model();
    match outcome {
        Ok((plan, fingerprint)) => {
            done.plan = Set(Some(plan.to_string()));
            done.fingerprint = Set(Some(fingerprint));
            done.state = Set("planned".to_owned());
            done.error = Set(None);
        }
        Err(message) => {
            done.state = Set("discover_failed".to_owned());
            done.error = Set(Some(message));
        }
    }
    done.updated_at = Set(chrono::Utc::now());
    done.update(state.db()).await?;
    Ok(())
}

/// What Docker says about the containers an import is about, read now.
pub struct Gathered {
    /// The raw inspects of the members (their environments are in them: they
    /// stay on the server).
    pub docs: BTreeMap<String, Value>,
    /// Every container of the host the graph looked at.
    pub members: Vec<Member>,
    pub closure: import_graph_service::Closure,
    pub details: Vec<Detail>,
    pub fingerprint: String,
}

async fn gather(state: &AppState, row: &container_import::Model) -> Result<Gathered, String> {
    let docker = docker(state).map_err(|e| e.to_string())?;
    let engine = docker.engine();
    let listed = engine
        .containers_detailed()
        .await
        .map_err(|e| e.to_string())?;
    let mut all_docs: BTreeMap<String, Value> = BTreeMap::new();
    for item in &listed {
        let system = item["Labels"]["rdockpanel.system"] == "true";
        let Some(id) = item["Id"].as_str().filter(|_| !system) else {
            continue;
        };
        if let Ok(doc) = engine.inspect_container_raw(id, false).await {
            all_docs.insert(id.to_owned(), doc);
        }
    }
    let members: Vec<Member> = all_docs.values().map(Member::from_inspect).collect();
    let seeds: Vec<String> = members
        .iter()
        .filter(|m| match row.source_kind.as_str() {
            "project" => m.compose_project() == Some(row.source_ref.as_str()),
            _ => m.id == row.source_ref || m.name == row.source_ref,
        })
        .map(|m| m.id.clone())
        .collect();
    if seeds.is_empty() {
        return Err("no such container or Compose project".to_owned());
    }
    let closure = import_graph_service::closure(&members, &seeds);
    let mut details = Vec::new();
    let mut docs = BTreeMap::new();
    for id in &closure.members {
        let Some(doc) = all_docs.get(id) else {
            continue;
        };
        let image_doc = engine
            .image_config(&image_of(doc))
            .await
            .unwrap_or(Value::Null);
        // The size is slow: asked for the members only.
        let sized = engine
            .inspect_container_raw(id, true)
            .await
            .unwrap_or_else(|_| doc.clone());
        let mut detail = Detail::from_inspect(&sized, &image_doc);
        for mount in detail.member.mounts.iter().filter(|m| m.kind == "volume") {
            if let Ok(volume) = engine.inspect_volume(&mount.source).await {
                let options = volume["Options"]
                    .as_object()
                    .is_some_and(|options| !options.is_empty());
                detail.volume_drivers.insert(
                    mount.source.clone(),
                    (
                        volume["Driver"].as_str().unwrap_or("local").to_owned(),
                        options,
                    ),
                );
            }
        }
        docs.insert(id.clone(), doc.clone());
        details.push(detail);
    }
    let key = state.keyring().current().derive("rdockpanel.import");
    let taken: Vec<Value> = docs.values().cloned().collect();
    let fingerprint = import_plan_service::fingerprint(&taken, &key);
    Ok(Gathered {
        docs,
        members,
        closure,
        details,
        fingerprint,
    })
}

/// The plan (as JSON) and its fingerprint.
async fn read(state: &AppState, row: &container_import::Model) -> Result<(Value, String), String> {
    let gathered = gather(state, row).await?;
    let docker = docker(state).map_err(|e| e.to_string())?;
    let compose_info =
        compose_of(state, &docker, &gathered.members, &gathered.closure.members).await;
    let plan = import_plan_service::plan_of(
        &gathered.details,
        &gathered.closure,
        &row.source_ref,
        &row.project_name,
        &panel_facts(),
        &gathered.fingerprint,
    );
    let mut plan = serde_json::to_value(&plan).map_err(|e| e.to_string())?;
    plan["compose"] = compose_info;
    mark_taken_domains(state.db(), &mut plan)
        .await
        .map_err(|e| e.to_string())?;
    Ok((plan, gathered.fingerprint))
}

/// What the Compose files of the members' projects say. Read through a
/// helper that never starts; a file that cannot be read is said in `notes`
/// and the plan stands on what Docker told.
async fn compose_of(
    state: &AppState,
    docker: &Docker,
    members: &[Member],
    taken: &[String],
) -> Value {
    use sha2::{Digest, Sha256};
    let mut projects: Vec<(&str, &Member)> = Vec::new();
    for member in members.iter().filter(|m| taken.contains(&m.id)) {
        if let Some(project) = member.compose_project()
            && !projects.iter().any(|(p, _)| *p == project)
        {
            projects.push((project, member));
        }
    }
    let state_dir = &config().state_dir;
    let helper = helper_image(state, docker).await;
    let mut out = Vec::new();
    for (project, member) in projects {
        let label = |name: &str| {
            member
                .labels
                .get(name)
                .map(String::as_str)
                .unwrap_or_default()
        };
        let working_dir = label("com.docker.compose.project.working_dir");
        let paths = compose::config_files(label("com.docker.compose.project.config_files"));
        let mut notes: Vec<String> = Vec::new();
        let mut files = Vec::new();
        let mut facts = compose::Facts::default();
        let mut total = 0;
        if paths.len() > compose::MAX_FILES {
            notes.push("it has too many Compose files to read".to_owned());
        }
        for path in paths.iter().take(compose::MAX_FILES) {
            let mut file = json!({ "path": path, "read": false });
            let read = if !compose::path_ok(path, working_dir, state_dir) {
                Err("not a file the panel may read".to_owned())
            } else if let Some(image) = &helper {
                docker
                    .engine()
                    .read_bind_file(image, path, compose::MAX_FILE)
                    .await
                    .map_err(|e| e.to_string())
                    .and_then(|archive| compose::file_of_archive(&archive))
            } else {
                Err("no helper image: set `[panel] helper_image`".to_owned())
            };
            match read {
                Ok(text) => {
                    total += text.len();
                    if total > compose::MAX_TOTAL {
                        notes.push("the Compose files are too big together".to_owned());
                        break;
                    }
                    file["sha256"] = json!(
                        Sha256::digest(text.as_bytes())
                            .iter()
                            .map(|b| format!("{b:02x}"))
                            .collect::<String>()
                    );
                    match compose::parse(&text) {
                        Ok(found) => {
                            file["read"] = json!(true);
                            facts.services.extend(found.services);
                            facts.profiled.extend(found.profiled);
                            facts.built.extend(found.built);
                            facts.with_secrets.extend(found.with_secrets);
                            facts.command_vars.extend(found.command_vars);
                        }
                        Err(why) => notes.push(format!("{path}: {why}")),
                    }
                }
                Err(why) => notes.push(format!("{path}: {why}")),
            }
            files.push(file);
        }
        facts.services.sort();
        facts.services.dedup();
        let without_container: Vec<&String> = facts
            .services
            .iter()
            .filter(|service| {
                !members.iter().any(|m| {
                    m.compose_project() == Some(project)
                        && m.compose_service() == Some(service.as_str())
                })
            })
            .collect();
        out.push(json!({
            "project": project,
            "files": files,
            "services": facts.services,
            "profiled": facts.profiled,
            "built": facts.built,
            "with_secrets": facts.with_secrets,
            "command_vars": facts.command_vars,
            "without_container": without_container,
            // Whether every container of the project goes: only then its file
            // is renamed (else the ones left behind would lose it).
            "containers": members.iter().filter(|m| m.compose_project() == Some(project)).count(),
            "taken": members
                .iter()
                .filter(|m| m.compose_project() == Some(project) && taken.contains(&m.id))
                .count(),
            "notes": notes,
        }));
    }
    json!(out)
}

/// The image id a container runs (`sha256:…`), the exact image whatever its tag.
fn image_of(doc: &Value) -> String {
    doc["Image"]
        .as_str()
        .or_else(|| doc["Config"]["Image"].as_str())
        .unwrap_or_default()
        .to_owned()
}

/// The tunnel token of an original cloudflared (its environment or command line),
/// read again from Docker when it is needed: the plan never holds it.
pub async fn tunnel_token_of_container(docker: &Docker, id: &str) -> Result<String, String> {
    let doc = docker
        .engine()
        .inspect_container_raw(id, false)
        .await
        .map_err(|e| e.to_string())?;
    let member = Member::from_inspect(&doc);
    let strings = |pointer: &str| -> Vec<String> {
        doc.pointer(pointer)
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut args = strings("/Config/Cmd");
    args.extend(strings("/Args"));
    import_plan_service::tunnel_token(&member.env, &args)
        .ok_or_else(|| "its tunnel token cannot be read".to_owned())
}

/// A name the panel takes for a service or a volume, from a Docker one.
fn sanitized(name: &str) -> String {
    let mut out: String = name
        .to_ascii_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_lowercase() || c.is_ascii_digit() {
                c
            } else {
                '-'
            }
        })
        .collect();
    out = out.trim_matches('-').to_owned();
    if !out.starts_with(|c: char| c.is_ascii_lowercase()) {
        out = format!("s{out}");
    }
    out.truncate(40);
    out.trim_end_matches('-').to_owned()
}

/// The symlink of a folder an imported service uses.
pub fn link_path(links: &std::path::Path, project: &str, service: &str, name: &str) -> String {
    links
        .join(format!("{project}_{service}"))
        .join(name)
        .to_string_lossy()
        .into_owned()
}

/// A volume or folder to copy or link: what the original was, what the panel
/// names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Moved {
    pub original: String,
    /// The volume's short name in the service, or the link's name.
    pub name: String,
    /// `copy` (a named volume) or `link` (a folder).
    pub mode: &'static str,
}

/// What a plan item becomes: the service to create, what to move, and the
/// image (`None`: a digest pins it; `Some(id)`: only this machine has it and
/// it is tagged first).
pub struct Made {
    pub service: crate::services::creation_service::NewService,
    pub moved: Vec<Moved>,
    pub local_image: Option<String>,
    pub healthcheck: Option<(String, i32, i32)>,
}

/// A word of a shell script: as it is when it is plain, else quoted.
fn script_word(word: &str) -> String {
    if !word.is_empty()
        && word.chars().all(|c| {
            c.is_ascii_alphanumeric()
                || matches!(c, '_' | '-' | '.' | '/' | ':' | '=' | '+' | '@' | ',')
        })
    {
        word.to_owned()
    } else {
        shell_words::quote(word).into_owned()
    }
}

/// The command the service runs. A secret in the original's command line
/// moves to an environment variable (`RDP_SECRET_<n>`, added to `env`) and the
/// command becomes `sh -c 'exec <entrypoint> <command> "$RDP_SECRET_<n>"'`: the
/// image's entrypoint runs it, which runs the original entrypoint and command.
fn command_of(detail: &Detail, env: &mut std::collections::BTreeMap<String, String>) -> String {
    use crate::services::secret_scan_service::{ArgPart, secret_name, split_secret_args};
    let known: Vec<String> = detail
        .member
        .env
        .iter()
        .filter(|(name, value)| secret_name(name) && !value.is_empty())
        .map(|(_, value)| value.clone())
        .collect();
    let Some(parts) = split_secret_args(&detail.cmd, &known)
        .filter(|parts| parts.iter().any(|p| matches!(p, ArgPart::Secret { .. })))
    else {
        return shell_words::join(&detail.cmd);
    };
    let mut script = String::from("exec");
    for word in &detail.image_entrypoint {
        script.push(' ');
        script.push_str(&script_word(word));
    }
    let mut n = 0;
    for part in parts {
        script.push(' ');
        match part {
            ArgPart::Plain(arg) => script.push_str(&script_word(&arg)),
            ArgPart::Secret { prefix, value } => {
                n += 1;
                let name = format!("RDP_SECRET_{n}");
                script.push_str(&format!(
                    "{}\"${name}\"",
                    if prefix.is_empty() {
                        String::new()
                    } else {
                        script_word(&prefix)
                    }
                ));
                env.insert(name, value);
            }
        }
    }
    shell_words::join(["sh", "-c", script.as_str()])
}

/// The service a container becomes (an app: databases map to apps for now).
pub fn new_service_of(
    detail: &Detail,
    project: &str,
    links: &std::path::Path,
) -> Result<Made, String> {
    use crate::services::creation_service::{NewMount, NewPort, NewService};
    let m = &detail.member;
    let name = sanitized(m.compose_service().unwrap_or(&m.name));
    if !crate::services::deploy_rules_service::valid_name(&name) {
        return Err(format!("`{}` is not a name the panel takes", m.name));
    }
    let (image, local_image) = match detail.repo_digests.first() {
        Some(digest) => (digest.clone(), None),
        None => {
            let tag: String = detail
                .image_id
                .trim_start_matches("sha256:")
                .chars()
                .take(12)
                .collect();
            (
                format!(
                    "{}{project}/{name}:{tag}",
                    crate::services::deployment_service::LOCAL_REGISTRY
                ),
                Some(detail.image_id.clone()),
            )
        }
    };
    let mut new = NewService::app(&name, image);
    new.env = m
        .env
        .iter()
        .filter(|pair| !detail.image_env.contains(pair))
        .cloned()
        .collect();
    if !detail.cmd.is_empty() && detail.cmd != detail.image_cmd {
        new.command = Some(command_of(detail, &mut new.env));
    }
    // Another user than the image's: kept (root too), the service's administrators' only.
    if detail.user != detail.image_user && !detail.user.is_empty() {
        new.user = Some(detail.user.clone());
    }
    if detail.memory > 0 {
        new.mem_limit_mb = u32::try_from(detail.memory / (1024 * 1024)).ok();
    }
    let reserved = &config().reserved_ports;
    // Docker lists a port once for IPv4 and once for IPv6: the panel has one.
    let mut published = std::collections::BTreeSet::new();
    for port in &detail.ports {
        let loopback = matches!(port.host_ip.as_str(), "127.0.0.1" | "::1");
        if !loopback
            && !reserved.contains(&port.host_port)
            && published.insert((port.host_port, port.protocol.clone()))
        {
            new.ports.push(NewPort {
                published: port.host_port,
                target: port.container_port,
                protocol: port.protocol.clone(),
            });
        }
    }
    let mut moved = Vec::new();
    for (n, mount) in m.mounts.iter().enumerate() {
        let short = if mount.kind == "volume"
            && crate::services::deploy_rules_service::valid_volume_name(&mount.source)
            && mount.source.len() < 40
        {
            mount.source.clone()
        } else {
            format!("data{}", n + 1)
        };
        if mount.kind == "volume" {
            new.mounts.push(NewMount::Volume {
                name: short.clone(),
                target: mount.destination.clone(),
            });
            moved.push(Moved {
                original: mount.source.clone(),
                name: short,
                mode: "copy",
            });
        } else {
            new.mounts.push(NewMount::Link {
                source: link_path(links, project, &name, &short),
                link_target: mount.source.clone(),
                target: mount.destination.clone(),
            });
            moved.push(Moved {
                original: mount.source.clone(),
                name: short,
                mode: "link",
            });
        }
    }
    // Two tasks must not write the same data (also checked when it deploys).
    new.zero_downtime = m.mounts.iter().all(|mount| !mount.writable);
    let healthcheck = match detail.health_test.as_slice() {
        [kind, rest @ ..] if !rest.is_empty() => {
            let command = if kind == "CMD-SHELL" {
                rest.join(" ")
            } else {
                shell_words::join(rest)
            };
            let interval =
                i32::try_from((detail.health_interval_ns / 1_000_000_000).max(1)).unwrap_or(30);
            let retries = i32::try_from(detail.health_retries.max(1)).unwrap_or(3);
            Some((command, interval, retries))
        }
        _ => None,
    };
    Ok(Made {
        service: new,
        moved,
        local_image,
        healthcheck,
    })
}

/// Starts an import: the plan read again (it must be the one the admin
/// saw), then in one write the project, a service for each container, what
/// the import holds (its items, volumes, claims), its audit entry and its
/// first step. Docker is read and the images tagged before the write.
pub async fn start(
    app: &AppState,
    principal: &rustcms::Principal,
    request: rustcms::audit::RequestMeta,
    import_id: i64,
    confirm: &str,
) -> AppResult<()> {
    use rustcms::ValidationErrors;
    use rustcms::audit::{self, AuditActor, AuditContext, AuditEntry};

    use crate::models::{container_import_claim, project, service as service_model};
    let row = container_import::Entity::find_by_id(import_id)
        .one(app.db())
        .await?
        .ok_or(AppError::NotFound)?;
    if row.state != "planned" {
        return Err(AppError::Conflict {
            code: "import_not_planned",
            reason: "state",
            message: "Only a plan that is ready can be started",
        });
    }
    if confirm != row.project_name {
        return Err(
            ValidationErrors::field("confirm", "type the project's name to confirm").into(),
        );
    }
    let plan: Value = row
        .plan
        .as_deref()
        .and_then(|p| serde_json::from_str(p).ok())
        .ok_or(AppError::NotFound)?;
    let items = plan["items"].as_array().cloned().unwrap_or_default();
    if items.iter().any(|i| i["choice"] == "blocked") {
        return Err(
            ValidationErrors::field("plan", "something in the plan blocks the import").into(),
        );
    }
    let brought: Vec<&Value> = items.iter().filter(|i| i["choice"] == "bring").collect();
    // The ticked domains of each container that becomes a service.
    let kept_domains = |item: &Value| -> Vec<crate::services::creation_service::NewDomain> {
        item["domains"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|d| d["skip"] != true)
            .map(|d| crate::services::creation_service::NewDomain {
                host: d["host"].as_str().unwrap_or_default().to_owned(),
                path: d["path"].as_str().unwrap_or("/").to_owned(),
                port: d["port"]
                    .as_u64()
                    .and_then(|p| u16::try_from(p).ok())
                    .unwrap_or(80),
                https: d["https"] == true,
                compose_service: None,
            })
            .collect()
    };
    let with_domains = brought
        .iter()
        .any(|i| i["as_tunnel"] != true && !kept_domains(i).is_empty());
    for permission in [
        "projects.create",
        "services.create",
        "service_domains.create",
    ] {
        if permission == "service_domains.create" && !with_domains {
            continue;
        }
        if !crate::services::access_service::admits(app.db(), principal, permission).await? {
            return Err(AppError::Forbidden);
        }
    }
    // The containers as they are now: the plan must still be true.
    let gathered = gather(app, &row)
        .await
        .map_err(|why| AppError::BadRequest(format!("Docker could not be read: {why}")))?;
    if Some(&gathered.fingerprint) != row.fingerprint.as_ref() {
        return Err(AppError::Conflict {
            code: "plan_stale",
            reason: "stale",
            message: "The containers changed since the plan was read: read it again",
        });
    }
    let mut made = Vec::new();
    // Containers that become the panel's own Cloudflare Tunnel: no service.
    let mut tunnels = Vec::new();
    for item in brought.iter().filter(|i| i["as_tunnel"] == true) {
        let id = item["container_id"].as_str().unwrap_or_default().to_owned();
        let docker = docker(app)?;
        let token = tunnel_token_of_container(&docker, &id)
            .await
            .map_err(|why| ValidationErrors::field("tunnel", &why))?;
        // The panel has one tunnel: not taken from another.
        if let Some(running) = crate::services::tunnel_service::running_token(app).await?
            && running != token
        {
            return Err(ValidationErrors::field(
                "tunnel",
                "the panel's Cloudflare Tunnel already runs another tunnel",
            )
            .into());
        }
        tunnels.push((
            id,
            item["name"].as_str().unwrap_or_default().to_owned(),
            item["compose_service"].as_str().map(str::to_owned),
        ));
    }
    for item in brought.iter().filter(|i| i["as_tunnel"] != true) {
        let id = item["container_id"].as_str().unwrap_or_default();
        let detail = gathered
            .details
            .iter()
            .find(|d| d.member.id == id)
            .ok_or_else(|| AppError::BadRequest("a container of the plan is gone".into()))?;
        let mut one = new_service_of(detail, &row.project_name, &settings(app).links())
            .map_err(AppError::BadRequest)?;
        // Written with the service, by its rules (`creation_service::create`).
        one.service.domains = kept_domains(item);
        made.push((
            id.to_owned(),
            detail.member.name.clone(),
            detail.member.compose_service().map(str::to_owned),
            one,
        ));
    }
    // Images only this machine has are tagged first (a second name).
    let docker = docker(app)?;
    for (_, _, _, one) in &made {
        if let Some(image_id) = &one.local_image {
            let reference = one.service.image.clone().unwrap_or_default();
            let (repo, tag) = reference.rsplit_once(':').unwrap_or((&reference, "latest"));
            docker
                .engine()
                .tag_image(image_id, repo, tag)
                .await
                .map_err(|e| AppError::BadRequest(format!("an image could not be kept: {e}")))?;
        }
    }
    let levels: Vec<Vec<String>> =
        serde_json::from_value(plan["levels"].clone()).unwrap_or_default();
    let level_of = |id: &str| -> i32 {
        levels
            .iter()
            .position(|l| l.iter().any(|x| x == id))
            .and_then(|p| i32::try_from(p).ok())
            .unwrap_or(0)
    };
    let txn = rustcms::db::begin_write(app.db()).await?;
    let current = container_import::Entity::find_by_id(import_id)
        .lock_exclusive()
        .one(&txn)
        .await?
        .ok_or(AppError::NotFound)?;
    if current.state != "planned" {
        return Err(AppError::Conflict {
            code: "import_not_planned",
            reason: "state",
            message: "Only a plan that is ready can be started",
        });
    }
    if project::Entity::find()
        .filter(project::Column::Name.eq(&row.project_name))
        .one(&txn)
        .await?
        .is_some()
    {
        return Err(ValidationErrors::field(
            "project_name",
            "a project of that name exists: choose another",
        )
        .into());
    }
    let now = chrono::Utc::now();
    let project = project::ActiveModel {
        name: Set(row.project_name.clone()),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(&txn)
    .await?;
    let claimed = |e: sea_orm::DbErr| -> AppError {
        let text = e.to_string().to_lowercase();
        if text.contains("unique") || text.contains("duplicate") {
            AppError::Conflict {
                code: "import_claimed",
                reason: "importing",
                message: "Another import holds one of these containers or volumes",
            }
        } else {
            e.into()
        }
    };
    let mut claimed_volumes = std::collections::BTreeSet::new();
    for (container_id, container_name, compose_service, one) in made {
        let created = crate::services::creation_service::create(
            &txn,
            app.keyring(),
            &project,
            one.service.clone(),
        )
        .await?;
        let admin_only = one.local_image.is_some()
            || one.service.user.is_some()
            || one.moved.iter().any(|m| m.mode == "link");
        let mut change = created.clone().into_active_model();
        if let Some((command, interval, retries)) = &one.healthcheck {
            change.healthcheck_cmd = Set(Some(command.clone()));
            change.healthcheck_interval_s = Set(Some(*interval));
            change.healthcheck_retries = Set(Some(*retries));
        }
        change.admin_only = Set(admin_only);
        change.update(&txn).await?;
        container_import_item::ActiveModel {
            import_id: Set(import_id),
            service_id: Set(Some(created.id)),
            container_id: Set(container_id.clone()),
            container_name: Set(container_name),
            compose_service: Set(compose_service),
            level: Set(level_of(&container_id)),
            choice: Set("bring".to_owned()),
            was_running: Set(false),
            state: Set("pending".to_owned()),
            ..Default::default()
        }
        .insert(&txn)
        .await?;
        container_import_claim::ActiveModel {
            import_id: Set(import_id),
            container_id: Set(Some(container_id)),
            volume: Set(None),
            ..Default::default()
        }
        .insert(&txn)
        .await
        .map_err(claimed)?;
        for moved in &one.moved {
            let panel_volume = (moved.mode == "copy").then(|| {
                crate::services::deploy_service::volume_name(
                    &project.name,
                    &created.name,
                    &moved.name,
                )
            });
            // A copy for each service that mounts it (a panel volume is one
            // service's); the claim, once.
            let first_claim =
                moved.mode == "copy" && claimed_volumes.insert(moved.original.clone());
            container_import_volume::ActiveModel {
                import_id: Set(Some(import_id)),
                original_name: Set(moved.original.clone()),
                mode: Set(moved.mode.to_owned()),
                panel_volume: Set(panel_volume),
                ..Default::default()
            }
            .insert(&txn)
            .await?;
            if first_claim {
                container_import_claim::ActiveModel {
                    import_id: Set(import_id),
                    container_id: Set(None),
                    volume: Set(Some(moved.original.clone())),
                    ..Default::default()
                }
                .insert(&txn)
                .await
                .map_err(claimed)?;
            }
        }
    }
    for (container_id, container_name, compose_service) in tunnels {
        container_import_item::ActiveModel {
            import_id: Set(import_id),
            service_id: Set(None),
            container_id: Set(container_id.clone()),
            container_name: Set(container_name),
            compose_service: Set(compose_service),
            level: Set(level_of(&container_id)),
            choice: Set("bring".to_owned()),
            was_running: Set(false),
            state: Set("pending".to_owned()),
            ..Default::default()
        }
        .insert(&txn)
        .await?;
        container_import_claim::ActiveModel {
            import_id: Set(import_id),
            container_id: Set(Some(container_id)),
            volume: Set(None),
            ..Default::default()
        }
        .insert(&txn)
        .await
        .map_err(claimed)?;
    }
    let mut queued = current.into_active_model();
    queued.state = Set("queued".to_owned());
    queued.project_id = Set(Some(project.id));
    queued.updated_at = Set(chrono::Utc::now());
    queued.update(&txn).await?;
    let entry = AuditEntry::new("imports.start", "container_imports")
        .key(import_id.to_string())
        .meta(serde_json::json!({
            "project": row.project_name,
            "services": brought.len(),
            "domains": brought
                .iter()
                .filter(|i| i["as_tunnel"] != true)
                .flat_map(|i| kept_domains(i))
                .map(|d| d.host)
                .collect::<Vec<_>>(),
        }));
    audit::record(
        &txn,
        &AuditContext::new(AuditActor::user(principal), request),
        entry,
    )
    .await?;
    rustcms::queue::dispatch(&txn, &crate::jobs::import_step::ImportStep { import_id }).await?;
    txn.commit().await?;
    let _ = service_model::Entity;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::import_plan_service::Port;

    #[test]
    fn a_secret_in_the_command_goes_to_the_environment_and_the_command_reads_it() {
        let mut detail = Detail::default();
        detail.member.name = "cache".to_owned();
        detail.repo_digests = vec!["valkey@sha256:abc".to_owned()];
        detail.image_entrypoint = vec!["docker-entrypoint.sh".to_owned()];
        detail.entrypoint = detail.image_entrypoint.clone();
        detail.cmd = vec![
            "valkey-server".to_owned(),
            "--requirepass".to_owned(),
            "s3cr3t value".to_owned(),
            "--auth=tok3n".to_owned(),
        ];
        let made = new_service_of(&detail, "p", std::path::Path::new("/links")).unwrap();
        let command = made.service.command.clone().unwrap();
        // The panel's command is `sh -c '…'`: the image's entrypoint runs it, which runs the
        // original entrypoint and command, with the secrets read from the environment.
        assert!(command.starts_with("sh -c "), "{command}");
        assert!(
            command.contains(
                "exec docker-entrypoint.sh valkey-server --requirepass \"$RDP_SECRET_1\""
            ),
            "{command}"
        );
        assert!(command.contains("--auth=\"$RDP_SECRET_2\""), "{command}");
        assert!(
            !command.contains("s3cr3t") && !command.contains("tok3n"),
            "{command}"
        );
        assert_eq!(made.service.env["RDP_SECRET_1"], "s3cr3t value");
        assert_eq!(made.service.env["RDP_SECRET_2"], "tok3n");
        // The command splits back into `sh`, `-c`, the script.
        assert_eq!(
            crate::services::deploy_service::command_args(&command)
                .unwrap()
                .len(),
            3
        );
    }

    #[test]
    fn a_port_published_on_both_address_families_is_one_port() {
        let mut detail = Detail::default();
        detail.member.name = "web".to_owned();
        detail.repo_digests = vec!["web@sha256:abc".to_owned()];
        let port = |ip: &str| Port {
            host_ip: ip.to_owned(),
            host_port: 9000,
            container_port: 80,
            protocol: "tcp".to_owned(),
        };
        detail.ports = vec![port("0.0.0.0"), port("::")];
        let made = new_service_of(&detail, "p", std::path::Path::new("/links")).unwrap();
        assert_eq!(made.service.ports.len(), 1, "{:?}", made.service.ports);
    }
}
