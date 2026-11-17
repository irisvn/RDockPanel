//! What a container import takes with a container: the set of containers
//! that must move together, found from what Docker says about them (shared
//! volumes, links, DNS names in env and commands). Pure: the documents are
//! read elsewhere, nothing here touches Docker or the database. An edge
//! carries where it was seen (`env DATABASE_URL`), never a value.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;
use serde_json::Value;

/// More than this many containers is not one import.
pub const MAX_MEMBERS: usize = 50;

/// How an edge binds two containers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Hardness {
    /// Pulls the other side in: they share writable data. If the other side
    /// cannot be taken, neither can this one.
    Hard,
    /// Always blocks (`network_mode: container:x`, a hard-coded IP).
    Block,
    /// One calls the other by name; chosen by default, can be left behind
    /// with a confirmation that lists what breaks.
    Soft,
    /// A call across Compose projects: shown, not chosen (it would drag the
    /// other project's data and secrets in).
    SoftOff,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Edge {
    pub from: String,
    pub to: String,
    /// `volume`, `bind`, `volumes_from`, `network_mode`, `ip`, `dns`, `depends_on`, `links`.
    pub kind: &'static str,
    /// Where it was seen: `volume data`, `env DATABASE_URL`, `cmd`.
    pub signal: String,
    pub hardness: Hardness,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mount {
    /// `volume` or `bind`.
    pub kind: String,
    /// The volume's name, or the host directory.
    pub source: String,
    pub destination: String,
    pub writable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkUse {
    pub network: String,
    pub aliases: Vec<String>,
    pub ip: String,
}

/// What the graph needs of a container. Env values are kept here, in memory
/// only, to look for names and addresses: they never leave the server.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Member {
    pub id: String,
    pub name: String,
    pub state: String,
    pub labels: BTreeMap<String, String>,
    pub env: Vec<(String, String)>,
    /// `Config.Cmd`, `Entrypoint` and the health check, one string each.
    pub command: Vec<String>,
    pub mounts: Vec<Mount>,
    pub volumes_from: Vec<String>,
    pub links: Vec<String>,
    pub networks: Vec<NetworkUse>,
    /// `HostConfig.NetworkMode`, `PidMode`, `IpcMode`.
    pub modes: Vec<String>,
    /// `Config.WorkingDir`.
    pub working_dir: String,
    /// Ports published on the host: (host address, port).
    pub published: Vec<(String, u16)>,
}

const NAME_MARKERS: [&str; 6] = ["HOST", "ADDR", "URL", "SERVER", "ENDPOINT", "DSN"];

static URL_AUTHORITY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[A-Za-z][A-Za-z0-9+.-]*://([^/\s?#]*)").expect("valid regex"));
static HOST_PORT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z0-9][A-Za-z0-9._-]*:\d{1,5}$").expect("valid regex"));
static IPV4: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\d{1,3}(?:\.\d{1,3}){3}").expect("valid regex"));

impl Member {
    pub fn compose_project(&self) -> Option<&str> {
        self.labels
            .get("com.docker.compose.project")
            .map(String::as_str)
    }

    pub fn compose_service(&self) -> Option<&str> {
        self.labels
            .get("com.docker.compose.service")
            .map(String::as_str)
    }

    /// From a container's raw inspect document.
    pub fn from_inspect(doc: &Value) -> Member {
        let text = |pointer: &str| {
            doc.pointer(pointer)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let list = |pointer: &str| -> Vec<String> {
            doc.pointer(pointer)
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default()
        };
        let labels = doc
            .pointer("/Config/Labels")
            .and_then(Value::as_object)
            .map(|map| {
                map.iter()
                    .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned())))
                    .collect()
            })
            .unwrap_or_default();
        let env = list("/Config/Env")
            .into_iter()
            .filter_map(|e| {
                e.split_once('=')
                    .map(|(name, value)| (name.to_owned(), value.to_owned()))
            })
            .collect();
        let mut command = Vec::new();
        for pointer in ["/Config/Cmd", "/Config/Entrypoint"] {
            let words = list(pointer);
            if !words.is_empty() {
                command.push(words.join(" "));
            }
        }
        let health = list("/Config/Healthcheck/Test");
        if health.len() > 1 {
            command.push(health[1..].join(" "));
        }
        let mounts = doc
            .pointer("/Mounts")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|m| {
                        let kind = m["Type"].as_str()?;
                        let source = match kind {
                            "volume" => m["Name"].as_str()?,
                            "bind" => m["Source"].as_str()?,
                            _ => return None,
                        };
                        Some(Mount {
                            kind: kind.to_owned(),
                            source: source.to_owned(),
                            destination: m["Destination"].as_str().unwrap_or_default().to_owned(),
                            writable: m["RW"].as_bool().unwrap_or(true),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let networks = doc
            .pointer("/NetworkSettings/Networks")
            .and_then(Value::as_object)
            .map(|map| {
                map.iter()
                    .map(|(network, n)| NetworkUse {
                        network: network.clone(),
                        aliases: n["Aliases"]
                            .as_array()
                            .map(|a| {
                                a.iter()
                                    .filter_map(Value::as_str)
                                    .map(str::to_owned)
                                    .collect()
                            })
                            .unwrap_or_default(),
                        ip: n["IPAddress"].as_str().unwrap_or_default().to_owned(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let modes = ["NetworkMode", "PidMode", "IpcMode"]
            .iter()
            .map(|k| text(&format!("/HostConfig/{k}")))
            .filter(|m| !m.is_empty())
            .collect();
        let mut published = Vec::new();
        if let Some(Value::Object(ports)) = doc.pointer("/NetworkSettings/Ports") {
            for bindings in ports.values().filter_map(Value::as_array) {
                for b in bindings {
                    if let Some(port) = b["HostPort"].as_str().and_then(|p| p.parse().ok()) {
                        let ip = b["HostIp"].as_str().unwrap_or_default().to_owned();
                        published.push((ip, port));
                    }
                }
            }
        }
        Member {
            id: text("/Id"),
            name: text("/Name").trim_start_matches('/').to_owned(),
            state: text("/State/Status"),
            labels,
            env,
            command,
            mounts,
            volumes_from: list("/HostConfig/VolumesFrom"),
            links: list("/HostConfig/Links"),
            networks,
            modes,
            working_dir: text("/Config/WorkingDir"),
            published,
        }
    }
}

/// What makes the panel's own containers (never imported).
#[derive(Debug, Clone, Default)]
pub struct PanelFacts {
    /// Loopback ports the panel's database listens on (a container publishing
    /// one is the panel's MySQL when it runs from source).
    pub database_ports: Vec<u16>,
    /// The panel's working directory, when it runs from source.
    pub working_dir: Option<String>,
    /// The installer's directory (`[panel] state_dir`): never a service's mount.
    pub state_dir: std::path::PathBuf,
}

/// Why a container can never be imported, whoever asks.
pub fn excluded(member: &Member, panel: &PanelFacts) -> Option<&'static str> {
    let label = |name: &str| member.labels.get(name).is_some_and(|v| v == "true");
    if label("rdockpanel.system") {
        return Some("it belongs to the panel");
    }
    if label("rdockpanel.managed") {
        return Some("it is a service of the panel already");
    }
    if member.labels.contains_key("com.docker.swarm.service.id") {
        return Some("it is a task of a Swarm service");
    }
    let loopback = |ip: &str| matches!(ip, "127.0.0.1" | "::1" | "");
    if member
        .published
        .iter()
        .any(|(ip, port)| loopback(ip) && panel.database_ports.contains(port))
    {
        return Some("it looks like the panel's own database");
    }
    if let Some(dir) = &panel.working_dir {
        let inside = |path: &str| path == dir || path.starts_with(&format!("{dir}/"));
        if inside(&member.working_dir)
            || member
                .mounts
                .iter()
                .any(|m| m.kind == "bind" && inside(&m.source))
        {
            return Some("it runs the panel's own files");
        }
    }
    None
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Closure {
    /// Container ids, in discovery order.
    pub members: Vec<String>,
    pub edges: Vec<Edge>,
    /// More than [`MAX_MEMBERS`] were reachable: the rest is left out.
    pub truncated: bool,
}

/// The hosts a text names: a URL's host (no userinfo), `host:port`.
pub fn host_tokens(text: &str) -> Vec<String> {
    let mut hosts = Vec::new();
    for caps in URL_AUTHORITY.captures_iter(text) {
        let authority = caps[1].rsplit('@').next().unwrap_or_default();
        let host = if let Some(rest) = authority.strip_prefix('[') {
            rest.split(']').next().unwrap_or_default()
        } else {
            authority.split(':').next().unwrap_or_default()
        };
        if !host.is_empty() {
            hosts.push(host.to_owned());
        }
    }
    for word in text.split_whitespace() {
        if !word.contains("://") && HOST_PORT.is_match(word) {
            hosts.push(word.split(':').next().unwrap_or_default().to_owned());
        }
    }
    hosts
}

/// Where a member's texts are looked at: env values (named) and commands.
fn texts(member: &Member) -> Vec<(String, &str, bool)> {
    let mut out: Vec<(String, &str, bool)> = member
        .env
        .iter()
        .map(|(name, value)| {
            let marked = {
                let upper = name.to_ascii_uppercase();
                NAME_MARKERS.iter().any(|m| upper.contains(m))
            };
            (format!("env {name}"), value.as_str(), marked)
        })
        .collect();
    out.extend(
        member
            .command
            .iter()
            .map(|c| ("cmd".to_owned(), c.as_str(), false)),
    );
    out
}

/// Whether `a` and `b` are the same directory or one is inside the other.
fn nested(a: &str, b: &str) -> bool {
    let inside = |outer: &str, inner: &str| {
        let outer = outer.trim_end_matches('/');
        inner == outer || inner.starts_with(&format!("{outer}/"))
    };
    inside(a, b) || inside(b, a)
}

/// Every edge between two of `all`.
pub fn edges(all: &[Member]) -> Vec<Edge> {
    let mut out: Vec<Edge> = Vec::new();
    let mut add = |from: &Member, to: &Member, kind: &'static str, signal: String, hardness| {
        let seen = out
            .iter()
            .any(|e| e.from == from.id && e.to == to.id && e.kind == kind);
        let mirrored = matches!(kind, "volume" | "bind")
            && out
                .iter()
                .any(|e| e.from == to.id && e.to == from.id && e.kind == kind);
        if !seen && !mirrored {
            out.push(Edge {
                from: from.id.clone(),
                to: to.id.clone(),
                kind,
                signal,
                hardness,
            });
        }
    };
    for a in all {
        for b in all.iter().filter(|b| b.id != a.id) {
            // Writable data both see.
            for ma in &a.mounts {
                for mb in &b.mounts {
                    if !(ma.writable || mb.writable) {
                        continue;
                    }
                    if ma.kind == "volume" && mb.kind == "volume" && ma.source == mb.source {
                        add(
                            a,
                            b,
                            "volume",
                            format!("volume {}", ma.source),
                            Hardness::Hard,
                        );
                    } else if ma.kind == "bind"
                        && mb.kind == "bind"
                        && nested(&ma.source, &mb.source)
                    {
                        add(
                            a,
                            b,
                            "bind",
                            format!("bind {}", ma.destination),
                            Hardness::Hard,
                        );
                    }
                }
            }
            let is_b = |reference: &str| {
                let name = reference.split(':').next().unwrap_or_default();
                name == b.id || name == b.name || b.compose_service() == Some(name)
            };
            if a.volumes_from.iter().any(|r| is_b(r)) {
                add(
                    a,
                    b,
                    "volumes_from",
                    "volumes_from".to_owned(),
                    Hardness::Hard,
                );
            }
            if let Some(mode) = a
                .modes
                .iter()
                .filter_map(|m| m.strip_prefix("container:"))
                .find(|r| *r == b.id || *r == b.name)
            {
                let _ = mode;
                add(
                    a,
                    b,
                    "network_mode",
                    "network_mode".to_owned(),
                    Hardness::Block,
                );
            }
            if a.links.iter().any(|r| is_b(r)) {
                add(a, b, "links", "links".to_owned(), Hardness::Soft);
            }
            if let Some(deps) = a.labels.get("com.docker.compose.depends_on")
                && a.compose_project() == b.compose_project()
                && let Some(service) = b.compose_service()
                && deps
                    .split(',')
                    .any(|d| d.split(':').next() == Some(service))
            {
                add(a, b, "depends_on", "depends_on".to_owned(), Hardness::Soft);
            }
            let ips: BTreeSet<&str> = b
                .networks
                .iter()
                .map(|n| n.ip.as_str())
                .filter(|ip| !ip.is_empty())
                .collect();
            let shared = a
                .networks
                .iter()
                .any(|na| b.networks.iter().any(|nb| na.network == nb.network));
            let names: BTreeSet<&str> = b
                .networks
                .iter()
                .filter(|nb| a.networks.iter().any(|na| na.network == nb.network))
                .flat_map(|n| n.aliases.iter().map(String::as_str))
                .chain([b.name.as_str()])
                .chain(b.compose_service())
                .collect();
            for (signal, text, marked) in texts(a) {
                if IPV4.find_iter(text).any(|m| ips.contains(m.as_str())) {
                    add(a, b, "ip", signal.clone(), Hardness::Block);
                }
                if !shared {
                    continue;
                }
                let mut hosts = host_tokens(text);
                if marked && !text.contains(['/', ' ', ':']) {
                    hosts.push(text.to_owned());
                }
                if hosts.iter().any(|h| names.contains(h.as_str())) {
                    let hardness = if a.compose_project() == b.compose_project() {
                        Hardness::Soft
                    } else {
                        Hardness::SoftOff
                    };
                    add(a, b, "dns", signal, hardness);
                }
            }
        }
    }
    out
}

/// The containers reachable from `seeds` over every edge but the ones left
/// off by default ([`Hardness::SoftOff`]).
pub fn closure(all: &[Member], seeds: &[String]) -> Closure {
    let found = edges(all);
    let known: BTreeSet<&str> = all.iter().map(|m| m.id.as_str()).collect();
    let mut members: Vec<String> = Vec::new();
    let mut queue: VecDeque<String> = VecDeque::new();
    let mut truncated = false;
    let mut take = |id: &str, members: &mut Vec<String>, queue: &mut VecDeque<String>| {
        if members.iter().any(|m| m == id) {
            return;
        }
        if members.len() >= MAX_MEMBERS {
            truncated = true;
            return;
        }
        members.push(id.to_owned());
        queue.push_back(id.to_owned());
    };
    for seed in seeds.iter().filter(|s| known.contains(s.as_str())) {
        take(seed, &mut members, &mut queue);
    }
    while let Some(id) = queue.pop_front() {
        for edge in found.iter().filter(|e| e.hardness != Hardness::SoftOff) {
            let other = if edge.from == id {
                &edge.to
            } else if edge.to == id {
                &edge.from
            } else {
                continue;
            };
            take(other, &mut members, &mut queue);
        }
    }
    let edges = found
        .into_iter()
        .filter(|e| members.contains(&e.from) && members.contains(&e.to))
        .collect();
    Closure {
        members,
        edges,
        truncated,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(id: &str, project: &str, service: &str) -> Member {
        Member {
            id: id.to_owned(),
            name: format!("{project}-{service}-1"),
            state: "running".to_owned(),
            labels: BTreeMap::from([
                ("com.docker.compose.project".to_owned(), project.to_owned()),
                ("com.docker.compose.service".to_owned(), service.to_owned()),
            ]),
            networks: vec![NetworkUse {
                network: format!("{project}_default"),
                aliases: vec![service.to_owned(), id.to_owned()],
                ip: format!("172.20.0.{}", id.len() + 1),
            }],
            ..Default::default()
        }
    }

    fn volume(name: &str, writable: bool) -> Mount {
        Mount {
            kind: "volume".to_owned(),
            source: name.to_owned(),
            destination: "/data".to_owned(),
            writable,
        }
    }

    fn kinds(edges: &[Edge]) -> Vec<(&str, &str, &'static str, Hardness)> {
        edges
            .iter()
            .map(|e| (e.from.as_str(), e.to.as_str(), e.kind, e.hardness))
            .collect()
    }

    #[test]
    fn a_shared_writable_volume_binds_hard_even_to_a_stopped_container() {
        let mut a = member("a", "p", "web");
        let mut b = member("b", "p", "worker");
        b.state = "exited".to_owned();
        a.mounts = vec![volume("shared", true)];
        b.mounts = vec![volume("shared", false)];
        let found = edges(&[a.clone(), b.clone()]);
        assert_eq!(
            kinds(&found),
            [("a", "b", "volume", Hardness::Hard)],
            "{found:?}"
        );
        // Both read-only: nothing writes, no edge.
        a.mounts = vec![volume("shared", false)];
        assert!(edges(&[a, b]).is_empty());
    }

    #[test]
    fn a_bind_inside_another_binds_hard() {
        let mut a = member("a", "p", "web");
        let mut b = member("b", "q", "tool");
        let bind = |source: &str, writable| Mount {
            kind: "bind".to_owned(),
            source: source.to_owned(),
            destination: "/x".to_owned(),
            writable,
        };
        a.mounts = vec![bind("/srv/data", true)];
        b.mounts = vec![bind("/srv/data/sub", false)];
        let found = edges(&[a, b]);
        assert_eq!(kinds(&found), [("a", "b", "bind", Hardness::Hard)]);
    }

    #[test]
    fn network_mode_container_and_a_hard_coded_ip_block() {
        let mut a = member("a", "p", "web");
        let mut c = member("c", "p", "db");
        a.modes = vec!["container:c".to_owned()];
        c.env = vec![];
        let mut d = member("d", "p", "app");
        d.env = vec![(
            "DB".to_owned(),
            "postgres://u:p@172.20.0.2:5432/x".to_owned(),
        )];
        // `a` has the IP 172.20.0.2 (id of length 1).
        let found = edges(&[a, c, d]);
        assert!(found.iter().any(|e| e.kind == "network_mode"
            && e.from == "a"
            && e.to == "c"
            && e.hardness == Hardness::Block));
        assert!(found.iter().any(|e| e.kind == "ip"
            && e.from == "d"
            && e.to == "a"
            && e.hardness == Hardness::Block));
    }

    #[test]
    fn a_dns_call_is_soft_and_across_projects_it_is_left_off() {
        let mut web = member("web1", "p", "web");
        let db = member("db1", "p", "db");
        let mut other = member("oth1", "q", "tika");
        web.env = vec![
            (
                "DATABASE_URL".to_owned(),
                "postgres://u:pw@db:5432/app".to_owned(),
            ),
            ("TIKA_ENDPOINT".to_owned(), "http://tika:9998".to_owned()),
            ("NOTE".to_owned(), "the db is nice".to_owned()),
        ];
        // `tika` is only reachable by a network they share.
        other.networks[0].network = "p_default".to_owned();
        let found = edges(&[web, db, other]);
        let dns: Vec<_> = found.iter().filter(|e| e.kind == "dns").collect();
        assert_eq!(dns.len(), 2, "{found:?}");
        let to_db = dns.iter().find(|e| e.to == "db1").unwrap();
        assert_eq!(to_db.hardness, Hardness::Soft);
        assert_eq!(to_db.signal, "env DATABASE_URL");
        let to_tika = dns.iter().find(|e| e.to == "oth1").unwrap();
        assert_eq!(to_tika.hardness, Hardness::SoftOff);
        // The value never appears in an edge.
        assert!(!format!("{found:?}").contains("pw"));
    }

    #[test]
    fn a_name_is_matched_only_at_a_host_position() {
        assert_eq!(host_tokens("postgres://u:pw@db:5432/app"), ["db"]);
        assert_eq!(
            host_tokens("redis-cli -h cache -p 6379"),
            Vec::<String>::new()
        );
        assert_eq!(host_tokens("cache:6379"), ["cache"]);
        assert_eq!(host_tokens("/usr/bin/db --flag"), Vec::<String>::new());
        assert_eq!(host_tokens("https://x.io/db"), ["x.io"]);
    }

    #[test]
    fn closure_follows_edges_but_not_the_ones_left_off() {
        let mut web = member("web1", "p", "web");
        let db = member("db1", "p", "db");
        let mut other = member("oth1", "q", "tika");
        web.env = vec![
            ("DATABASE_URL".to_owned(), "postgres://u:p@db/x".to_owned()),
            ("TIKA".to_owned(), "http://tika:9998".to_owned()),
        ];
        other.networks[0].network = "p_default".to_owned();
        let all = [web, db, other];
        let out = closure(&all, &["web1".to_owned()]);
        assert_eq!(out.members, ["web1", "db1"]);
        assert!(!out.truncated);
        // Seeded from the cross-project one: it brings nothing back in.
        let out = closure(&all, &["oth1".to_owned()]);
        assert_eq!(out.members, ["oth1"]);
    }

    #[test]
    fn closure_stops_at_the_cap() {
        let mut all: Vec<Member> = (0..60)
            .map(|i| member(&format!("m{i}"), "p", &format!("s{i}")))
            .collect();
        // A chain through volumes: m0 - m1 - m2 …
        for i in 0..59 {
            let name = format!("v{i}");
            all[i].mounts.push(volume(&name, true));
            all[i + 1].mounts.push(volume(&name, true));
        }
        let out = closure(&all, &["m0".to_owned()]);
        assert_eq!(out.members.len(), MAX_MEMBERS);
        assert!(out.truncated);
    }

    #[test]
    fn the_panels_own_containers_are_never_imported() {
        let panel = PanelFacts {
            database_ports: vec![3306],
            working_dir: Some("/Users/me/rdockpanel".to_owned()),
            ..Default::default()
        };
        let plain = member("a", "p", "web");
        assert_eq!(excluded(&plain, &panel), None);
        let mut system = member("b", "p", "x");
        system
            .labels
            .insert("rdockpanel.system".to_owned(), "true".to_owned());
        assert!(excluded(&system, &panel).is_some());
        let mut managed = member("c", "p", "x");
        managed
            .labels
            .insert("rdockpanel.managed".to_owned(), "true".to_owned());
        assert!(excluded(&managed, &panel).is_some());
        let mut task = member("d", "p", "x");
        task.labels
            .insert("com.docker.swarm.service.id".to_owned(), "abc".to_owned());
        assert!(excluded(&task, &panel).is_some());
        let mut db = member("e", "p", "x");
        db.labels
            .insert("rdockpanel.test".to_owned(), "1".to_owned());
        db.mounts = vec![Mount {
            kind: "bind".to_owned(),
            source: "/Users/me/rdockpanel/storage/panel.sqlite".to_owned(),
            destination: "/db".to_owned(),
            writable: true,
        }];
        assert!(excluded(&db, &panel).is_some());
    }

    #[test]
    fn a_member_reads_from_an_inspect_document() {
        let doc = serde_json::json!({
            "Id": "abc123",
            "Name": "/shop-web-1",
            "State": {"Status": "running"},
            "Config": {
                "Env": ["A=1", "B=x=y"],
                "Cmd": ["serve", "--port", "80"],
                "Entrypoint": ["/entry.sh"],
                "Healthcheck": {"Test": ["CMD-SHELL", "curl -f http://localhost"]},
                "Labels": {"com.docker.compose.project": "shop"}
            },
            "HostConfig": {"NetworkMode": "container:zzz", "PidMode": "", "VolumesFrom": ["db"], "Links": ["db:database"]},
            "Mounts": [
                {"Type": "volume", "Name": "data", "Destination": "/data", "RW": true},
                {"Type": "bind", "Source": "/srv/x", "Destination": "/x", "RW": false}
            ],
            "NetworkSettings": {"Networks": {"shop_default": {"Aliases": ["web"], "IPAddress": "172.20.0.5"}}}
        });
        let m = Member::from_inspect(&doc);
        assert_eq!(m.id, "abc123");
        assert_eq!(m.name, "shop-web-1");
        assert_eq!(m.state, "running");
        assert_eq!(
            m.env,
            [
                ("A".to_owned(), "1".to_owned()),
                ("B".to_owned(), "x=y".to_owned())
            ]
        );
        assert_eq!(m.command.len(), 3);
        assert_eq!(m.mounts[0], volume("data", true));
        assert_eq!(m.mounts[1].kind, "bind");
        assert!(!m.mounts[1].writable);
        assert_eq!(m.volumes_from, ["db"]);
        assert_eq!(m.links, ["db:database"]);
        assert_eq!(m.networks[0].ip, "172.20.0.5");
        assert!(m.modes.contains(&"container:zzz".to_owned()));
        assert_eq!(m.compose_project(), Some("shop"));
    }
}
