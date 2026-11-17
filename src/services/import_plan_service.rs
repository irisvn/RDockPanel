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
    pub health_timeout_ns: i64,
    pub health_start_period_ns: i64,
    /// What the panel carries over: capabilities, sysctls, groups, CPUs.
    pub cap_add: Vec<String>,
    pub cap_drop: Vec<String>,
    pub sysctls: BTreeMap<String, String>,
    pub group_add: Vec<String>,
    pub nano_cpus: i64,
    /// What it cannot (said in the plan): `(setting, value as said)`.
    pub uncarried: Vec<(String, String)>,
    /// `HostConfig.Memory` (bytes, 0: none).
    pub memory: i64,
    /// The image the container runs (`sha256:…`) and its `RepoDigests`.
    pub image_id: String,
    pub repo_digests: Vec<String>,
    /// The image's own environment (a pair equal to one here is left out).
    pub image_env: Vec<(String, String)>,
    pub ports: Vec<Port>,
    /// `Config.ExposedPorts` (TCP): the port a domain without one leads to.
    pub exposed: Vec<u16>,
    pub size_rw: Option<i64>,
    /// Named volumes: name → (driver, has options).
    pub volume_drivers: BTreeMap<String, (String, bool)>,
}

/// The settings of a container the panel does not carry over, as the plan
/// says them (`ShmSize`, ulimits, …): names and short values, never env.
fn uncarried(doc: &Value, image_doc: &Value) -> Vec<(String, String)> {
    let host = &doc["HostConfig"];
    let config = &doc["Config"];
    let mut out = Vec::new();
    let list = |v: &Value| -> Vec<String> {
        v.as_array()
            .into_iter()
            .flatten()
            .filter_map(|x| {
                x.as_str()
                    .map(str::to_owned)
                    .or_else(|| x["Name"].as_str().map(str::to_owned))
            })
            .collect()
    };
    if let Some(shm) = host["ShmSize"]
        .as_i64()
        .filter(|s| *s > 0 && *s != 64 * 1024 * 1024)
    {
        out.push((
            "shm_size".to_owned(),
            format!("{} MiB", shm / (1024 * 1024)),
        ));
    }
    let ulimits = list(&host["Ulimits"]);
    if !ulimits.is_empty() {
        out.push(("ulimits".to_owned(), ulimits.join(", ")));
    }
    if let Some(tmpfs) = host["Tmpfs"].as_object().filter(|t| !t.is_empty()) {
        out.push((
            "tmpfs".to_owned(),
            tmpfs.keys().cloned().collect::<Vec<_>>().join(", "),
        ));
    }
    let devices: Vec<String> = host["Devices"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|d| d["PathOnHost"].as_str().map(str::to_owned))
        .collect();
    if !devices.is_empty() {
        out.push(("devices".to_owned(), devices.join(", ")));
    }
    for (key, name) in [
        ("ExtraHosts", "extra_hosts"),
        ("Dns", "dns"),
        ("SecurityOpt", "security_opt"),
    ] {
        let values = list(&host[key]);
        if !values.is_empty() {
            out.push((name.to_owned(), values.join(", ")));
        }
    }
    if host["ReadonlyRootfs"].as_bool() == Some(true) {
        out.push(("read_only".to_owned(), "true".to_owned()));
    }
    if let Some(signal) = config["StopSignal"].as_str().filter(|s| {
        !s.is_empty()
            && *s != "SIGTERM"
            && *s
                != image_doc["Config"]["StopSignal"]
                    .as_str()
                    .unwrap_or("SIGTERM")
    }) {
        out.push(("stop_signal".to_owned(), signal.to_owned()));
    }
    if let Some(timeout) = config["StopTimeout"].as_i64() {
        out.push(("stop_grace_period".to_owned(), format!("{timeout}s")));
    }
    let workdir = config["WorkingDir"].as_str().unwrap_or_default();
    if !workdir.is_empty()
        && workdir
            != image_doc["Config"]["WorkingDir"]
                .as_str()
                .unwrap_or_default()
    {
        out.push(("working_dir".to_owned(), workdir.to_owned()));
    }
    if let Some(driver) = host["LogConfig"]["Type"]
        .as_str()
        .filter(|t| !t.is_empty() && *t != "json-file")
    {
        out.push(("logging".to_owned(), driver.to_owned()));
    }
    if host["Init"].as_bool() == Some(false) {
        out.push(("init".to_owned(), "off (the panel runs tini)".to_owned()));
    }
    out
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
            health_timeout_ns: doc["Config"]["Healthcheck"]["Timeout"]
                .as_i64()
                .unwrap_or(0),
            health_start_period_ns: doc["Config"]["Healthcheck"]["StartPeriod"]
                .as_i64()
                .unwrap_or(0),
            memory: doc["HostConfig"]["Memory"].as_i64().unwrap_or(0),
            cap_add: strings(&doc["HostConfig"]["CapAdd"]),
            cap_drop: strings(&doc["HostConfig"]["CapDrop"]),
            sysctls: doc["HostConfig"]["Sysctls"]
                .as_object()
                .map(|m| {
                    m.iter()
                        .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned())))
                        .collect()
                })
                .unwrap_or_default(),
            group_add: strings(&doc["HostConfig"]["GroupAdd"]),
            nano_cpus: doc["HostConfig"]["NanoCpus"].as_i64().unwrap_or(0),
            uncarried: uncarried(doc, image_doc),
            image_id: doc["Image"].as_str().unwrap_or_default().to_owned(),
            repo_digests: strings(&image_doc["RepoDigests"]),
            image_env,
            ports,
            exposed: doc["Config"]["ExposedPorts"]
                .as_object()
                .map(|map| {
                    map.keys()
                        .filter_map(|k| k.strip_suffix("/tcp")?.parse().ok())
                        .collect()
                })
                .unwrap_or_default(),
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

/// The token of a Cloudflare tunnel, from the environment (`TUNNEL_TOKEN`) or
/// the command line (`--token x`, `--token=x`): only one that looks like one.
/// A credentials file is not a token.
pub fn tunnel_token(env: &[(String, String)], args: &[String]) -> Option<String> {
    let from_env = env
        .iter()
        .find(|(name, _)| name == "TUNNEL_TOKEN")
        .map(|(_, value)| value.clone());
    let from_args = args.iter().enumerate().find_map(|(i, arg)| {
        arg.strip_prefix("--token=").map(str::to_owned).or_else(|| {
            (arg == "--token")
                .then(|| args.get(i + 1).cloned())
                .flatten()
        })
    });
    from_env
        .or(from_args)
        .filter(|token| crate::services::tunnel_service::valid_token(token))
}

/// A domain the source container was routed by (names only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DomainHint {
    pub host: String,
    pub path: String,
    pub port: Option<u16>,
    pub https: bool,
    /// Left out of the service: unticked, or the panel serves it already.
    pub skip: bool,
    /// The Traefik basic auth middleware that protects it: its users (read
    /// from Docker when the import starts, never kept in the plan) become the
    /// service's basic auth.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<String>,
    /// A protection the panel cannot carry over (forwardAuth, an allow list,
    /// a middleware defined elsewhere): the domain stays out of the service,
    /// it cannot be ticked, or the site would be served unprotected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guard: Option<String>,
    /// Middlewares that do not protect and are not carried over.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub lost: Vec<String>,
}

/// Middlewares that change a request or answer but guard nothing: lost on
/// import, said. Any other kind guards (or may): its domain stays out.
const HARMLESS_MIDDLEWARES: [&str; 18] = [
    "addprefix",
    "buffering",
    "circuitbreaker",
    "compress",
    "contenttype",
    "errors",
    "grpcweb",
    "headers",
    "inflightreq",
    "passtlsclientcert",
    "ratelimit",
    "redirectregex",
    "redirectscheme",
    "replacepath",
    "replacepathregex",
    "retry",
    "stripprefix",
    "stripprefixregex",
];

/// What a router's middlewares mean for its domains.
#[derive(Debug, Default, PartialEq, Eq)]
struct Guarding {
    auth: Option<String>,
    guard: Option<String>,
    lost: Vec<String>,
}

/// Reads `traefik.http.routers.<router>.middlewares` and the definitions of
/// those middlewares on the container itself (Traefik's option names are
/// case-insensitive).
fn guarding(member: &Member, router: &str) -> Guarding {
    let mut out = Guarding::default();
    let lower: Vec<(String, &str)> = member
        .labels
        .iter()
        .map(|(k, v)| (k.to_ascii_lowercase(), v.as_str()))
        .collect();
    let key = format!("traefik.http.routers.{router}.middlewares").to_ascii_lowercase();
    let Some(list) = lower.iter().find(|(k, _)| *k == key).map(|(_, v)| *v) else {
        return out;
    };
    let guard = |why: String, out: &mut Guarding| {
        out.guard.get_or_insert(why);
    };
    for raw in list.split(',').map(str::trim).filter(|m| !m.is_empty()) {
        let (name, provider) = match raw.split_once('@') {
            Some((name, provider)) => (name, Some(provider)),
            None => (raw, None),
        };
        if provider.is_some_and(|p| p != "docker") {
            guard(format!("{raw}: defined outside the container"), &mut out);
            continue;
        }
        let prefix = format!("traefik.http.middlewares.{name}.").to_ascii_lowercase();
        let options: Vec<(&str, &str)> = lower
            .iter()
            .filter_map(|(k, v)| Some((k.strip_prefix(&prefix)?, *v)))
            .collect();
        let kinds: BTreeSet<&str> = options
            .iter()
            .filter_map(|(rest, _)| rest.split('.').next())
            .collect();
        if kinds.is_empty() {
            guard(format!("{raw}: not defined on the container"), &mut out);
        }
        for kind in kinds {
            match kind {
                "basicauth" => {
                    let users = options.iter().any(|(k, _)| *k == "basicauth.users");
                    let file = options.iter().any(|(k, _)| *k == "basicauth.usersfile");
                    if !users || file {
                        guard(format!("{name}: basic auth from a file"), &mut out);
                    } else if out.auth.as_deref().is_some_and(|a| a != name) {
                        guard(format!("{name}: a second basic auth"), &mut out);
                    } else {
                        out.auth = Some(name.to_owned());
                    }
                }
                kind if HARMLESS_MIDDLEWARES.contains(&kind) => {
                    out.lost.push(format!("{name} ({kind})"));
                }
                kind => guard(format!("{name} ({kind})"), &mut out),
            }
        }
    }
    out
}

/// The users (`user:hash`) of a basic auth middleware defined on the
/// container, as Traefik reads them: a comma-separated list. None: missing,
/// or a hash Traefik and the panel do not take (bcrypt, MD5 `$apr1$`, SHA1).
pub fn basic_auth_users(member: &Member, middleware: &str) -> Option<Vec<(String, String)>> {
    let key = format!("traefik.http.middlewares.{middleware}.basicauth.users").to_ascii_lowercase();
    let value = member
        .labels
        .iter()
        .find(|(k, _)| k.to_ascii_lowercase() == key)
        .map(|(_, v)| v.as_str())?;
    let mut users = Vec::new();
    for entry in value.split(',').map(str::trim).filter(|e| !e.is_empty()) {
        let (user, hash) = entry.split_once(':')?;
        let known = ["$2y$", "$2a$", "$2b$", "$apr1$", "{SHA}"]
            .iter()
            .any(|p| hash.starts_with(p));
        if !known || !crate::services::deploy_service::valid_auth_user(user) || hash.len() > 200 {
            return None;
        }
        users.push((user.to_owned(), hash.to_owned()));
    }
    (!users.is_empty()).then_some(users)
}

/// A plain hostname: dotted labels of letters, digits and hyphens.
fn plain_host(host: &str) -> bool {
    host.contains('.')
        && host.split('.').all(|label| {
            !label.is_empty()
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
}

/// One `&&` branch of a rule as (hosts, paths); `None` when it holds anything
/// but plain `Host(...)` and `PathPrefix(...)` terms.
fn branch_of(branch: &str) -> Option<(Vec<String>, Vec<String>)> {
    let (mut hosts, mut paths) = (vec![], vec![]);
    for term in branch.split("&&").map(str::trim) {
        let inner = term.strip_suffix(')')?;
        let (name, args) = inner.split_once('(')?;
        let args: Vec<String> = args
            .split('`')
            .skip(1)
            .step_by(2)
            .map(str::to_owned)
            .collect();
        // Every argument is one backtick string, separated by commas.
        if args.is_empty() || inner.matches('`').count() != args.len() * 2 {
            return None;
        }
        match name {
            "Host" => hosts.extend(args.iter().map(|h| h.to_ascii_lowercase())),
            "PathPrefix" if paths.is_empty() => paths.extend(args),
            _ => return None,
        }
    }
    (!hosts.is_empty()
        && hosts.iter().all(|h| plain_host(h))
        && paths.iter().all(|p| p.starts_with('/')))
    .then_some((hosts, paths))
}

/// The domains a container was routed by (Traefik labels, `VIRTUAL_HOST`) and
/// the rules or entries that are not plain hosts (regex, wildcard, SNI, other
/// matchers): those are said, not kept. A rule is read branch by branch
/// (`||`), so one unsupported branch does not drop the others.
pub fn domains_of(member: &Member) -> (Vec<DomainHint>, Vec<String>) {
    let label = |key: &str| member.labels.get(key).map(String::as_str);
    let port_of = |value: Option<&str>| value.and_then(|v| v.trim().parse::<u16>().ok());
    let service_ports: Vec<(&str, &str)> = member
        .labels
        .iter()
        .filter_map(|(k, v)| {
            let name = k
                .strip_prefix("traefik.http.services.")?
                .strip_suffix(".loadbalancer.server.port")?;
            Some((name, v.as_str()))
        })
        .collect();
    let mut hints: Vec<DomainHint> = vec![];
    let mut notes: Vec<String> = vec![];
    // The same host and path from two routers: one domain, protected as the
    // stricter of them says.
    let push = |hints: &mut Vec<DomainHint>, hint: DomainHint| {
        if let Some(had) = hints
            .iter_mut()
            .find(|h| h.host == hint.host && h.path == hint.path)
        {
            if had.auth != hint.auth {
                had.guard
                    .get_or_insert_with(|| "two routers protect it differently".to_owned());
            }
            if had.guard.is_none() {
                had.guard = hint.guard;
            }
            had.skip = had.skip || had.guard.is_some();
        } else {
            hints.push(hint);
        }
    };
    for (key, rule) in &member.labels {
        let Some(router) = key
            .strip_prefix("traefik.http.routers.")
            .and_then(|k| k.strip_suffix(".rule"))
        else {
            continue;
        };
        // Traefik links a router to the only service when it names none.
        let port = match label(&format!("traefik.http.routers.{router}.service")) {
            Some(service) => {
                let service = service.split('@').next().unwrap_or(service);
                port_of(
                    service_ports
                        .iter()
                        .find(|(n, _)| *n == service)
                        .map(|(_, v)| *v),
                )
            }
            None if service_ports.len() == 1 => port_of(Some(service_ports[0].1)),
            None => port_of(
                service_ports
                    .iter()
                    .find(|(n, _)| *n == router)
                    .map(|(_, v)| *v),
            ),
        };
        let https = label(&format!("traefik.http.routers.{router}.tls")) == Some("true")
            || member
                .labels
                .keys()
                .any(|k| k.starts_with(&format!("traefik.http.routers.{router}.tls.")))
            || label(&format!("traefik.http.routers.{router}.entrypoints")).is_some_and(|e| {
                e.split(',')
                    .any(|e| matches!(e.trim(), "websecure" | "https"))
            });
        let guarded = guarding(member, router);
        for branch in rule.split("||").map(str::trim) {
            let Some((hosts, mut paths)) = branch_of(branch) else {
                notes.push(branch.to_owned());
                continue;
            };
            if paths.is_empty() {
                paths.push("/".to_owned());
            }
            for host in &hosts {
                for path in &paths {
                    let hint = DomainHint {
                        host: host.clone(),
                        path: path.clone(),
                        port,
                        https,
                        skip: guarded.guard.is_some(),
                        auth: guarded.auth.clone(),
                        guard: guarded.guard.clone(),
                        lost: guarded.lost.clone(),
                    };
                    push(&mut hints, hint);
                }
            }
        }
    }
    let env = |name: &str| {
        member
            .env
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    };
    let port = port_of(env("VIRTUAL_PORT"));
    for host in env("VIRTUAL_HOST")
        .unwrap_or_default()
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|h| !h.is_empty())
    {
        let lower = host.to_ascii_lowercase();
        if plain_host(&lower) {
            let hint = DomainHint {
                host: lower,
                path: "/".to_owned(),
                port,
                https: false,
                skip: false,
                auth: None,
                guard: None,
                lost: vec![],
            };
            push(&mut hints, hint);
        } else {
            notes.push(host.to_owned());
        }
    }
    // The service has one basic auth: the first one kept is it; a domain with
    // another stays out (its users would let the others in).
    let first = hints
        .iter()
        .filter(|h| !h.skip)
        .find_map(|h| h.auth.clone());
    for hint in hints.iter_mut().filter(|h| !h.skip) {
        if hint.auth.is_some() && hint.auth != first {
            hint.guard = Some("a basic auth other than the service's".to_owned());
            hint.skip = true;
        }
    }
    // Last, once every guard is known: a path guarded on a host: a domain of that host whose path covers it
    // (`/` covers `/admin`) would serve it unguarded. It stays out too.
    let guarded: Vec<(String, String)> = hints
        .iter()
        .filter(|h| h.guard.is_some())
        .map(|h| (h.host.clone(), h.path.clone()))
        .collect();
    for hint in hints.iter_mut().filter(|h| h.guard.is_none()) {
        // As Traefik's `PathPrefix` matches: a plain prefix (`/app` also
        // answers `/application`).
        let covers = guarded
            .iter()
            .any(|(host, path)| *host == hint.host && path.starts_with(&hint.path));
        if covers {
            hint.guard = Some("a path under it is protected".to_owned());
            hint.skip = true;
        }
    }
    (hints, notes)
}

/// Whether the container is a Cloudflare tunnel whose token is readable.
pub fn tunnel_option(detail: &Detail) -> bool {
    detail.image.to_ascii_lowercase().contains("cloudflared")
        && tunnel_token(&detail.member.env, &detail.cmd).is_some()
}

/// The values of the container's secret environment variables: a word equal
/// to one is a secret wherever it is.
pub fn known_secrets(detail: &Detail) -> Vec<String> {
    use crate::services::secret_scan_service::secret_env_name;
    detail
        .member
        .env
        .iter()
        .filter(|(name, value)| secret_env_name(name) && !value.is_empty())
        .map(|(_, value)| value.clone())
        .collect()
}

/// The secrets of the command line, as an import cuts them.
pub fn command_cut(detail: &Detail) -> crate::services::secret_scan_service::Cut {
    crate::services::secret_scan_service::cut_argv(
        &detail.cmd,
        &known_secrets(detail),
        &detail.image_entrypoint,
    )
}

/// The secrets of the health check, as an import cuts them: `CMD-SHELL` is a
/// shell line, `CMD` an argv (none: `NONE`, or no check).
pub fn health_cut(detail: &Detail) -> crate::services::secret_scan_service::Cut {
    use crate::services::secret_scan_service::{Cut, cut_argv, cut_shell};
    let known = known_secrets(detail);
    match detail.health_test.as_slice() {
        [kind, line] if kind == "CMD-SHELL" => cut_shell(line, &known, None),
        [kind, args @ ..] if kind == "CMD" && !args.is_empty() => cut_argv(args, &known, &[]),
        _ => Cut::Clean,
    }
}

/// How many secrets of the command line move to the environment (their
/// variables are `RDP_SECRET_<n>`, made when the services are).
pub fn command_secrets(detail: &Detail) -> usize {
    use crate::services::secret_scan_service::{ArgPart, Cut};
    match command_cut(detail) {
        Cut::Parts(parts) => parts
            .iter()
            .filter(|p| matches!(p, ArgPart::Secret { .. }))
            .count(),
        _ => 0,
    }
}

/// Whether an image's entrypoint runs the command it is given (so that a
/// command rewritten as `sh -c 'exec …'` still reaches the program): none, the
/// official images' `docker-entrypoint.sh` (it runs what it does not know),
/// or an init that execs its arguments. A program as entrypoint would get
/// `sh` as its first argument.
fn runs_its_arguments(entrypoint: &[String]) -> bool {
    let base = |s: &str| s.rsplit('/').next().unwrap_or(s).to_owned();
    match entrypoint {
        [] => true,
        [only] => base(only) == "docker-entrypoint.sh",
        [init, dashes] => matches!(base(init).as_str(), "tini" | "dumb-init") && dashes == "--",
        _ => false,
    }
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
    // A secret of the command line or the health check moves to the
    // environment (the command reads it there); one that cannot be set apart
    // blocks the command, and asks the admin for the health check.
    use crate::services::secret_scan_service::Cut;
    // Longer than the service keeps, as it is or once rewritten: the admin
    // drops it or types a shorter one.
    // Measured on the very line the import writes.
    let health_len = match crate::services::import_service::health_line(detail, &|_| false) {
        crate::services::import_service::HealthLine::Line(line, _) => line.len(),
        _ => 0,
    };
    let too_long = health_len > crate::services::import_service::HEALTH_MAX;
    if too_long {
        out.push(finding(
            "healthcheck_long",
            Level::Block,
            "healthcheck",
            "its health check is longer than the panel keeps (500 characters)",
            Some("In its details, drop the health check or type a shorter one."),
        ));
    }
    match health_cut(detail) {
        _ if too_long => {}
        Cut::Clean => {}
        Cut::Parts(_) => out.push(finding(
            "healthcheck_secret_moved",
            Level::Confirm,
            "healthcheck",
            "its health check holds a password: it moves to an environment variable the check reads",
            Some("Confirm."),
        )),
        Cut::Stuck => out.push(finding(
            "healthcheck_secret",
            Level::Block,
            "healthcheck",
            "its health check holds a secret the panel cannot set apart",
            Some("In its details, drop the health check or type a new one."),
        )),
    }
    match command_cut(detail) {
        Cut::Clean => {}
        Cut::Parts(_) if runs_its_arguments(&detail.image_entrypoint) => {
            // The panel moves it: into an environment variable the command reads.
            out.push(finding(
                "argv_secret_moved",
                Level::Confirm,
                "command",
                "its command line holds a secret: it moves to an environment variable and the command runs through `sh -c`",
                Some("Confirm; the image needs a `/bin/sh`."),
            ));
        }
        _ => out.push(finding(
            "argv_secret",
            Level::Block,
            "command",
            "its command line holds a secret",
            Some("Make it read the secret from an environment variable, then read again."),
        )),
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
    // Run with `--entrypoint ""` over an image entrypoint: the panel cannot
    // clear it, the program would differ.
    if detail.entrypoint.is_empty() && !detail.image_entrypoint.is_empty() {
        out.push(finding(
            "entrypoint_cleared",
            Level::Block,
            "entrypoint",
            "it runs without its image's entrypoint: the panel cannot clear an entrypoint",
            Some("Leave this container outside the panel."),
        ));
    }
    for (setting, value) in &detail.uncarried {
        out.push(finding(
            "setting_lost",
            Level::Warn,
            &format!("{setting}: {value}"),
            "a setting of the container the panel does not carry over",
            None,
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
    // A host place no service of the panel may have (the Docker socket,
    // /proc, the installer's folder, `/`): the rules of bind mounts hold.
    for mount in m.mounts.iter().filter(|m| m.kind == "bind") {
        if crate::services::deploy_rules_service::forbidden_path(
            std::path::Path::new(&mount.source),
            &panel.state_dir,
        ) {
            out.push(finding(
                "forbidden_mount",
                Level::Block,
                &format!("mount {}", mount.source),
                "it mounts a place of the host the panel never gives a service (the Docker socket, /proc, the panel's own folder…)",
                Some("Leave this container outside the panel, or remove that mount from it first."),
            ));
        }
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
        } else if if panel.reserved_ports.is_empty() {
            RESERVED_PORTS.contains(&port.host_port)
        } else {
            panel.reserved_ports.contains(&port.host_port)
        } {
            out.push(finding(
                "port_reserved",
                Level::Confirm,
                &format!("port {}", port.host_port),
                "the panel keeps this port: it is not published; reach the service by a domain, or publish it on another port (its details)",
                Some("Confirm, or choose another port in its details."),
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
    /// A Cloudflare tunnel whose token can be read: it may run as a tunnel of
    /// the panel (Settings → Cloudflare Tunnel) instead of a service.
    pub tunnel_token: bool,
    /// Chosen to run as a tunnel of the panel: no service is made.
    pub as_tunnel: bool,
    /// Domains it was routed by: names only, added to the service when ticked.
    pub domains: Vec<DomainHint>,
    /// Rules that are not plain hosts: said, not kept.
    pub skip_domains: Vec<String>,
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

/// Blocks again, in a saved plan (JSON), the hard groups of the members a
/// finding blocks, after a choice changed them: a group is blocked by any
/// member it cannot take, and taken again once none is left.
pub fn regroup(plan: &mut serde_json::Value) {
    use serde_json::json;
    let Some(items) = plan["items"].as_array_mut() else {
        return;
    };
    for item in items.iter_mut() {
        if let Some(findings) = item["findings"].as_array_mut() {
            findings.retain(|f| f["code"] != "group_blocked");
        }
        let own_block = item["findings"]
            .as_array()
            .is_some_and(|f| f.iter().any(|f| f["level"] == "BLOCK"));
        if item["choice"] == "blocked" && !own_block {
            item["choice"] = json!("bring");
        }
        if own_block {
            item["choice"] = json!("blocked");
        }
    }
    let blocked: BTreeSet<u64> = items
        .iter()
        .filter(|i| i["choice"] == "blocked")
        .filter_map(|i| i["hard_group"].as_u64())
        .collect();
    for item in items.iter_mut() {
        let in_blocked = item["hard_group"]
            .as_u64()
            .is_some_and(|g| blocked.contains(&g));
        if in_blocked && item["choice"] != "blocked" {
            item["choice"] = json!("blocked");
            if let Some(findings) = item["findings"].as_array_mut() {
                findings.push(json!({
                    "code": "group_blocked",
                    "level": "BLOCK",
                    "field": "group",
                    "text": "it shares writable data or a network with a container that cannot be imported",
                    "how_to_unblock": null
                }));
            }
        }
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
                    tunnel_token: false,
                    as_tunnel: false,
                    domains: vec![],
                    skip_domains: vec![],
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
            let (mut domains, skip_domains) = domains_of(m);
            // As Traefik and nginx-proxy do: the only exposed port, else 80.
            let fallback = match detail.exposed.as_slice() {
                [only] => *only,
                _ => 80,
            };
            for hint in domains.iter_mut().filter(|h| h.port.is_none()) {
                hint.port = Some(fallback);
            }
            let service_auth = domains.iter().filter(|h| !h.skip).find_map(|h| h.auth.clone());
            for hint in &domains {
                let field = if hint.path == "/" {
                    hint.host.clone()
                } else {
                    format!("{}{}", hint.host, hint.path)
                };
                if let Some(why) = &hint.guard {
                    found.push(finding(
                        "domain_protected",
                        Level::Warn,
                        &format!("{field} ({why})"),
                        "a Traefik protection the panel cannot carry over guards it: it is left out, or the site would be open",
                        Some("Protect the service the same way in the panel, then add the domain."),
                    ));
                } else if !hint.lost.is_empty() {
                    found.push(finding(
                        "domain_middleware_lost",
                        Level::Info,
                        &format!("{field} ({})", hint.lost.join(", ")),
                        "Traefik middlewares that are not carried over (they protect nothing)",
                        None,
                    ));
                }
            }
            if service_auth.is_some() {
                found.push(finding(
                    "domain_basic_auth",
                    Level::Info,
                    "basic auth",
                    "its basic auth users come along, and guard every domain of the service",
                    None,
                ));
            }
            for rule in &skip_domains {
                found.push(finding(
                    "domain_rule",
                    Level::Warn,
                    rule,
                    "a routing rule that is not a plain host is not kept",
                    None,
                ));
            }
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
                tunnel_token: tunnel_option(detail),
                as_tunnel: false,
                domains,
                skip_domains,
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
    fn a_path_under_guard_keeps_the_domains_that_cover_it_out() {
        let mut d = detail("a", "web");
        for (k, v) in [
            ("traefik.http.routers.site.rule", "Host(`x.io`)"),
            (
                "traefik.http.routers.admin.rule",
                "Host(`x.io`) && PathPrefix(`/admin`)",
            ),
            ("traefik.http.routers.admin.middlewares", "sso"),
            (
                "traefik.http.middlewares.sso.forwardauth.address",
                "http://auth",
            ),
            ("traefik.http.routers.app.rule", "Host(`y.io`)"),
            ("traefik.http.routers.app.middlewares", "cut"),
            ("traefik.http.middlewares.cut.stripprefix.prefixes", "/app"),
        ] {
            d.member.labels.insert(k.to_owned(), v.to_owned());
        }
        let (hints, _) = domains_of(&d.member);
        let of = |host: &str, path: &str| {
            hints
                .iter()
                .find(|h| h.host == host && h.path == path)
                .unwrap()
        };
        // `/` would answer `/admin` without its forwardAuth.
        assert!(of("x.io", "/admin").skip);
        assert!(of("x.io", "/").skip);
        // stripPrefix guards nothing: kept, said as lost.
        assert!(!of("y.io", "/").skip);
        assert_eq!(of("y.io", "/").lost.len(), 1);
    }

    #[test]
    fn a_route_covers_what_its_plain_prefix_matches_and_a_second_basic_auth() {
        let mut d = detail("a", "web");
        for (k, v) in [
            (
                "traefik.http.routers.app.rule",
                "Host(`x.io`) && PathPrefix(`/app`)",
            ),
            (
                "traefik.http.routers.apps.rule",
                "Host(`x.io`) && PathPrefix(`/application`)",
            ),
            ("traefik.http.routers.apps.middlewares", "sso"),
            (
                "traefik.http.middlewares.sso.forwardauth.address",
                "http://auth",
            ),
            ("traefik.http.routers.site.rule", "Host(`y.io`)"),
            ("traefik.http.routers.site.middlewares", "a"),
            ("traefik.http.middlewares.a.basicauth.users", "u:$apr1$x$y"),
            (
                "traefik.http.routers.adm.rule",
                "Host(`y.io`) && PathPrefix(`/admin`)",
            ),
            ("traefik.http.routers.adm.middlewares", "b"),
            ("traefik.http.middlewares.b.basicauth.users", "v:$apr1$x$z"),
        ] {
            d.member.labels.insert(k.to_owned(), v.to_owned());
        }
        let (hints, _) = domains_of(&d.member);
        let of = |host: &str, path: &str| {
            hints
                .iter()
                .find(|h| h.host == host && h.path == path)
                .unwrap()
        };
        // Traefik's `/app` answers `/application` too.
        assert!(of("x.io", "/app").skip);
        // Two basic auths: one is the service's, the other domain stays out;
        // never `/` kept while `/admin` (under it) is left out.
        let (root, admin) = (of("y.io", "/"), of("y.io", "/admin"));
        assert!(root.skip != admin.skip, "{hints:?}");
        assert!(!(admin.skip && !root.skip), "{hints:?}");
    }

    #[test]
    fn a_secret_in_the_command_of_a_program_entrypoint_blocks() {
        let panel = PanelFacts::default();
        let mut d = detail("a", "app");
        d.cmd = vec!["--password".to_owned(), "s3cr3t-value".to_owned()];
        // The program would get `sh` as its first argument.
        d.image_entrypoint = vec!["/app/server".to_owned()];
        d.entrypoint = d.image_entrypoint.clone();
        let found = findings(&d, &panel);
        assert!(codes(&found).contains(&"argv_secret"), "{found:?}");
        assert!(!codes(&found).contains(&"argv_secret_moved"));
        // The official images' entrypoint runs what it is given.
        d.image_entrypoint = vec!["docker-entrypoint.sh".to_owned()];
        d.entrypoint = d.image_entrypoint.clone();
        assert!(codes(&findings(&d, &panel)).contains(&"argv_secret_moved"));
    }

    #[test]
    fn a_secret_in_the_health_check_moves_or_waits_for_the_admin() {
        let panel = PanelFacts::default();
        let mut d = detail("a", "cache");
        d.member.env = vec![("REDIS_PASSWORD".to_owned(), "hunter22".to_owned())];
        d.health_test = vec![
            "CMD-SHELL".to_owned(),
            "redis-cli -a hunter22 ping".to_owned(),
        ];
        let found = findings(&d, &panel);
        // `redis-cli -a <password>`: moved to the environment, confirmed.
        let f = found
            .iter()
            .find(|f| f.code == "healthcheck_secret_moved")
            .unwrap();
        assert_eq!(f.level, Level::Confirm);
        assert!(!format!("{found:?}").contains("hunter22"));
        // One the panel cannot set apart blocks until the admin chooses.
        d.health_test[1] = "curl -f http://u:hunter22@localhost/".to_owned();
        let found = findings(&d, &panel);
        let f = found
            .iter()
            .find(|f| f.code == "healthcheck_secret")
            .unwrap();
        assert_eq!(f.level, Level::Block);
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
    fn a_cloudflared_with_a_token_can_become_the_panels_tunnel() {
        let token = "eyJhIjoiMDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWYiLCJ0IjoiMDEyMzQ1NjctODlhYi1jZGVmLTAxMjMtNDU2Nzg5YWJjZGVmIiwicyI6ImMyVmpjbVYwTFhObFkzSmxkQzF6WldOeVpYUT0ifQ==";
        // In the environment.
        let env = vec![("TUNNEL_TOKEN".to_owned(), token.to_owned())];
        assert_eq!(tunnel_token(&env, &[]).as_deref(), Some(token));
        // On the command line, both forms.
        let args = ["tunnel", "--no-autoupdate", "run", "--token", token].map(str::to_owned);
        assert_eq!(tunnel_token(&[], &args).as_deref(), Some(token));
        let joined = ["tunnel", "run", &format!("--token={token}")].map(str::to_owned);
        assert_eq!(tunnel_token(&[], &joined).as_deref(), Some(token));
        // A credentials file or a token that is not one: none.
        let file = ["tunnel", "run", "--credentials-file", "/etc/c.json"].map(str::to_owned);
        assert_eq!(tunnel_token(&[], &file), None);
        let bad = vec![("TUNNEL_TOKEN".to_owned(), "nope".to_owned())];
        assert_eq!(tunnel_token(&bad, &[]), None);
        // The plan says so, only for a cloudflared, and never shows the token.
        let mut d = detail("a", "tunnel");
        d.image = "cloudflare/cloudflared:2026.10.2".to_owned();
        d.member.env = env;
        let c = closure(&[d.member.clone()], &["a".to_owned()]);
        let plan = plan_of(&[d.clone()], &c, "a", "p", &PanelFacts::default(), "fp");
        assert!(plan.items[0].tunnel_token);
        assert!(!plan.items[0].as_tunnel);
        assert!(!serde_json::to_string(&plan).unwrap().contains(token));
        d.image = "nginx:1".to_owned();
        let plan = plan_of(&[d.clone()], &c, "a", "p", &PanelFacts::default(), "fp");
        assert!(!plan.items[0].tunnel_token);
    }

    fn hint(host: &str, path: &str, port: Option<u16>, https: bool) -> DomainHint {
        DomainHint {
            host: host.to_owned(),
            path: path.to_owned(),
            port,
            https,
            skip: false,
            auth: None,
            guard: None,
            lost: vec![],
        }
    }

    #[test]
    fn reads_hosts_paths_and_port_from_traefik_labels() {
        let mut d = detail("a", "web");
        for (k, v) in [
            (
                "traefik.http.routers.a.rule",
                "Host(`x.io`) || Host(`y.io`) && PathPrefix(`/app`)",
            ),
            ("traefik.http.routers.a.tls", "true"),
            ("traefik.http.services.a.loadbalancer.server.port", "3000"),
        ] {
            d.member.labels.insert(k.to_owned(), v.to_owned());
        }
        let (hints, notes) = domains_of(&d.member);
        assert_eq!(
            hints,
            vec![
                hint("x.io", "/", Some(3000), true),
                hint("y.io", "/app", Some(3000), true)
            ]
        );
        assert!(notes.is_empty());
    }

    #[test]
    fn a_regex_host_is_not_a_domain() {
        let mut d = detail("a", "web");
        d.member.labels.insert(
            "traefik.http.routers.a.rule".to_owned(),
            "HostRegexp(`{s:.+}.io`)".to_owned(),
        );
        let (hints, notes) = domains_of(&d.member);
        assert!(hints.is_empty());
        assert_eq!(notes.len(), 1);
        let c = closure(&[d.member.clone()], &["a".to_owned()]);
        let plan = plan_of(&[d], &c, "a", "p", &PanelFacts::default(), "fp");
        assert!(codes(&plan.items[0].findings).contains(&"domain_rule"));
        assert_eq!(plan.items[0].skip_domains, notes);
    }

    #[test]
    fn a_domain_without_a_port_leads_to_the_only_exposed_one_else_80() {
        let mut d = detail("a", "web");
        d.member.env = vec![("VIRTUAL_HOST".to_owned(), "a.io".to_owned())];
        let c = closure(&[d.member.clone()], &["a".to_owned()]);
        let port = |d: &Detail| {
            plan_of(
                std::slice::from_ref(d),
                &c,
                "a",
                "p",
                &PanelFacts::default(),
                "fp",
            )
            .items[0]
                .domains[0]
                .port
        };
        assert_eq!(port(&d), Some(80));
        d.exposed = vec![3000];
        assert_eq!(port(&d), Some(3000));
        d.exposed = vec![3000, 9000];
        assert_eq!(port(&d), Some(80));
    }

    #[test]
    fn virtual_host_env_gives_domains() {
        let mut d = detail("a", "web");
        d.member.env = vec![
            ("VIRTUAL_HOST".to_owned(), "a.io,b.io".to_owned()),
            ("VIRTUAL_PORT".to_owned(), "8080".to_owned()),
        ];
        let (hints, _) = domains_of(&d.member);
        assert_eq!(
            hints,
            vec![
                hint("a.io", "/", Some(8080), false),
                hint("b.io", "/", Some(8080), false)
            ]
        );
    }

    fn with_labels(pairs: &[(&str, &str)]) -> Member {
        let mut d = detail("a", "web");
        for (k, v) in pairs {
            d.member.labels.insert((*k).to_owned(), (*v).to_owned());
        }
        d.member
    }

    #[test]
    fn or_branches_keep_their_own_paths() {
        let m = with_labels(&[(
            "traefik.http.routers.a.rule",
            "Host(`a.io`) && PathPrefix(`/x`) || Host(`b.io`) && PathPrefix(`/y`)",
        )]);
        let (hints, notes) = domains_of(&m);
        assert_eq!(
            hints,
            vec![
                hint("a.io", "/x", None, false),
                hint("b.io", "/y", None, false)
            ]
        );
        assert!(notes.is_empty());
    }

    #[test]
    fn virtual_host_entries_must_be_plain_hostnames() {
        let mut d = detail("a", "web");
        d.member.env = vec![(
            "VIRTUAL_HOST".to_owned(),
            "a.io ~^app\\..*$ *.example.com b.io,localhost".to_owned(),
        )];
        let (hints, notes) = domains_of(&d.member);
        assert_eq!(
            hints,
            vec![
                hint("a.io", "/", None, false),
                hint("b.io", "/", None, false)
            ]
        );
        assert_eq!(notes.len(), 3);
    }

    #[test]
    fn the_only_service_port_is_used_and_provider_suffix_is_dropped() {
        let m = with_labels(&[
            ("traefik.http.routers.r.rule", "Host(`a.io`)"),
            ("traefik.http.services.web.loadbalancer.server.port", "81"),
        ]);
        assert_eq!(domains_of(&m).0[0].port, Some(81));
        let m = with_labels(&[
            ("traefik.http.routers.r.rule", "Host(`a.io`)"),
            ("traefik.http.routers.r.service", "web@docker"),
            ("traefik.http.services.web.loadbalancer.server.port", "81"),
            ("traefik.http.services.other.loadbalancer.server.port", "82"),
        ]);
        assert_eq!(domains_of(&m).0[0].port, Some(81));
    }

    #[test]
    fn one_unsupported_branch_does_not_drop_the_rule() {
        let m = with_labels(&[(
            "traefik.http.routers.a.rule",
            "Host(`A.io`) || HostRegexp(`x`)",
        )]);
        let (hints, notes) = domains_of(&m);
        assert_eq!(hints, vec![hint("a.io", "/", None, false)]);
        assert_eq!(notes.len(), 1);
    }

    #[test]
    fn empty_host_path_and_negated_matchers_are_notes() {
        for rule in [
            "Host(``)",
            "Host(`a.io`) && Path(`/p`)",
            "Host(`a.io`) && !PathPrefix(`/p`)",
        ] {
            let (hints, notes) = domains_of(&with_labels(&[("traefik.http.routers.a.rule", rule)]));
            assert!(hints.is_empty(), "{rule}");
            assert_eq!(notes.len(), 1, "{rule}");
        }
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
