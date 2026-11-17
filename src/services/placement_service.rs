//! Where a service's tasks run (`services.placement`): automatic (`None`,
//! the panel's node when only that machine has what it needs), one node
//! (`node:<id>`), or any node (`any`, storage shared between nodes). The
//! panel reaches containers through its own Docker only: backups, scripts,
//! the terminal and live stats work for the tasks on the panel's node.

use crate::services::docker_service::{Docker, SwarmTask, TaskState};

/// Any node of the Swarm.
pub const ANY: &str = "any";
/// Longest stored value (`node:` and a Swarm node id).
pub const MAX: usize = 80;

/// A placement as chosen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Placement {
    Any,
    Node(String),
}

/// `any` or `node:<id>` (letters and digits, as Swarm's ids); anything else:
/// none.
pub fn parse(value: &str) -> Option<Placement> {
    if value == ANY {
        return Some(Placement::Any);
    }
    let id = value.strip_prefix("node:")?;
    (!id.is_empty() && id.len() <= MAX - 5 && id.chars().all(|c| c.is_ascii_alphanumeric()))
        .then(|| Placement::Node(id.to_owned()))
}

/// Why a service cannot be placed by choice: a Compose project (its own
/// file says), an image the panel builds or only this machine has (no other
/// node has it). None: it can.
pub fn problem(kind: &str, source: &str, image: &str) -> Option<&'static str> {
    if kind == "compose" {
        Some("a Compose service runs on the panel's node")
    } else if source != "image" {
        Some("an image the panel builds is only on the panel's node")
    } else if crate::services::deploy_rules_service::local_image(image) {
        Some("an image only this machine has stays on the panel's node")
    } else {
        None
    }
}

/// The node this panel's Docker is.
pub async fn panel_node(docker: &Docker) -> Result<String, String> {
    let node = docker.info().await.map_err(|e| e.to_string())?.node_id;
    if node.is_empty() {
        return Err("Docker did not say which Swarm node this is".to_owned());
    }
    Ok(node)
}

/// The Swarm's nodes as the form lists them: id, name, role, availability,
/// state, and whether it is the panel's.
pub async fn nodes(docker: &Docker) -> Result<Vec<serde_json::Value>, String> {
    let here = panel_node(docker).await.unwrap_or_default();
    let list = docker
        .engine()
        .list_nodes()
        .await
        .map_err(|e| e.to_string())?;
    Ok(list
        .into_iter()
        .filter_map(|n| {
            let id = n["ID"].as_str()?.to_owned();
            Some(serde_json::json!({
                "id": id,
                "name": n["Description"]["Hostname"].as_str().unwrap_or(&id),
                "role": n["Spec"]["Role"].as_str().unwrap_or_default(),
                "availability": n["Spec"]["Availability"].as_str().unwrap_or_default(),
                "state": n["Status"]["State"].as_str().unwrap_or_default(),
                "panel": id == here,
            }))
        })
        .collect())
}

/// Why the panel may not reach a service's containers: it runs (or may
/// run) on another node. None: the panel's node.
pub async fn away(docker: &Docker, placement: Option<&str>) -> Result<Option<String>, String> {
    match placement.and_then(parse) {
        None => Ok(None),
        Some(Placement::Any) => Ok(Some(
            "it may run on any node, and the panel reaches only the containers on its own node"
                .to_owned(),
        )),
        Some(Placement::Node(id)) => Ok((id != panel_node(docker).await?).then(|| {
            "it runs on another node, and the panel reaches only the containers on its own node"
                .to_owned()
        })),
    }
}

/// Whether a task is on `node` (a task without one: a single machine's).
pub fn on_node(task: &SwarmTask, node: &str) -> bool {
    task.node_id
        .as_deref()
        .is_none_or(|n| n.is_empty() || n == node)
}

/// The first running task's container on the panel's node.
pub fn running_here(tasks: &[SwarmTask], node: &str) -> Option<String> {
    tasks
        .iter()
        .filter(|t| t.status.as_ref().and_then(|s| s.state) == Some(TaskState::RUNNING))
        .filter(|t| on_node(t, node))
        .find_map(|t| {
            t.status
                .as_ref()?
                .container_status
                .as_ref()?
                .container_id
                .clone()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_placement_is_any_or_a_node_id() {
        assert_eq!(parse("any"), Some(Placement::Any));
        assert_eq!(parse("node:abc123"), Some(Placement::Node("abc123".into())));
        assert_eq!(parse("node:"), None);
        assert_eq!(parse("node:a b"), None);
        assert_eq!(parse("node:x==y"), None);
        assert_eq!(parse("ANY"), None);
    }

    #[test]
    fn built_local_and_compose_services_stay_on_the_panels_node() {
        assert!(problem("compose", "image", "redis:7").is_some());
        assert!(problem("app", "git", "").is_some());
        assert!(problem("app", "image", "rdp.invalid/x:1").is_some());
        assert!(problem("app", "image", "redis:7").is_none());
        assert!(problem("postgres", "image", "postgres:17").is_none());
    }
}
