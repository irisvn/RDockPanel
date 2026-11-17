//! The Swarm's nodes (Docker → Nodes): what the list shows, and the rules of
//! every change. The panel's own node is never paused, drained or removed
//! (the panel, its database and Traefik run there); a manager is never
//! removed; a node is removed only once it has left (Down), never forced; a
//! node's spec is changed whole, as read (its labels and role kept).

use std::collections::{BTreeSet, HashMap};

use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter};
use serde_json::{Value, json};

use crate::services::docker_service::{Docker, MANAGED_LABEL, SYSTEM_LABEL, TaskState};

/// Why a change is refused (said to the user as is).
#[derive(Debug, PartialEq, Eq)]
pub enum Refused {
    /// No such node.
    Gone,
    /// The rule that refuses it.
    Rule(&'static str),
    /// Docker could not be read or refused.
    Docker(String),
}

/// A node's availability in Swarm's words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Availability {
    Active,
    Pause,
    Drain,
}

impl Availability {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "active" => Some(Self::Active),
            "pause" => Some(Self::Pause),
            "drain" => Some(Self::Drain),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Pause => "pause",
            Self::Drain => "drain",
        }
    }
}

/// The panel's node, read now (never guessed: a mutation without it is refused).
async fn panel_node(docker: &Docker) -> Result<String, Refused> {
    crate::services::placement_service::panel_node(docker)
        .await
        .map_err(Refused::Docker)
}

async fn node(docker: &Docker, id: &str) -> Result<Value, Refused> {
    docker
        .engine()
        .inspect_node(id)
        .await
        .map_err(|e| Refused::Docker(e.to_string()))?
        .ok_or(Refused::Gone)
}

/// The Swarm services by id: name and labels.
async fn services(docker: &Docker) -> Result<HashMap<String, (String, Value)>, Refused> {
    Ok(docker
        .engine()
        .list_services()
        .await
        .map_err(|e| Refused::Docker(e.to_string()))?
        .into_iter()
        .filter_map(|s| {
            let id = s["ID"].as_str()?.to_owned();
            let name = s["Spec"]["Name"].as_str().unwrap_or_default().to_owned();
            Some((id, (name, s["Spec"].clone())))
        })
        .collect())
}

fn is_system(spec: &Value) -> bool {
    spec["Labels"][SYSTEM_LABEL] == "true"
}

/// The nodes as the page lists them: whitelisted fields, labels with
/// secret-looking values hidden, and the running tasks of the services the
/// reader may see (the panel's own only for `system`).
pub async fn list(docker: &Docker, system: bool) -> Result<Vec<Value>, Refused> {
    let here = crate::services::placement_service::panel_node(docker)
        .await
        .unwrap_or_default();
    let nodes = docker
        .engine()
        .list_nodes()
        .await
        .map_err(|e| Refused::Docker(e.to_string()))?;
    // Unread, the panel's own could not be told apart: an error, never a count.
    let services = services(docker).await?;
    let mut out = Vec::new();
    for n in nodes {
        let Some(id) = n["ID"].as_str().map(str::to_owned) else {
            continue;
        };
        let tasks = docker
            .engine()
            .node_tasks(&id)
            .await
            .map_err(|e| Refused::Docker(e.to_string()))?;
        let running = tasks
            .iter()
            .filter(|t| t.status.as_ref().and_then(|s| s.state) == Some(TaskState::RUNNING))
            .filter(|t| {
                let spec = t
                    .service_id
                    .as_deref()
                    .and_then(|s| services.get(s))
                    .map(|(_, spec)| spec);
                // Who may not see the panel's own counts only the tasks of a
                // service known to be someone else's.
                system || spec.is_some_and(|spec| !is_system(spec))
            })
            .count();
        let mut labels: HashMap<String, String> = n["Spec"]["Labels"]
            .as_object()
            .map(|m| {
                m.iter()
                    .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_owned()))
                    .collect()
            })
            .unwrap_or_default();
        crate::services::secret_scan_service::redact_labels(&mut labels);
        let resources = &n["Description"]["Resources"];
        out.push(json!({
            "id": id,
            "name": n["Description"]["Hostname"].as_str().unwrap_or(&id),
            "role": n["Spec"]["Role"].as_str().unwrap_or_default(),
            "availability": n["Spec"]["Availability"].as_str().unwrap_or_default(),
            "state": n["Status"]["State"].as_str().unwrap_or_default(),
            "message": n["Status"]["Message"].as_str().unwrap_or_default(),
            "address": n["Status"]["Addr"].as_str().unwrap_or_default(),
            "leader": n["ManagerStatus"]["Leader"] == true,
            "reachability": n["ManagerStatus"]["Reachability"].as_str(),
            "engine": n["Description"]["Engine"]["EngineVersion"].as_str().unwrap_or_default(),
            "os": n["Description"]["Platform"]["OS"].as_str().unwrap_or_default(),
            "arch": n["Description"]["Platform"]["Architecture"].as_str().unwrap_or_default(),
            "cpus": resources["NanoCPUs"].as_i64().map(|c| c as f64 / 1e9),
            "memory": resources["MemoryBytes"].as_i64(),
            "labels": labels.into_iter().collect::<std::collections::BTreeMap<_, _>>(),
            "panel": !here.is_empty() && id == here,
            "tasks": running,
        }));
    }
    // The panel's node first, then by name.
    out.sort_by(|a, b| {
        (b["panel"] == true, a["name"].as_str()).cmp(&(a["panel"] == true, b["name"].as_str()))
    });
    Ok(out)
}

/// Sets a node's availability: its whole spec as read, only `Availability`
/// changed, at the version read (a change meanwhile is refused by Docker).
/// The panel's node only ever goes back to active.
pub async fn set_availability(
    docker: &Docker,
    id: &str,
    availability: Availability,
) -> Result<(), Refused> {
    let here = panel_node(docker).await?;
    let node = node(docker, id).await?;
    let full = node["ID"].as_str().ok_or(Refused::Gone)?;
    if full == here && availability != Availability::Active {
        return Err(Refused::Rule(
            "the panel's own node runs the panel, its database and Traefik: it is never paused or drained",
        ));
    }
    let version = node["Version"]["Index"].as_i64().ok_or(Refused::Rule(
        "Docker did not give the node's version: look again",
    ))?;
    let mut spec = node["Spec"].clone();
    if !spec.is_object() || spec["Role"].as_str().is_none() {
        return Err(Refused::Rule(
            "Docker did not give the node's settings: look again",
        ));
    }
    spec["Availability"] = json!(availability.as_str());
    docker
        .engine()
        .update_node(full, version, spec)
        .await
        .map_err(|e| Refused::Docker(e.to_string()))
}

/// Removes a worker that has left the Swarm (`docker swarm leave` on it:
/// Down). Never the panel's node, never a manager, never forced.
pub async fn remove(docker: &Docker, id: &str) -> Result<(), Refused> {
    let here = panel_node(docker).await?;
    let node = node(docker, id).await?;
    let full = node["ID"].as_str().ok_or(Refused::Gone)?;
    if full == here {
        return Err(Refused::Rule("the panel's own node is never removed"));
    }
    match node["Spec"]["Role"].as_str() {
        Some("worker") => {}
        Some(_) => {
            return Err(Refused::Rule(
                "a manager is never removed here: demote it with Docker first",
            ));
        }
        None => {
            return Err(Refused::Rule(
                "Docker did not give the node's role: look again",
            ));
        }
    }
    if node["Status"]["State"] != "down" {
        return Err(Refused::Rule(
            "it is still in the Swarm: run `docker swarm leave` on it first (it then shows Down)",
        ));
    }
    // Swarm moves a gone node's tasks away: while some are still assigned
    // to it, it has not finished.
    let assigned = docker
        .engine()
        .node_tasks(full)
        .await
        .map_err(|e| Refused::Docker(e.to_string()))?
        .iter()
        .any(|t| t.desired_state == Some(TaskState::RUNNING));
    if assigned {
        return Err(Refused::Rule(
            "Swarm still assigns tasks to it: wait until they have moved, then remove it",
        ));
    }
    docker
        .engine()
        .remove_node(full)
        .await
        .map_err(|e| Refused::Docker(e.to_string()))
}

/// What draining (or losing) a node does to what runs there, from Docker's
/// live state: each service with tasks on it, whether it is tied to that
/// node (stays pending), keeps local data (starts empty elsewhere) or is
/// not the panel's; and the panel's services placed on it for their next
/// deployment. Promises nothing: Swarm decides.
pub async fn impact<C: ConnectionTrait>(
    db: &C,
    docker: &Docker,
    id: &str,
) -> Result<Value, Refused> {
    let node = node(docker, id).await?;
    let full = node["ID"].as_str().ok_or(Refused::Gone)?.to_owned();
    let hostname = node["Description"]["Hostname"].as_str().unwrap_or_default();
    let services = services(docker).await?;
    let tasks = docker
        .engine()
        .node_tasks(&full)
        .await
        .map_err(|e| Refused::Docker(e.to_string()))?;
    let mut counted: HashMap<String, usize> = HashMap::new();
    for task in tasks
        .iter()
        .filter(|t| t.desired_state == Some(TaskState::RUNNING))
    {
        if let Some(service) = task.service_id.as_deref() {
            *counted.entry(service.to_owned()).or_default() += 1;
        }
    }
    let mut affected: Vec<Value> = counted
        .into_iter()
        .map(|(service, tasks)| {
            let (name, spec) = services
                .get(&service)
                .cloned()
                .unwrap_or_else(|| (service.clone(), Value::Null));
            let constraints: Vec<String> = spec["TaskTemplate"]["Placement"]["Constraints"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|c| c.as_str().map(|s| s.replace(' ', "")))
                .collect();
            let tied = constraints.iter().any(|c| {
                c == &format!("node.id=={full}")
                    || (!hostname.is_empty() && c == &format!("node.hostname=={hostname}"))
            });
            // Other placement rules (labels, roles, platforms): whether
            // another node fits is Swarm's to say.
            let constrained = !tied && !constraints.is_empty();
            let mounts = spec["TaskTemplate"]["ContainerSpec"]["Mounts"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            json!({
                "name": name,
                "tasks": tasks,
                "tied": tied,
                "constrained": constrained,
                "volumes": mounts.iter().any(|m| m["Type"] == "volume"),
                "folders": mounts.iter().any(|m| m["Type"] == "bind"),
                "panel": spec["Labels"][MANAGED_LABEL] == "true",
                "system": is_system(&spec),
            })
        })
        .collect();
    affected.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    use crate::models::{project, service};
    let db_err = |e: sea_orm::DbErr| Refused::Docker(e.to_string());
    let placed_rows = service::Entity::find()
        .filter(service::Column::Placement.eq(format!("node:{full}")))
        .all(db)
        .await
        .map_err(db_err)?;
    let projects: HashMap<i64, String> = project::Entity::find()
        .filter(project::Column::Id.is_in(placed_rows.iter().map(|s| s.project_id)))
        .all(db)
        .await
        .map_err(db_err)?
        .into_iter()
        .map(|p| (p.id, p.name))
        .collect();
    let placed: BTreeSet<String> = placed_rows
        .iter()
        .map(|s| {
            format!(
                "{}_{}",
                projects.get(&s.project_id).cloned().unwrap_or_default(),
                s.name
            )
        })
        .collect();
    Ok(json!({ "services": affected, "placed": placed }))
}

/// The address other servers join at: the panel's node's advertised manager
/// address (`ManagerStatus.Addr`). A loopback or unspecified one (a Mac's
/// Docker advertises 127.0.0.1) is no use elsewhere: why instead.
pub async fn join_address(docker: &Docker) -> Result<Result<String, &'static str>, Refused> {
    let here = panel_node(docker).await?;
    let node = node(docker, &here).await?;
    let Some(addr) = node["ManagerStatus"]["Addr"]
        .as_str()
        .filter(|a| !a.is_empty())
    else {
        return Ok(Err("this node is not a Swarm manager the others can join"));
    };
    Ok(usable_address(addr).map(|()| addr.to_owned()))
}

/// Whether another server can reach `addr` (`host:port`, `[v6]:port`).
pub fn usable_address(addr: &str) -> Result<(), &'static str> {
    let host = match addr.rsplit_once(':') {
        Some((host, port)) if port.parse::<u16>().is_ok() => host,
        _ => return Err("the Swarm's address has no port"),
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    match host.parse::<std::net::IpAddr>() {
        Ok(ip) if ip.is_loopback() => {
            Err("the Swarm listens on this machine only (127.0.0.1): other servers cannot join it")
        }
        Ok(ip) if ip.is_unspecified() => Err("the Swarm has no address other servers can use"),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_other_servers_can_reach() {
        assert!(usable_address("192.168.0.245:2377").is_ok());
        assert!(usable_address("[2001:db8::1]:2377").is_ok());
        assert!(usable_address("manager.example.com:4000").is_ok());
        assert!(usable_address("127.0.0.1:2377").is_err());
        assert!(usable_address("[::1]:2377").is_err());
        assert!(usable_address("0.0.0.0:2377").is_err());
        assert!(usable_address("192.168.0.245").is_err());
    }

    #[test]
    fn availabilities_are_swarms_words() {
        assert_eq!(Availability::parse("drain"), Some(Availability::Drain));
        assert_eq!(Availability::parse("Drain"), None);
        assert_eq!(Availability::Pause.as_str(), "pause");
    }
}
