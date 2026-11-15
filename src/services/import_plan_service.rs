//! The plan of a container import: what each container becomes in the panel,
//! what blocks it, what is lost, in which order the services start. Pure
//! functions over what Docker said (`import_graph_service::Member` and the
//! rest of an inspect, in [`Detail`]); the plan holds names, ports and
//! findings, never a value of an environment variable.

use std::collections::{BTreeMap, BTreeSet};

use hmac::{Hmac, KeyInit, Mac};
use serde::Serialize;
use serde_json::Value;
use sha2::Sha256;

use crate::services::deploy_service::valid_env_name;
use crate::services::import_graph_service::{Closure, Edge, Hardness, Member, PanelFacts};

/// The plan's format: a plan of another version is read again, not trusted.
pub const PLAN_VERSION: u32 = 1;

/// Host ports the panel and its Traefik use.
const RESERVED_PORTS: [u16; 6] = [80, 443, 3000, 8000, 8080, 8443];
/// The panel's environment limits (`deploy_rules_service`).
const MAX_ENV: usize = 100;
const MAX_ENV_VALUE: usize = 16 * 1024;
/// A writable layer bigger than this is said before the move (bytes).
const BIG_LAYER: i64 = 100 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Level {
    /// Cannot be imported until the cause is gone.
    Block,
    /// Needs the admin's confirmation.
    Confirm,
    Warn,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    pub code: &'static str,
    pub level: Level,
    pub field: String,
    pub text: String,
    pub how_to_unblock: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Choice {
    Bring,
    Leave,
    Blocked,
}

/// A port published on the host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Port {
    pub host_ip: String,
    pub host_port: u16,
    pub container_port: u16,
    pub protocol: String,
}

/// What the plan needs of a container beyond [`Member`]: the rest of its
/// inspect and of its image.
#[derive(Debug, Clone, Default)]
pub struct Detail {
    pub member: Member,
    pub image: String,
    pub restart: String,
    pub auto_remove: bool,
    pub privileged: bool,
    pub user: String,
    pub image_user: String,
    pub entrypoint: Vec<String>,
    pub image_entrypoint: Vec<String>,
    /// `Config.Cmd` and the image's own.
    pub cmd: Vec<String>,
    pub image_cmd: Vec<String>,
    /// `Config.Healthcheck`: `Test`, interval (ns) and retries.
    pub health_test: Vec<String>,
    pub health_interval_ns: i64,
    pub health_retries: i64,
    /// `HostConfig.Memory` (bytes, 0: none).
    pub memory: i64,
    /// The image the container runs (`sha256:…`) and its `RepoDigests`.
    pub image_id: String,
    pub repo_digests: Vec<String>,
    /// The image's own environment (a pair equal to one here is left out).
    pub image_env: Vec<(String, String)>,
    pub ports: Vec<Port>,
    pub size_rw: Option<i64>,
    /// Named volumes: name → (driver, has options).
    pub volume_drivers: BTreeMap<String, (String, bool)>,
}

impl Detail {
    /// From the container's raw inspect and its image's.
    pub fn from_inspect(doc: &Value, image_doc: &Value) -> Detail {
        let member = Member::from_inspect(doc);
        let strings = |v: &Value| -> Vec<String> {
            v.as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default()
        };
        let image_env = strings(&image_doc["Config"]["Env"])
            .into_iter()
            .filter_map(|e| e.split_once('=').map(|(k, v)| (k.to_owned(), v.to_owned())))
            .collect();
        let mut ports = Vec::new();
        if let Some(map) = doc["NetworkSettings"]["Ports"].as_object() {
            for (key, bindings) in map {
                let (container, protocol) = key.split_once('/').unwrap_or((key, "tcp"));
                for b in bindings.as_array().into_iter().flatten() {
                    if let (Ok(container_port), Some(Ok(host_port))) = (
                        container.parse(),
                        b["HostPort"].as_str().map(str::parse::<u16>),
                    ) {
                        ports.push(Port {
                            host_ip: b["HostIp"].as_str().unwrap_or_default().to_owned(),
                            host_port,
                            container_port,
                            protocol: protocol.to_owned(),
                        });
                    }
                }
            }
        }
        ports.sort_by_key(|p| (p.host_port, p.container_port));
        Detail {
            image: doc["Config"]["Image"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            restart: doc["HostConfig"]["RestartPolicy"]["Name"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            auto_remove: doc["HostConfig"]["AutoRemove"].as_bool().unwrap_or(false),
            privileged: doc["HostConfig"]["Privileged"].as_bool().unwrap_or(false),
            user: doc["Config"]["User"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            image_user: image_doc["Config"]["User"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            entrypoint: strings(&doc["Config"]["Entrypoint"]),
            image_entrypoint: strings(&image_doc["Config"]["Entrypoint"]),
            cmd: strings(&doc["Config"]["Cmd"]),
            image_cmd: strings(&image_doc["Config"]["Cmd"]),
            health_test: strings(&doc["Config"]["Healthcheck"]["Test"]),
            health_interval_ns: doc["Config"]["Healthcheck"]["Interval"]
                .as_i64()
                .unwrap_or(0),
            health_retries: doc["Config"]["Healthcheck"]["Retries"]
                .as_i64()
                .unwrap_or(0),
            memory: doc["HostConfig"]["Memory"].as_i64().unwrap_or(0),
            image_id: doc["Image"].as_str().unwrap_or_default().to_owned(),
            repo_digests: strings(&image_doc["RepoDigests"]),
            image_env,
            ports,
            size_rw: doc["SizeRw"].as_i64(),
            volume_drivers: BTreeMap::new(),
            member,
        }
    }
}

fn finding(
    code: &'static str,
    level: Level,
    field: &str,
    text: &str,
    unblock: Option<&str>,
) -> Finding {
    Finding {
        code,
        level,
        field: field.to_owned(),
        text: text.to_owned(),
        how_to_unblock: unblock.map(str::to_owned),
    }
}

/// How many secrets of the command line move to the environment (their
/// variables are `RDP_SECRET_<n>`, made when the services are).
pub fn command_secrets(detail: &Detail) -> usize {
    use crate::services::secret_scan_service::{ArgPart, secret_name, split_secret_args};
    let known: Vec<String> = detail
        .member
        .env
        .iter()
        .filter(|(name, value)| secret_name(name) && !value.is_empty())
        .map(|(_, value)| value.clone())
        .collect();
    split_secret_args(&detail.cmd, &known)
        .map(|parts| {
            parts
                .iter()
                .filter(|p| matches!(p, ArgPart::Secret { .. }))
                .count()
        })
        .unwrap_or(0)
}

/// What the panel cannot or will not carry over, container by container.
pub fn findings(detail: &Detail, panel: &PanelFacts) -> Vec<Finding> {
    use crate::services::import_graph_service::excluded;
    let mut out = Vec::new();
    let m = &detail.member;
    if let Some(why) = excluded(m, panel) {
        out.push(finding("excluded", Level::Block, "container", why, None));
    }
    if matches!(m.state.as_str(), "paused" | "dead") {
        out.push(finding(
            "state",
            Level::Block,
            "state",
            "it is paused or dead",
            Some("Start or remove it, then read again."),
        ));
    }
    if detail.auto_remove {
        out.push(finding(
            "auto_remove",
            Level::Block,
            "auto_remove",
            "it is removed when it stops (--rm): there would be nothing to undo to",
            None,
        ));
    }
    if detail.privileged {
        out.push(finding(
            "privileged",
            Level::Block,
            "privileged",
            "it runs privileged",
            None,
        ));
    }
    if m.modes.iter().any(|mode| mode == "host") {
        out.push(finding(
            "network_mode",
            Level::Block,
            "network_mode",
            "it uses the host's network",
            None,
        ));
    }
    if m.env.len() > MAX_ENV {
        out.push(finding(
            "env_limit",
            Level::Block,
            "env",
            "it has more than 100 environment variables",
            None,
        ));
    }
    for (name, value) in &m.env {
        if !valid_env_name(name) {
            out.push(finding(
                "env_name",
                Level::Block,
                &format!("env {name}"),
                "the name is not one a shell takes",
                Some("Rename it in the container."),
            ));
        } else if value.contains("$(") {
            out.push(finding(
                "env_placeholder",
                Level::Block,
                &format!("env {name}"),
                "the value has `$(…)`, which the panel would replace when it deploys",
                None,
            ));
        } else if value.len() > MAX_ENV_VALUE {
            out.push(finding(
                "env_limit",
                Level::Block,
                &format!("env {name}"),
                "the value is over 16 KiB",
                None,
            ));
        }
    }
    // A secret in the command line moves to the environment: the panel does
    // not rewrite a command yet, so it is the admin's to do.
    let known: Vec<String> = m
        .env
        .iter()
        .filter(|(name, value)| {
            crate::services::secret_scan_service::secret_name(name) && !value.is_empty()
        })
        .map(|(_, value)| value.clone())
        .collect();
    let health = detail.health_test.get(1..).unwrap_or_default();
    if !health.is_empty()
        && crate::services::secret_scan_service::redact_args(health, &known) != health
    {
        out.push(finding(
            "argv_secret",
            Level::Block,
            "healthcheck",
            "its health check holds a secret",
            Some("Make it read the secret from an environment variable, then read again."),
        ));
    }
    if crate::services::secret_scan_service::redact_args(&detail.cmd, &known) != detail.cmd {
        if crate::services::secret_scan_service::split_secret_args(&detail.cmd, &known).is_some() {
            // The panel moves it: into an environment variable the command reads.
            out.push(finding(
                "argv_secret_moved",
                Level::Confirm,
                "command",
                "its command line holds a secret: it moves to an environment variable and the command runs through `sh -c`",
                Some("Confirm; the image needs a `/bin/sh`."),
            ));
        } else {
            out.push(finding(
                "argv_secret",
                Level::Block,
                "command",
                "its command line holds a secret",
                Some("Make it read the secret from an environment variable, then read again."),
            ));
        }
    }
    if detail.user != detail.image_user {
        // Kept: the service runs as that user (root is said as such).
        out.push(finding(
            "user_kept",
            Level::Confirm,
            "user",
            "it runs as another user than its image's: the service keeps that user",
            Some("Confirm, or change the image's user and read again."),
        ));
    }
    if !detail.entrypoint.is_empty() && detail.entrypoint != detail.image_entrypoint {
        out.push(finding(
            "entrypoint",
            Level::Block,
            "entrypoint",
            "its entrypoint is not its image's",
            None,
        ));
    }
    for (name, (driver, options)) in &detail.volume_drivers {
        if driver != "local" || *options {
            out.push(finding(
                "volume_driver",
                Level::Block,
                &format!("volume {name}"),
                "the volume is not a plain local one (it may be a bind in disguise)",
                None,
            ));
        }
    }
    // Docker lists a port for IPv4 and again for IPv6: said once.
    let mut said_ports = BTreeSet::new();
    for port in &detail.ports {
        if !said_ports.insert((port.host_port, port.protocol.clone())) {
            continue;
        }
        if matches!(port.host_ip.as_str(), "127.0.0.1" | "::1") {
            out.push(finding(
                "port_loopback",
                Level::Info,
                &format!("port {}", port.host_port),
                "published on the loopback only: left out, never opened to the network",
                None,
            ));
        } else if !matches!(port.host_ip.as_str(), "" | "0.0.0.0" | "::") {
            out.push(finding(
                "port_one_address",
                Level::Confirm,
                &format!("port {}", port.host_port),
                "published on one address only: the panel publishes it on every address",
                Some("Confirm, or leave the port out and use a domain."),
            ));
        } else if RESERVED_PORTS.contains(&port.host_port) {
            out.push(finding(
                "port_reserved",
                Level::Confirm,
                &format!("port {}", port.host_port),
                "the panel keeps this port: it is not published; reach the service by a domain, or give it another port afterwards",
                Some("Confirm: add a domain or another port to the service after the import."),
            ));
        }
    }
    if !matches!(detail.restart.as_str(), "always" | "unless-stopped") {
        out.push(finding(
            "restart_policy",
            Level::Warn,
            "restart",
            "the panel always restarts a service",
            None,
        ));
    }
    if detail.size_rw.is_some_and(|size| size > BIG_LAYER) {
        out.push(finding(
            "writable_layer",
            Level::Confirm,
            "writable_layer",
            "its writable layer is large and does not move with it",
            None,
        ));
    }
    let own_names: BTreeSet<&str> = [m.id.as_str(), m.compose_service().unwrap_or_default()]
        .into_iter()
        .collect();
    if m.networks.len() > 1
        || m.networks
            .iter()
            .flat_map(|n| &n.aliases)
            .any(|a| !own_names.contains(a.as_str()) && !m.id.starts_with(a.as_str()))
    {
        out.push(finding(
            "lost_settings",
            Level::Warn,
            "network",
            "extra aliases and networks are not kept",
            None,
        ));
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Item {
    pub container_id: String,
    pub name: String,
    pub compose_project: Option<String>,
    pub compose_service: Option<String>,
    pub state_at_read: String,
    pub choice: Choice,
    /// Members bound by a hard edge share a group number.
    pub hard_group: usize,
    /// The names of the environment variables kept (a pair equal to the
    /// image's own is left out). Values are never in the plan.
    pub env_names: Vec<String>,
    /// Secrets of its command line that become environment variables
    /// (`RDP_SECRET_1`…): names the plan shows, values never.
    pub command_secrets: usize,
    pub ports: Vec<Port>,
    pub findings: Vec<Finding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Plan {
    pub version: u32,
    pub seed: String,
    pub project_name: String,
    pub fingerprint: String,
    pub items: Vec<Item>,
    pub edges: Vec<Edge>,
    /// Container ids by deploy level: a level starts when the one before is up.
    pub levels: Vec<Vec<String>>,
    pub truncated: bool,
}

impl Plan {
    /// Whether anything stops the import: a blocked member, or members the
    /// admin must still confirm.
    pub fn blocked(&self) -> bool {
        self.items.iter().any(|i| i.choice == Choice::Blocked)
    }
}

/// The plan of `closure` over `details` (one per member, in any order).
pub fn plan_of(
    details: &[Detail],
    closure: &Closure,
    seed: &str,
    project_name: &str,
    panel: &PanelFacts,
    fingerprint: &str,
) -> Plan {
    let by_id: BTreeMap<&str, &Detail> =
        details.iter().map(|d| (d.member.id.as_str(), d)).collect();
    // Groups: members bound by a hard edge (union by the smallest index).
    let index: BTreeMap<&str, usize> = closure
        .members
        .iter()
        .enumerate()
        .map(|(i, id)| (id.as_str(), i))
        .collect();
    let mut group: Vec<usize> = (0..closure.members.len()).collect();
    fn root(group: &[usize], mut i: usize) -> usize {
        while group[i] != i {
            i = group[i];
        }
        i
    }
    for edge in closure
        .edges
        .iter()
        .filter(|e| matches!(e.hardness, Hardness::Hard | Hardness::Block))
    {
        if let (Some(&a), Some(&b)) = (index.get(edge.from.as_str()), index.get(edge.to.as_str())) {
            let (ra, rb) = (root(&group, a), root(&group, b));
            group[ra.max(rb)] = ra.min(rb);
        }
    }
    let mut items: Vec<Item> = closure
        .members
        .iter()
        .enumerate()
        .map(|(i, id)| {
            let Some(detail) = by_id.get(id.as_str()) else {
                return Item {
                    container_id: id.clone(),
                    name: id.clone(),
                    compose_project: None,
                    compose_service: None,
                    state_at_read: String::new(),
                    choice: Choice::Blocked,
                    hard_group: root(&group, i),
                    env_names: vec![],
                    command_secrets: 0,
                    ports: vec![],
                    findings: vec![finding(
                        "unreadable",
                        Level::Block,
                        "container",
                        "Docker did not give its details",
                        Some("Read again."),
                    )],
                };
            };
            let m = &detail.member;
            let mut found = findings(detail, panel);
            for edge in closure
                .edges
                .iter()
                .filter(|e| e.from == *id && e.hardness == Hardness::Block)
            {
                found.push(finding(
                    "edge_block",
                    Level::Block,
                    &edge.signal,
                    "it is tied to another container by its network mode or an address in its settings",
                    None,
                ));
            }
            // A volume of the panel belongs to one service: two members sharing
            // one would each get a copy, and stop sharing without a word.
            if closure
                .edges
                .iter()
                .any(|e| e.kind == "volume" && (e.from == *id || e.to == *id))
            {
                found.push(finding(
                    "shared_volume",
                    Level::Block,
                    "volume",
                    "it shares a volume with another container: in the panel a volume belongs to one service",
                    Some("Give each its own volume, or run them as one service."),
                ));
            }
            let mut env_names: Vec<String> = m
                .env
                .iter()
                .filter(|pair| !detail.image_env.contains(pair))
                .map(|(name, _)| name.clone())
                .collect();
            env_names.sort();
            let blocked = found.iter().any(|f| f.level == Level::Block);
            Item {
                container_id: id.clone(),
                name: m.name.clone(),
                compose_project: m.compose_project().map(str::to_owned),
                compose_service: m.compose_service().map(str::to_owned),
                state_at_read: m.state.clone(),
                choice: if blocked { Choice::Blocked } else { Choice::Bring },
                hard_group: root(&group, i),
                env_names,
                command_secrets: command_secrets(detail),
                ports: detail.ports.clone(),
                findings: found,
            }
        })
        .collect();
    // One member of a hard group that cannot be taken keeps the group.
    let blocked_groups: BTreeSet<usize> = items
        .iter()
        .filter(|i| i.choice == Choice::Blocked)
        .map(|i| i.hard_group)
        .collect();
    for item in items
        .iter_mut()
        .filter(|i| i.choice != Choice::Blocked && blocked_groups.contains(&i.hard_group))
    {
        item.choice = Choice::Blocked;
        item.findings.push(finding(
            "group_blocked",
            Level::Block,
            "group",
            "it shares writable data or a network with a container that cannot be imported",
            None,
        ));
    }
    let calls: Vec<Edge> = closure
        .edges
        .iter()
        .filter(|e| matches!(e.kind, "dns" | "depends_on" | "links"))
        .cloned()
        .collect();
    Plan {
        version: PLAN_VERSION,
        seed: seed.to_owned(),
        project_name: project_name.to_owned(),
        fingerprint: fingerprint.to_owned(),
        levels: order(&closure.members, &calls),
        items,
        edges: closure.edges.clone(),
        truncated: closure.truncated,
    }
}

/// Deploy levels: a container after the ones it depends on (`depends_on`,
/// a DNS call), the others in the same level; a cycle is one level.
pub fn order(ids: &[String], edges: &[Edge]) -> Vec<Vec<String>> {
    let wants = |a: &str| -> BTreeSet<&str> {
        edges
            .iter()
            .filter(|e| e.from == a && ids.contains(&e.to))
            .map(|e| e.to.as_str())
            .collect()
    };
    // Whether `from` reaches `to` by what each wants.
    let reaches = |from: &str, to: &str| -> bool {
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        let mut stack = vec![from];
        while let Some(x) = stack.pop() {
            for next in wants(x) {
                if next == to {
                    return true;
                }
                if seen.insert(next) {
                    stack.push(next);
                }
            }
        }
        false
    };
    let mut left: Vec<&str> = ids.iter().map(String::as_str).collect();
    let mut levels = Vec::new();
    while !left.is_empty() {
        let mut level: Vec<&str> = left
            .iter()
            .copied()
            .filter(|a| wants(a).iter().all(|d| !left.contains(d)))
            .collect();
        if level.is_empty() {
            // A cycle: the ones whose every need leads back to them go together.
            level = left
                .iter()
                .copied()
                .filter(|a| {
                    wants(a)
                        .iter()
                        .filter(|d| left.contains(d))
                        .all(|d| reaches(d, a))
                })
                .collect();
        }
        if level.is_empty() {
            level = left.clone();
        }
        left.retain(|a| !level.contains(a));
        levels.push(level.into_iter().map(str::to_owned).collect());
    }
    levels
}

/// The HMAC of what an import depends on in a container: its Config,
/// HostConfig, Mounts and networks (without addresses), not what changes by
/// itself (health, start time, restart count, pid, sizes).
pub fn fingerprint(docs: &[Value], key: &[u8]) -> String {
    fn canonical(value: &Value) -> Value {
        match value {
            Value::Object(map) => {
                let sorted: BTreeMap<&String, Value> =
                    map.iter().map(|(k, v)| (k, canonical(v))).collect();
                Value::Object(sorted.into_iter().map(|(k, v)| (k.clone(), v)).collect())
            }
            Value::Array(items) => Value::Array(items.iter().map(canonical).collect()),
            other => other.clone(),
        }
    }
    let mut parts: Vec<String> = docs
        .iter()
        .map(|doc| {
            let networks: BTreeMap<String, Value> = doc["NetworkSettings"]["Networks"]
                .as_object()
                .map(|map| {
                    map.iter()
                        .map(|(name, n)| (name.clone(), n["Aliases"].clone()))
                        .collect()
                })
                .unwrap_or_default();
            let stable = serde_json::json!({
                "Id": doc["Id"],
                "Config": doc["Config"],
                "HostConfig": doc["HostConfig"],
                "Mounts": doc["Mounts"],
                "Status": doc["State"]["Status"],
                "Networks": networks,
            });
            canonical(&stable).to_string()
        })
        .collect();
    parts.sort();
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC takes any key");
    for part in &parts {
        mac.update(part.as_bytes());
        mac.update(&[0]);
    }
    mac.finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::services::import_graph_service::{NetworkUse, closure};

    fn detail(id: &str, service: &str) -> Detail {
        let mut d = Detail::default();
        d.member.id = id.to_owned();
        d.member.name = format!("p-{service}-1");
        d.member.state = "running".to_owned();
        d.member.labels = BTreeMap::from([
            ("com.docker.compose.project".to_owned(), "p".to_owned()),
            ("com.docker.compose.service".to_owned(), service.to_owned()),
        ]);
        d.member.networks = vec![NetworkUse {
            network: "p_default".to_owned(),
            aliases: vec![service.to_owned(), id.to_owned()],
            ip: format!("172.20.0.{}", id.len() + 1),
        }];
        d.restart = "always".to_owned();
        d
    }

    fn codes(findings: &[Finding]) -> Vec<&'static str> {
        findings.iter().map(|f| f.code).collect()
    }

    #[test]
    fn what_blocks_and_what_is_said() {
        let panel = PanelFacts::default();
        assert!(findings(&detail("a", "web"), &panel).is_empty());
        let mut d = detail("a", "web");
        d.member.state = "paused".to_owned();
        d.auto_remove = true;
        d.privileged = true;
        d.member.modes = vec!["host".to_owned()];
        d.user = "0:0".to_owned();
        d.image_user = "app".to_owned();
        d.entrypoint = vec!["/custom".to_owned()];
        d.image_entrypoint = vec!["/entry.sh".to_owned()];
        d.member.env = vec![
            ("OK".to_owned(), "1".to_owned()),
            ("bad-name".to_owned(), "1".to_owned()),
            ("URL".to_owned(), "http://$(PRIMARY_DOMAIN)".to_owned()),
        ];
        d.volume_drivers
            .insert("nfs".to_owned(), ("nfs".to_owned(), true));
        d.restart = "no".to_owned();
        let found = findings(&d, &panel);
        for code in [
            "state",
            "auto_remove",
            "privileged",
            "network_mode",
            "entrypoint",
            "env_name",
            "env_placeholder",
            "volume_driver",
        ] {
            let f = found
                .iter()
                .find(|f| f.code == code)
                .unwrap_or_else(|| panic!("{code}"));
            assert_eq!(f.level, Level::Block, "{code}");
        }
        assert!(codes(&found).contains(&"restart_policy"));
        // A value never travels into a finding.
        assert!(!format!("{found:?}").contains("PRIMARY_DOMAIN"));
    }

    #[test]
    fn a_secret_in_the_command_line_moves_to_the_environment_when_it_can() {
        let panel = PanelFacts::default();
        let mut d = detail("a", "cache");
        d.cmd = vec![
            "valkey-server".to_owned(),
            "--requirepass".to_owned(),
            "s3cr3t-value".to_owned(),
        ];
        let found = findings(&d, &panel);
        // Confirmed, not blocked: the panel moves it (and the command runs through `sh -c`).
        let f = found
            .iter()
            .find(|f| f.code == "argv_secret_moved")
            .unwrap();
        assert_eq!(f.level, Level::Confirm);
        assert!(!codes(&found).contains(&"argv_secret"));
        assert!(!format!("{found:?}").contains("s3cr3t-value"));
        // One that cannot be cut out of its argument still blocks.
        d.cmd = vec!["app".to_owned(), "redis://:hunter22@cache".to_owned()];
        let blocked = findings(&d, &panel);
        let f = blocked.iter().find(|f| f.code == "argv_secret").unwrap();
        assert_eq!(f.level, Level::Block);
        // A command without one is left alone.
        d.cmd = vec![
            "valkey-server".to_owned(),
            "--port".to_owned(),
            "6379".to_owned(),
        ];
        let none = findings(&d, &panel);
        assert!(
            !codes(&none).contains(&"argv_secret") && !codes(&none).contains(&"argv_secret_moved")
        );
    }

    #[test]
    fn a_secret_in_the_health_check_blocks_too() {
        let panel = PanelFacts::default();
        let mut d = detail("a", "cache");
        d.member.env = vec![("REDIS_PASSWORD".to_owned(), "hunter22".to_owned())];
        d.health_test = vec![
            "CMD-SHELL".to_owned(),
            "redis-cli -a hunter22 ping".to_owned(),
        ];
        let found = findings(&d, &panel);
        let f = found.iter().find(|f| f.code == "argv_secret").unwrap();
        assert_eq!(f.field, "healthcheck");
        assert!(!format!("{found:?}").contains("hunter22"));
    }

    #[test]
    fn a_port_on_one_address_is_said_not_opened_on_all_silently() {
        let panel = PanelFacts::default();
        let mut d = detail("a", "web");
        d.ports = vec![Port {
            host_ip: "192.168.1.5".to_owned(),
            host_port: 9000,
            container_port: 80,
            protocol: "tcp".to_owned(),
        }];
        let found = findings(&d, &panel);
        let f = found.iter().find(|f| f.code == "port_one_address").unwrap();
        assert_eq!(f.level, Level::Confirm);
        // On every address (v4 and v6): nothing to say.
        d.ports = vec![
            Port {
                host_ip: "0.0.0.0".to_owned(),
                host_port: 9000,
                container_port: 80,
                protocol: "tcp".to_owned(),
            },
            Port {
                host_ip: "::".to_owned(),
                host_port: 9000,
                container_port: 80,
                protocol: "tcp".to_owned(),
            },
        ];
        assert!(!codes(&findings(&d, &panel)).contains(&"port_one_address"));
    }

    #[test]
    fn another_user_than_the_images_is_kept_and_confirmed_not_blocked() {
        let panel = PanelFacts::default();
        let mut d = detail("a", "tunnel");
        d.user = "0:0".to_owned();
        d.image_user = "nonroot".to_owned();
        let found = findings(&d, &panel);
        let f = found.iter().find(|f| f.code == "user_kept").unwrap();
        assert_eq!(f.level, Level::Confirm);
        assert!(!codes(&found).contains(&"user"));
        // The same user as the image's: nothing to say.
        d.user = "nonroot".to_owned();
        assert!(!codes(&findings(&d, &panel)).contains(&"user_kept"));
    }

    #[test]
    fn a_port_on_both_address_families_is_said_once_and_the_secrets_of_a_command_are_counted() {
        let panel = PanelFacts::default();
        let mut d = detail("a", "web");
        let port = |ip: &str| Port {
            host_ip: ip.to_owned(),
            host_port: 3000,
            container_port: 3000,
            protocol: "tcp".to_owned(),
        };
        d.ports = vec![port("0.0.0.0"), port("::")];
        let found = findings(&d, &panel);
        assert_eq!(
            found.iter().filter(|f| f.code == "port_reserved").count(),
            1
        );
        // The text says what happens: it is not published.
        let said = found.iter().find(|f| f.code == "port_reserved").unwrap();
        assert!(said.text.contains("not published"), "{}", said.text);
        d.cmd = vec![
            "valkey-server".to_owned(),
            "--requirepass".to_owned(),
            "s3cr3t-value".to_owned(),
        ];
        assert_eq!(command_secrets(&d), 1);
        d.cmd = vec!["valkey-server".to_owned()];
        assert_eq!(command_secrets(&d), 0);
    }

    #[test]
    fn env_limits_are_the_panels() {
        let panel = PanelFacts::default();
        let mut d = detail("a", "web");
        d.member.env = (0..101)
            .map(|i| (format!("V{i}"), "1".to_owned()))
            .collect();
        assert!(codes(&findings(&d, &panel)).contains(&"env_limit"));
        let mut d = detail("a", "web");
        d.member.env = vec![("BIG".to_owned(), "x".repeat(16 * 1024 + 1))];
        assert!(codes(&findings(&d, &panel)).contains(&"env_limit"));
    }

    #[test]
    fn ports_are_dropped_asked_or_kept() {
        let panel = PanelFacts::default();
        let mut d = detail("a", "web");
        let port = |ip: &str, host| Port {
            host_ip: ip.to_owned(),
            host_port: host,
            container_port: 80,
            protocol: "tcp".to_owned(),
        };
        d.ports = vec![
            port("127.0.0.1", 5432),
            port("0.0.0.0", 3000),
            port("0.0.0.0", 9000),
        ];
        let found = findings(&d, &panel);
        let by = |code| found.iter().filter(|f| f.code == code).count();
        assert_eq!(by("port_loopback"), 1);
        assert_eq!(by("port_reserved"), 1);
        assert_eq!(
            found
                .iter()
                .find(|f| f.code == "port_reserved")
                .unwrap()
                .level,
            Level::Confirm
        );
    }

    #[test]
    fn a_blocked_member_blocks_the_group_it_shares_data_with() {
        let mut web = detail("web1", "web");
        let mut db = detail("db1", "db");
        // A shared folder: it ties them (a volume is blocked on its own).
        let m = |source: &str| crate::services::import_graph_service::Mount {
            kind: "bind".to_owned(),
            source: source.to_owned(),
            destination: "/data".to_owned(),
            writable: true,
        };
        web.member.mounts = vec![m("/srv/shared")];
        db.member.mounts = vec![m("/srv/shared")];
        db.privileged = true;
        let mut free = detail("free1", "free");
        free.member.networks[0].network = "other".to_owned();
        let members = [web.member.clone(), db.member.clone(), free.member.clone()];
        let c = closure(&members, &["web1".to_owned()]);
        let plan = plan_of(
            &[web, db, free],
            &c,
            "web1",
            "p",
            &PanelFacts::default(),
            "fp",
        );
        let by = |id: &str| plan.items.iter().find(|i| i.container_id == id).unwrap();
        assert_eq!(by("db1").choice, Choice::Blocked);
        assert_eq!(
            by("web1").choice,
            Choice::Blocked,
            "it shares writable data with db1"
        );
        assert!(codes(&by("web1").findings).contains(&"group_blocked"));
        assert_eq!(by("web1").hard_group, by("db1").hard_group);
        assert!(plan.blocked());
        assert!(!plan.items.iter().any(|i| i.container_id == "free1"));
    }

    #[test]
    fn two_services_cannot_share_a_volume_in_the_panel() {
        let m = |source: &str| crate::services::import_graph_service::Mount {
            kind: "volume".to_owned(),
            source: source.to_owned(),
            destination: "/data".to_owned(),
            writable: true,
        };
        let mut web = detail("web1", "web");
        let mut worker = detail("work1", "worker");
        web.member.mounts = vec![m("shared")];
        worker.member.mounts = vec![m("shared")];
        let members = [web.member.clone(), worker.member.clone()];
        let c = closure(&members, &["web1".to_owned()]);
        let plan = plan_of(
            &[web, worker],
            &c,
            "web1",
            "p",
            &PanelFacts::default(),
            "fp",
        );
        for item in &plan.items {
            assert_eq!(item.choice, Choice::Blocked, "{}", item.name);
            let f = item
                .findings
                .iter()
                .find(|f| f.code == "shared_volume")
                .unwrap();
            assert_eq!(f.level, Level::Block);
        }
    }

    #[test]
    fn a_plan_keeps_names_of_env_not_values_and_leaves_out_the_images_own() {
        let mut d = detail("a", "web");
        d.member.env = vec![
            ("PATH".to_owned(), "/bin".to_owned()),
            ("SECRET_TOKEN".to_owned(), "s3cr3t-value".to_owned()),
            ("PGDATA".to_owned(), "/custom".to_owned()),
        ];
        d.image_env = vec![
            ("PATH".to_owned(), "/bin".to_owned()),
            ("PGDATA".to_owned(), "/var/lib/postgresql/data".to_owned()),
        ];
        let c = closure(&[d.member.clone()], &["a".to_owned()]);
        let plan = plan_of(&[d], &c, "a", "p", &PanelFacts::default(), "fp");
        assert_eq!(plan.items[0].env_names, ["PGDATA", "SECRET_TOKEN"]);
        assert!(
            !serde_json::to_string(&plan)
                .unwrap()
                .contains("s3cr3t-value")
        );
    }

    #[test]
    fn deploy_levels_follow_dependencies_and_collapse_cycles() {
        let edge = |from: &str, to: &str| Edge {
            from: from.to_owned(),
            to: to.to_owned(),
            kind: "dns",
            signal: "env X".to_owned(),
            hardness: Hardness::Soft,
        };
        let ids = ["app", "db", "cache", "tunnel"].map(str::to_owned);
        // app calls db and cache; tunnel calls app.
        let levels = order(
            &ids,
            &[
                edge("app", "db"),
                edge("app", "cache"),
                edge("tunnel", "app"),
            ],
        );
        assert_eq!(
            levels,
            [
                vec!["db".to_owned(), "cache".to_owned()],
                vec!["app".to_owned()],
                vec!["tunnel".to_owned()]
            ]
        );
        // a ↔ b: one level; c after them.
        let ids = ["a", "b", "c"].map(str::to_owned);
        let levels = order(&ids, &[edge("a", "b"), edge("b", "a"), edge("c", "a")]);
        assert_eq!(
            levels,
            [vec!["a".to_owned(), "b".to_owned()], vec!["c".to_owned()]]
        );
    }

    #[test]
    fn the_fingerprint_ignores_what_changes_by_itself() {
        let doc = |health: &str, started: &str, ip: &str, image: &str| {
            json!({
                "Id": "abc", "Config": {"Image": image, "Env": ["A=1"]},
                "HostConfig": {"RestartPolicy": {"Name": "always"}},
                "Mounts": [{"Type": "volume", "Name": "data"}],
                "State": {"Status": "running", "Health": {"Status": health}, "StartedAt": started, "Pid": 5},
                "RestartCount": 3,
                "SizeRw": 10,
                "NetworkSettings": {"Networks": {"n": {"IPAddress": ip, "EndpointID": ip, "MacAddress": ip, "Aliases": ["web"]}}}
            })
        };
        let base = fingerprint(&[doc("healthy", "t1", "1.1.1.1", "img:1")], b"key");
        assert_eq!(
            base,
            fingerprint(&[doc("starting", "t2", "2.2.2.2", "img:1")], b"key")
        );
        assert_ne!(
            base,
            fingerprint(&[doc("healthy", "t1", "1.1.1.1", "img:2")], b"key")
        );
        assert_ne!(
            base,
            fingerprint(&[doc("healthy", "t1", "1.1.1.1", "img:1")], b"other")
        );
        // Order of the documents does not matter.
        let a = doc("healthy", "t", "1.1.1.1", "img:1");
        let mut b = a.clone();
        b["Id"] = json!("def");
        assert_eq!(
            fingerprint(&[a.clone(), b.clone()], b"key"),
            fingerprint(&[b, a], b"key")
        );
    }
}
