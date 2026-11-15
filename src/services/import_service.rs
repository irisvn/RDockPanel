//! Importing existing containers: reading them and writing the plan
//! (docs/import.md). The state machine that stops, copies and
//! deploys comes with the cut-over; here an import is `discovering`, then
//! `planned` (or `discover_failed`).

use std::collections::BTreeMap;

use rustcms::{AppError, AppResult, AppState};
use sea_orm::{ActiveModelTrait, EntityTrait, IntoActiveModel, Set};
use serde_json::{Value, json};

use crate::models::container_import;
use crate::services::deploy_rules_service::config;
use crate::services::docker_service::Docker;
use crate::services::import_compose_service as compose;
use crate::services::import_graph_service::{self, Member, PanelFacts};
use crate::services::import_plan_service::{self, Detail};

/// What marks the panel's own database: the loopback ports of its URL, and
/// the working directory of a panel that runs from source.
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

/// The image of the helper containers (one with `cp` and `sh`): `[panel]
/// helper_image`, else the panel's own. Bound as a service so that tests can
/// bind another.
#[derive(Clone, Debug, Default)]
pub struct HelperImage(pub Option<String>);

impl HelperImage {
    pub fn from_config() -> Self {
        let configured = &config().helper_image;
        Self((!configured.is_empty()).then(|| configured.clone()))
    }
}

async fn helper_image(state: &AppState, docker: &Docker) -> Option<String> {
    if let Some(HelperImage(Some(image))) = state.service::<HelperImage>().as_deref() {
        return Some(image.clone());
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

/// The plan (as JSON) and its fingerprint.
async fn read(state: &AppState, row: &container_import::Model) -> Result<(Value, String), String> {
    let docker = docker(state).map_err(|e| e.to_string())?;
    let engine = docker.engine();
    let listed = engine
        .containers_detailed()
        .await
        .map_err(|e| e.to_string())?;
    // Raw inspects stay here: their environments are never sent on.
    let mut docs: BTreeMap<String, Value> = BTreeMap::new();
    for item in &listed {
        let system = item["Labels"]["rdockpanel.system"] == "true";
        let Some(id) = item["Id"].as_str().filter(|_| !system) else {
            continue;
        };
        if let Ok(doc) = engine.inspect_container_raw(id, false).await {
            docs.insert(id.to_owned(), doc);
        }
    }
    let members: Vec<Member> = docs.values().map(Member::from_inspect).collect();
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
    let mut taken = Vec::new();
    for id in &closure.members {
        let (Some(doc), Some(image)) = (docs.get(id), docs.get(id).map(image_of)) else {
            continue;
        };
        let image_doc = engine.image_config(&image).await.unwrap_or(Value::Null);
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
        taken.push(doc.clone());
        details.push(detail);
    }
    let key = state.keyring().current().derive("rdockpanel.import");
    let fingerprint = import_plan_service::fingerprint(&taken, &key);
    let compose_info = compose_of(state, &docker, &members, &closure.members).await;
    let plan = import_plan_service::plan_of(
        &details,
        &closure,
        &row.source_ref,
        &row.project_name,
        &panel_facts(),
        &fingerprint,
    );
    let mut plan = serde_json::to_value(&plan).map_err(|e| e.to_string())?;
    plan["compose"] = compose_info;
    Ok((plan, fingerprint))
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
