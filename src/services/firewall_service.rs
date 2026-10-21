//! The Firewall page: what the server listens on, and its firewall: default
//! deny inbound in the panel's own nftables table (`inet rdockpanel`), SSH,
//! 80 and 443 always open, the openings for their sources only. Each change
//! is a transaction the host journals first: applied with its revert armed,
//! then kept by the user or reverted by the host itself (docs/firewall.md).
//!
//! The host is read and changed by fixed scripts run in its namespaces
//! (`nsenter -t 1`) from a helper container of the panel's own image.
//! Installed panels only (from source, the "host" is the developer's machine).

use std::sync::LazyLock;
use std::time::Duration;

use chrono::Utc;
use futures_util::StreamExt;
use rustcms::{AppError, AppResult, AppState, ValidationErrors};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set};
use serde::Serialize;
use serde_json::{Value, json};

use crate::models::{firewall_blocked, firewall_opening, firewall_transaction, panel_setting};
use crate::services::action_service::{self, Initiator};
use crate::services::docker_service::{Docker, ExecChunk};
use crate::services::firewall_host::{self as host, fw};
use crate::services::firewall_rules::{
    self as rules, Cidr, HostFacts, Opening, Policy, Protocol, Target,
};

/// The most a host command's output is kept (each stream).
const OUTPUT_MAX: usize = 256 * 1024;
/// How long a host command may take.
const HOST_TIME: Duration = Duration::from_secs(45);
/// How long the user has to keep a change (the host reverts it at 120 s).
const KEEP_WINDOW: i64 = 110;
/// Swarm's own ports: cluster control, gossip, overlay data (VXLAN).
pub const SWARM_PORTS: [(u16, &str); 4] =
    [(2377, "tcp"), (7946, "tcp"), (7946, "udp"), (4789, "udp")];
/// The confirmed table's canonical hash (drift is measured against it).
const CONFIRMED_HASH: &str = "firewall.confirmed_hash";

/// One change at a time on this instance (the host has its own lock).
static LOCK: LazyLock<tokio::sync::Mutex<()>> = LazyLock::new(|| tokio::sync::Mutex::new(()));
/// Docker networks were created or removed: the trusted bridges change.
pub static NETWORKS_CHANGED: LazyLock<tokio::sync::Notify> =
    LazyLock::new(tokio::sync::Notify::new);
/// The bridges last given to the host.
static SENT_IFACES: LazyLock<std::sync::Mutex<Option<Vec<String>>>> =
    LazyLock::new(|| std::sync::Mutex::new(None));

/// What a host command printed, and how it ended.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct HostOutput {
    pub stdout: String,
    pub stderr: String,
    /// None: it did not end in time (its outcome unknown).
    pub code: Option<i64>,
}

/// Runs `script` (`sh -c`) in the host's namespaces, `input` on its standard
/// input, within `limit`: a helper container of `image`, removed whatever
/// happens.
pub async fn host_run(
    docker: &Docker,
    image: &str,
    script: &str,
    input: Option<Vec<u8>>,
    limit: Duration,
) -> Result<HostOutput, String> {
    let engine = docker.engine();
    // Idle a little longer than the command may take: never left behind.
    let lifetime = u32::try_from(limit.as_secs() + 30).unwrap_or(u32::MAX);
    let helper = engine
        .start_host_helper(image, lifetime)
        .await
        .map_err(|e| e.to_string())?;
    let cmd = [
        "nsenter", "-t", "1", "-m", "-u", "-i", "-n", "-p", "--", "sh", "-c", script,
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    let input = input.map(|bytes| futures_util::stream::iter([bytes]).boxed());
    let mark = format!(
        "rdp-host-{}",
        crate::services::database_service::random_string(12)
    );
    let run = async {
        let mut out = HostOutput::default();
        let mut stream = engine
            .run_in_container(&helper, cmd, &mark, input)
            .await
            .map_err(|e| e.to_string())?;
        while let Some(chunk) = stream.next().await {
            match chunk.map_err(|e| e.to_string())? {
                ExecChunk::Out(bytes) => append(&mut out.stdout, &bytes),
                ExecChunk::Err(bytes) => append(&mut out.stderr, &bytes),
                ExecChunk::Exit(code) => out.code = Some(code),
            }
        }
        Ok::<_, String>(out)
    };
    let result = match tokio::time::timeout(limit, run).await {
        Ok(result) => result,
        Err(_) => Ok(HostOutput {
            code: None,
            ..Default::default()
        }),
    };
    let _ = engine.remove_container(&helper, true).await;
    result
}

fn append(into: &mut String, bytes: &[u8]) {
    let room = OUTPUT_MAX.saturating_sub(into.len());
    into.push_str(&String::from_utf8_lossy(&bytes[..bytes.len().min(room)]));
}

/// What the read-only discovery prints, one section after another.
pub const DISCOVER: &str = r#"
echo '== ssh'
sshd -T 2>/dev/null | awk '$1 == "port" { print $2 }'
systemctl show -p Listen ssh.socket 2>/dev/null
echo '== listen'
ss -H -ltnup 2>/dev/null
echo '== tools'
command -v nft >/dev/null 2>&1 && echo nft=yes || echo nft=no
command -v systemd-run >/dev/null 2>&1 && echo systemd=yes || echo systemd=no
echo "ufw=$(systemctl is-active ufw 2>/dev/null)"
echo "ufw_status=$(ufw status 2>/dev/null | head -n 1)"
echo "firewalld=$(systemctl is-active firewalld 2>/dev/null)"
echo "nftables=$(systemctl is-enabled nftables 2>/dev/null)"
echo "nodes=$(docker info --format '{{.Swarm.Nodes}}' 2>/dev/null)"
echo '== drivers'
docker network ls --format '{{.Driver}}' 2>/dev/null | sort -u
echo '== bridges'
for id in $(docker network ls -q --filter driver=bridge 2>/dev/null); do
  docker network inspect -f '{{.Id}}|{{index .Options "com.docker.network.bridge.name"}}|{{index .Options "com.docker.network.bridge.gateway_mode_ipv4"}}|{{index .Options "com.docker.network.bridge.gateway_mode_ipv6"}}' "$id" 2>/dev/null
done
echo '== route'
ip -o route show default 2>/dev/null | awk '{ for (i = 1; i <= NF; i++) if ($i == "dev") print $(i + 1) }'
ip -o -6 route show default 2>/dev/null | awk '{ for (i = 1; i <= NF; i++) if ($i == "dev") print $(i + 1) }'
echo '== fw'
if [ -x /var/lib/rdockpanel/firewall/fw.sh ]; then sh /var/lib/rdockpanel/firewall/fw.sh status; else echo installed=no; fi
echo '== end'
"#;

/// A socket the host listens on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Listener {
    pub protocol: String,
    pub address: String,
    pub port: u16,
    /// The process (`sshd`, `docker-proxy`, `dockerd`), when `ss` names it.
    pub process: Option<String>,
}

impl Listener {
    /// Only this server reaches it (a loopback address).
    pub fn local(&self) -> bool {
        let address = self.address.split('%').next().unwrap_or("");
        address.starts_with("127.") || address == "::1"
    }
}

/// The host's firewall files (`fw.sh status`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct HostState {
    /// fw.sh is installed.
    pub installed: bool,
    /// A confirmed table exists (the firewall is on).
    pub confirmed: bool,
    /// The last transaction and its state.
    pub last: Option<(String, String)>,
    /// What the boot loader could not load (`confirmed`: the fallback is
    /// on; `both`: neither, Docker held).
    pub boot_failed: Option<String>,
    /// `nft -j list table inet rdockpanel` (None: no table).
    #[serde(skip)]
    pub table: Option<String>,
}

/// What the discovery found.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Discovery {
    pub ssh_ports: Vec<u16>,
    pub listeners: Vec<Listener>,
    pub nft: bool,
    pub systemd: bool,
    pub ufw_active: bool,
    pub firewalld_active: bool,
    /// `nftables.service` enabled (its stock config flushes every table).
    pub nftables_enabled: bool,
    /// Swarm's nodes (0: not in a swarm; None: unknown).
    pub nodes: Option<u32>,
    /// The drivers of the host's Docker networks.
    pub drivers: Vec<String>,
    /// Docker's bridges (the kernel's interface names).
    pub bridges: Vec<String>,
    /// Bridges whose gateway is not NAT (routed: reached without DNAT,
    /// past the firewall's published-port rules).
    pub routed: Vec<String>,
    /// The interfaces of the default routes (the public side).
    pub public_ifaces: Vec<String>,
    pub host: HostState,
}

impl Discovery {
    /// The bridges trusted by the firewall: Docker's, never a public one.
    pub fn docker_ifaces(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .bridges
            .iter()
            .filter(|n| rules::valid_ifname(n) && !self.public_ifaces.contains(n))
            .cloned()
            .collect();
        names.sort();
        names.dedup();
        names
    }

    /// Why the firewall may not be turned on here (empty: it may).
    pub fn unsupported(&self) -> Vec<String> {
        let mut why = vec![];
        if !self.nft {
            why.push("nft is not installed on the server".to_owned());
        }
        if !self.systemd {
            why.push("the server does not run systemd".to_owned());
        }
        if self.nodes.is_some_and(|n| n > 1) {
            why.push("this Swarm has more than one node (not supported yet)".to_owned());
        }
        for driver in &self.drivers {
            if matches!(driver.as_str(), "macvlan" | "ipvlan") {
                why.push(format!("a {driver} Docker network bypasses the firewall"));
            }
        }
        if !self.routed.is_empty() {
            why.push(format!(
                "Docker networks without NAT bypass the firewall: {}",
                self.routed.join(", ")
            ));
        }
        if self.nftables_enabled {
            why.push(
                "nftables.service is enabled: its configuration flushes every table at boot"
                    .to_owned(),
            );
        }
        if self.bridges.iter().any(|b| self.public_ifaces.contains(b)) {
            why.push("a Docker bridge carries the default route".to_owned());
        }
        why
    }
}

/// [`DISCOVER`]'s output, parsed.
pub fn parse_discovery(text: &str) -> Discovery {
    let mut found = Discovery::default();
    let mut section = "";
    let mut fw_lines = String::new();
    for raw in text.lines() {
        let line = raw.trim();
        // fw.sh's own status, its `== table` included, up to the end.
        if section == "fw" {
            if line == "== end" {
                section = "end";
            } else {
                fw_lines.push_str(raw);
                fw_lines.push('\n');
            }
            continue;
        }
        if let Some(name) = line.strip_prefix("== ") {
            section = name;
            continue;
        }
        if line.is_empty() {
            continue;
        }
        match section {
            "ssh" => {
                if let Ok(port) = line.parse::<u16>() {
                    found.ssh_ports.push(port);
                } else if let Some(listen) = line.strip_prefix("Listen=") {
                    // `Listen=[::]:22 (Stream)` (socket activation).
                    for part in listen.split_whitespace() {
                        if let Some(port) = part.rsplit(':').next().and_then(|p| p.parse().ok()) {
                            found.ssh_ports.push(port);
                        }
                    }
                }
            }
            "listen" => {
                if let Some(listener) = parse_listener(line) {
                    found.listeners.push(listener);
                }
            }
            "tools" => match line.split_once('=') {
                Some(("nft", v)) => found.nft = v == "yes",
                Some(("systemd", v)) => found.systemd = v == "yes",
                Some(("ufw", v)) => found.ufw_active |= v == "active",
                Some(("ufw_status", v)) => {
                    found.ufw_active |= v.ends_with("active") && !v.contains("inactive")
                }
                Some(("firewalld", v)) => found.firewalld_active = v == "active",
                Some(("nftables", v)) => found.nftables_enabled = v == "enabled",
                Some(("nodes", v)) => found.nodes = v.trim().parse().ok(),
                _ => {}
            },
            "drivers" => found.drivers.push(line.to_owned()),
            "bridges" => {
                // `<id>|<name>|<v4 mode>|<v6 mode>` (unset: empty or
                // `<no value>`; no name: `br-<id12>`).
                let set = |v: &str| {
                    let v = v.trim();
                    (!v.is_empty() && v != "<no value>").then(|| v.to_owned())
                };
                let mut parts = line.split('|');
                let id = parts.next().unwrap_or("").trim().to_owned();
                let name = match parts.next().and_then(set) {
                    Some(n) => n,
                    None if id.len() >= 12 => format!("br-{}", &id[..12]),
                    None => continue,
                };
                for mode in parts.filter_map(set) {
                    if mode != "nat" {
                        found.routed.push(format!("{name} ({mode})"));
                    }
                }
                found.bridges.push(name);
            }
            "route" if !found.public_ifaces.iter().any(|i| i == line) => {
                found.public_ifaces.push(line.to_owned());
            }
            _ => {}
        }
    }
    found.host = parse_status(&fw_lines);
    found.ssh_ports.sort_unstable();
    found.ssh_ports.dedup();
    found
        .listeners
        .sort_by(|a, b| (a.port, &a.protocol, &a.address).cmp(&(b.port, &b.protocol, &b.address)));
    found.listeners.dedup();
    found
}

/// `fw.sh status`'s output.
pub fn parse_status(text: &str) -> HostState {
    let mut state = HostState::default();
    let mut table = String::new();
    let mut in_table = false;
    for line in text.lines() {
        if in_table {
            table.push_str(line);
            continue;
        }
        match line.trim().split_once('=') {
            _ if line.trim() == "== table" => in_table = true,
            Some(("installed", v)) => state.installed = v == "yes",
            Some(("confirmed", v)) => state.confirmed = v == "yes",
            Some(("last", v)) => {
                let mut parts = v.split_whitespace();
                if let (Some(tx), Some(s)) = (parts.next(), parts.next()) {
                    state.last = Some((tx.to_owned(), s.to_owned()));
                }
            }
            Some(("boot_failed", v)) if v != "none" && !v.is_empty() => {
                state.boot_failed = Some(v.to_owned())
            }
            _ => {}
        }
    }
    if table.trim_start().starts_with('{') {
        state.table = Some(table);
    }
    state
}

/// One `ss -H -ltnup` line: `tcp LISTEN 0 4096 0.0.0.0:22 0.0.0.0:* users:(("sshd",pid=790,fd=3))`.
fn parse_listener(line: &str) -> Option<Listener> {
    let fields: Vec<&str> = line.split_whitespace().collect();
    let protocol = match *fields.first()? {
        "tcp" => "tcp",
        "udp" => "udp",
        _ => return None,
    };
    let local = fields.get(4)?;
    let (address, port) = local.rsplit_once(':')?;
    let port = port.parse::<u16>().ok()?;
    let address = address
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_owned();
    let process = fields
        .iter()
        .find_map(|f| f.strip_prefix("users:((\""))
        .and_then(|rest| rest.split('"').next())
        .map(str::to_owned);
    Some(Listener {
        protocol: protocol.to_owned(),
        address,
        port,
        process,
    })
}

/// The panel's own image, the host commands' helper (installed panels only).
async fn image(docker: &Docker, installed: bool) -> AppResult<String> {
    if !installed {
        return Err(ValidationErrors::field(
            "firewall",
            "only an installed panel manages its server's firewall",
        )
        .into());
    }
    crate::services::system_service::own_image(docker)
        .await?
        .map(|(reference, id)| id.unwrap_or(reference))
        .ok_or_else(|| AppError::internal("the panel's own image was not found"))
}

/// Reads the host: [`DISCOVER`] through [`host_run`].
pub async fn discover(docker: &Docker, image: &str) -> AppResult<Discovery> {
    let out = host_run(docker, image, DISCOVER, None, HOST_TIME)
        .await
        .map_err(AppError::internal)?;
    match out.code {
        Some(0) if out.stdout.contains("== end") => Ok(parse_discovery(&out.stdout)),
        Some(code) => Err(AppError::internal(format!(
            "the server's discovery failed ({code}): {}",
            out.stderr.trim()
        ))),
        None => Err(AppError::internal(
            "the server's discovery did not answer in time",
        )),
    }
}

/// The policy as saved (its enabled openings, its blocked addresses).
pub async fn policy<C: sea_orm::ConnectionTrait>(db: &C) -> AppResult<Policy> {
    let openings = firewall_opening::Entity::find()
        .filter(firewall_opening::Column::Enabled.eq(true))
        .order_by_asc(firewall_opening::Column::Id)
        .all(db)
        .await?
        .into_iter()
        .filter_map(|o| opening_of(&o))
        .collect();
    let blocked = firewall_blocked::Entity::find()
        .order_by_asc(firewall_blocked::Column::Id)
        .all(db)
        .await?
        .into_iter()
        .filter_map(|b| Cidr::parse(&b.cidr).ok())
        .collect();
    Ok(Policy { openings, blocked })
}

fn opening_of(o: &firewall_opening::Model) -> Option<Opening> {
    Some(Opening {
        id: o.id,
        ports: (
            u16::try_from(o.port_start).ok()?,
            u16::try_from(o.port_end).ok()?,
        ),
        protocol: if o.protocol == "udp" {
            Protocol::Udp
        } else {
            Protocol::Tcp
        },
        target: if o.target == "host" {
            Target::Host
        } else {
            Target::Published
        },
        anywhere: o.anywhere,
        sources: o
            .sources
            .split(',')
            .filter(|s| !s.trim().is_empty())
            .filter_map(|s| Cidr::parse(s).ok())
            .collect(),
    })
}

/// What a policy opens and blocks, as recorded and compared (stable).
pub fn summary(policy: &Policy) -> Value {
    let mut openings: Vec<Value> = policy
        .openings
        .iter()
        .map(|o| {
            let mut sources: Vec<String> = o.sources.iter().map(|c| c.to_string()).collect();
            sources.sort();
            json!({
                "ports": [o.ports.0, o.ports.1],
                "protocol": o.protocol.as_str(),
                "target": o.target,
                "anywhere": o.anywhere,
                "sources": sources,
            })
        })
        .collect();
    openings.sort_by_key(|v| v.to_string());
    let mut blocked: Vec<String> = policy.blocked.iter().map(|c| c.to_string()).collect();
    blocked.sort();
    json!({ "openings": openings, "blocked": blocked })
}

/// A recorded summary as the policy it was (ids are positions: only shown).
fn policy_of_summary(summary: &Value) -> Policy {
    let cidrs = |v: &Value| -> Vec<Cidr> {
        v.as_array()
            .into_iter()
            .flatten()
            .filter_map(|c| Cidr::parse(c.as_str()?).ok())
            .collect()
    };
    let openings = summary["openings"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .filter_map(|(i, o)| {
            let ports = o["ports"].as_array()?;
            Some(Opening {
                id: i64::try_from(i).ok()?,
                ports: (
                    u16::try_from(ports.first()?.as_u64()?).ok()?,
                    u16::try_from(ports.get(1)?.as_u64()?).ok()?,
                ),
                protocol: if o["protocol"] == "udp" {
                    Protocol::Udp
                } else {
                    Protocol::Tcp
                },
                target: if o["target"] == "host" {
                    Target::Host
                } else {
                    Target::Published
                },
                anywhere: o["anywhere"].as_bool().unwrap_or(false),
                sources: cidrs(&o["sources"]),
            })
        })
        .collect();
    Policy {
        openings,
        blocked: cidrs(&summary["blocked"]),
    }
}

/// The transaction not over yet (at most one).
async fn open_tx<C: sea_orm::ConnectionTrait>(
    db: &C,
) -> AppResult<Option<firewall_transaction::Model>> {
    Ok(firewall_transaction::Entity::find()
        .filter(firewall_transaction::Column::State.is_in(["intent", "pending", "unknown"]))
        .order_by_desc(firewall_transaction::Column::Id)
        .one(db)
        .await?)
}

/// The last kept change (what is on when the firewall is on).
async fn last_kept<C: sea_orm::ConnectionTrait>(
    db: &C,
) -> AppResult<Option<firewall_transaction::Model>> {
    Ok(firewall_transaction::Entity::find()
        .filter(firewall_transaction::Column::State.eq("kept"))
        .order_by_desc(firewall_transaction::Column::Id)
        .one(db)
        .await?)
}

async fn set_state(
    db: &sea_orm::DatabaseConnection,
    row: firewall_transaction::Model,
    state: &str,
    result: Option<String>,
) -> AppResult<firewall_transaction::Model> {
    let mut active: firewall_transaction::ActiveModel = row.into();
    active.state = Set(state.to_owned());
    if result.is_some() {
        active.result = Set(result);
    }
    active.updated_at = Set(Utc::now());
    Ok(active.update(db).await?)
}

/// Runs `fw.sh <args>` on the host (`input` on its standard input).
async fn run_fw(
    docker: &Docker,
    image: &str,
    args: &[&str],
    input: Option<Vec<u8>>,
) -> AppResult<HostOutput> {
    host_run(docker, image, &fw(args), input, HOST_TIME)
        .await
        .map_err(AppError::internal)
}

/// The host's journal state of `txid` (`fw.sh state`).
async fn host_tx_state(
    docker: &Docker,
    image: &str,
    txid: &str,
) -> AppResult<(String, bool, bool)> {
    let out = run_fw(docker, image, &["state", txid], None).await?;
    if out.code != Some(0) {
        return Err(AppError::internal(format!(
            "the server's firewall state could not be read: {}",
            out.stderr.trim()
        )));
    }
    let mut state = String::new();
    let (mut boot, mut now, mut timer) = (String::new(), String::new(), String::new());
    for line in out.stdout.lines() {
        match line.split_once('=') {
            Some(("state", v)) => state = v.to_owned(),
            Some(("boot", v)) => boot = v.to_owned(),
            Some(("now", v)) => now = v.to_owned(),
            Some(("timer", v)) => timer = v.to_owned(),
            _ => {}
        }
    }
    Ok((state, boot == now, timer == "active"))
}

/// Settles what an interrupted change left (the panel restarted, a request
/// went away): the host's journal decides; an expired change is reverted.
/// Under [`LOCK`] (its callers hold it).
async fn settle(app: &AppState, docker: &Docker, installed: bool) -> AppResult<()> {
    let db = app.db();
    let Some(row) = open_tx(db).await? else {
        return Ok(());
    };
    // An apply on its way leaves `intent` (no deadline yet): only one older
    // than the longest host command is settled.
    if row.state == "intent"
        && Utc::now() - row.updated_at
            < chrono::Duration::from_std(HOST_TIME * 2).unwrap_or_default()
    {
        return Ok(());
    }
    let image = image(docker, installed).await?;
    let (state, same_boot, timer) = host_tx_state(docker, &image, &row.txid).await?;
    let expired = row
        .deadline
        .is_none_or(|d| Utc::now() > d + chrono::Duration::seconds(20));
    let state = match state.as_str() {
        "committing" | "committed" => {
            finish(docker, &image, &["keep", &row.txid]).await?;
            host_tx_state(docker, &image, &row.txid).await?.0
        }
        "intended" | "armed" | "applied" if expired || !same_boot || !timer => {
            finish(docker, &image, &["revert", &row.txid]).await?;
            host_tx_state(docker, &image, &row.txid).await?.0
        }
        "intended" | "armed" | "applied" => {
            if row.state != "pending" {
                set_state(db, row, "pending", None).await?;
            }
            return Ok(());
        }
        _ => state,
    };
    adopt(app, row, &state, "settled from the server's journal").await
}

/// Runs a settling `fw.sh` step: its failure (or no answer) leaves the
/// change as it was, for the next look.
async fn finish(docker: &Docker, image: &str, args: &[&str]) -> AppResult<()> {
    let out = run_fw(docker, image, args, None).await?;
    if out.code == Some(0) {
        Ok(())
    } else {
        Err(AppError::internal(format!(
            "the server did not settle the change: {}",
            out.stderr.trim()
        )))
    }
}

/// Records the host's final state of `row` (a state not final: left as is).
async fn adopt(
    app: &AppState,
    row: firewall_transaction::Model,
    state: &str,
    why: &str,
) -> AppResult<()> {
    let db = app.db();
    match state {
        "kept" => {
            let hash = row.hash.clone();
            set_state(db, row, "kept", None).await?;
            if let Some(hash) = hash {
                panel_setting::set(db, CONFIRMED_HASH, &hash).await?;
            }
        }
        "reverted" | "aborted" => {
            set_state(db, row, "reverted", Some(why.to_owned())).await?;
        }
        "failed" => {
            set_state(db, row, "failed", None).await?;
        }
        "" => {
            set_state(
                db,
                row,
                "failed",
                Some("the server never received it".to_owned()),
            )
            .await?;
        }
        _ => {}
    }
    Ok(())
}

/// The Firewall page: the host, its listeners and what the firewall does
/// with each, the policy, the change waiting to be kept.
pub async fn report(app: &AppState, docker: &Docker, installed: bool) -> AppResult<Value> {
    if !installed {
        return Ok(json!({
            "available": false,
            "reason": "Only an installed panel manages its server's firewall (this one runs from source).",
        }));
    }
    if let Ok(_guard) = LOCK.try_lock() {
        let _ = settle(app, docker, installed).await;
    }
    let db = app.db();
    let image = image(docker, installed).await?;
    let found = discover(docker, &image).await?;
    let saved = policy(db).await?;
    let enabled = found.host.confirmed;
    let kept = last_kept(db).await?;
    let confirmed = kept
        .as_ref()
        .and_then(|k| serde_json::from_str::<Value>(&k.summary).ok());
    let changed = !enabled || confirmed.as_ref() != Some(&summary(&saved));
    // What the server runs now: the change being tried, else the fallback
    // (when the confirmed table did not load), else the confirmed policy;
    // the saved one is a draft until applied.
    let trying = open_tx(db).await?.filter(|t| t.state == "pending");
    let active = if let Some(t) = &trying {
        serde_json::from_str::<Value>(&t.summary)
            .map(|v| policy_of_summary(&v))
            .unwrap_or_default()
    } else if found.host.boot_failed.is_some() {
        Policy::default()
    } else {
        confirmed
            .as_ref()
            .map(policy_of_summary)
            .unwrap_or_default()
    };
    let drift = match (
        &found.host.table,
        panel_setting::get(db, CONFIRMED_HASH).await?,
    ) {
        (Some(table), Some(hash)) if enabled => rules::canonical_hash(table)
            .map(|h| h != hash)
            .unwrap_or(true),
        (None, _) if enabled => true,
        _ => false,
    };
    let pending = open_tx(db).await?.map(|t| {
        json!({
            "txid": t.txid,
            "state": t.state,
            "deadline": t.deadline,
            "summary": serde_json::from_str::<Value>(&t.summary).unwrap_or(Value::Null),
        })
    });
    let mut unsupported = found.unsupported();
    if !reached_by_name(app) {
        unsupported.push(
            "the panel has no domain (it answers on port 3000): give it a domain or the Cloudflare tunnel first"
                .to_owned(),
        );
    }
    let openings: Vec<Value> = firewall_opening::Entity::find()
        .order_by_asc(firewall_opening::Column::Id)
        .all(db)
        .await?
        .into_iter()
        .map(|o| serde_json::to_value(o).unwrap_or(Value::Null))
        .collect();
    let blocked: Vec<Value> = firewall_blocked::Entity::find()
        .order_by_asc(firewall_blocked::Column::Id)
        .all(db)
        .await?
        .into_iter()
        .map(|b| serde_json::to_value(b).unwrap_or(Value::Null))
        .collect();
    Ok(json!({
        "available": true,
        "enabled": enabled,
        "changed": changed,
        "drift": drift,
        "pending": pending,
        "unsupported": unsupported,
        "ssh_ports": found.ssh_ports,
        "host": {
            "nft": found.nft,
            "systemd": found.systemd,
            "ufw": found.ufw_active,
            "firewalld": found.firewalld_active,
            "nodes": found.nodes,
            "boot_failed": found.host.boot_failed,
        },
        "ports": ports(db, &found, &active, enabled).await?,
        "openings": openings,
        "blocked": blocked,
    }))
}

/// Every port: the host's listeners and the published ones (the routing
/// mesh listens nowhere `ss` sees), with their owner and what the firewall
/// does with them.
async fn ports(
    db: &sea_orm::DatabaseConnection,
    found: &Discovery,
    policy: &Policy,
    enabled: bool,
) -> AppResult<Vec<Value>> {
    use std::collections::BTreeMap;
    let published = published_ports(db).await?;
    // By port and protocol: its addresses, its process, whether all are local.
    type Row = (Vec<String>, Option<String>, bool);
    let mut rows: BTreeMap<(u16, String), Row> = BTreeMap::new();
    for l in &found.listeners {
        let entry = rows
            .entry((l.port, l.protocol.clone()))
            .or_insert_with(|| (vec![], l.process.clone(), true));
        entry.0.push(l.address.clone());
        entry.2 &= l.local();
        if entry.1.is_none() {
            entry.1 = l.process.clone();
        }
    }
    for (port, protocol, _) in &published {
        rows.entry((*port, protocol.clone()))
            .or_insert_with(|| (vec!["*".to_owned()], None, false));
    }
    let ssh = &found.ssh_ports;
    Ok(rows
        .into_iter()
        .map(|((port, protocol), (addresses, process, local))| {
            let service = published
                .iter()
                .find(|(p, proto, _)| *p == port && *proto == protocol)
                .map(|(_, _, owner)| owner.clone());
            let swarm = SWARM_PORTS
                .iter()
                .any(|(p, proto)| *p == port && *proto == protocol);
            let owner = if let Some(s) = &service {
                s.clone()
            } else if protocol == "tcp" && ssh.contains(&port) {
                "SSH".to_owned()
            } else if protocol == "tcp" && rules::WEB_PORTS.contains(&port) {
                "Traefik (the panel and the domains)".to_owned()
            } else if swarm {
                if port == 4789 {
                    "Docker Swarm (VXLAN)".to_owned()
                } else {
                    "Docker Swarm".to_owned()
                }
            } else {
                process.clone().unwrap_or_else(|| "unknown".to_owned())
            };
            let always =
                protocol == "tcp" && (ssh.contains(&port) || rules::WEB_PORTS.contains(&port));
            let openings: Vec<Value> = policy
                .openings
                .iter()
                .filter(|o| {
                    o.protocol.as_str() == protocol && (o.ports.0..=o.ports.1).contains(&port)
                })
                .map(|o| {
                    json!({
                        "id": o.id,
                        "anywhere": o.anywhere,
                        "sources": o.sources.iter().map(|c| c.to_string()).collect::<Vec<_>>(),
                    })
                })
                .collect();
            let state = if local {
                "local"
            } else if always {
                "always_open"
            } else if !openings.is_empty() && enabled {
                "opened"
            } else if enabled {
                "closed"
            } else {
                "open"
            };
            json!({
                "port": port,
                "protocol": protocol,
                "addresses": addresses,
                "owner": owner,
                "published": service.is_some(),
                "swarm": swarm,
                "state": state,
                "openings": openings,
            })
        })
        .collect())
}

/// The ports services publish: `(port, protocol, "project / service")`.
async fn published_ports(
    db: &sea_orm::DatabaseConnection,
) -> AppResult<Vec<(u16, String, String)>> {
    use crate::models::{project, service, service_port};
    let ports = service_port::Entity::find().all(db).await?;
    let services = service::Entity::find().all(db).await?;
    let projects = project::Entity::find().all(db).await?;
    Ok(ports
        .into_iter()
        .filter_map(|p| {
            let s = services.iter().find(|s| s.id == p.service_id)?;
            let project = projects.iter().find(|x| x.id == s.project_id)?;
            Some((
                u16::try_from(p.published).ok()?,
                p.protocol.clone(),
                format!("{} / {}", project.name, s.name),
            ))
        })
        .collect())
}

/// What turning the firewall on (or applying the policy) closes and opens:
/// the table it loads, and the listeners left closed.
pub async fn preview(app: &AppState, docker: &Docker, installed: bool) -> AppResult<Value> {
    let db = app.db();
    let image = image(docker, installed).await?;
    let found = discover(docker, &image).await?;
    let saved = policy(db).await?;
    let ruleset = rules::render(&saved, &facts(&found));
    let closes: Vec<Value> = ports(db, &found, &saved, true)
        .await?
        .into_iter()
        .filter(|p| p["state"] == "closed")
        .collect();
    Ok(json!({ "ruleset": ruleset, "closes": closes }))
}

/// The panel is reached by its domain (on 80/443) or through the tunnel,
/// never on port 3000 (which the firewall closes).
fn reached_by_name(app: &AppState) -> bool {
    let host = &crate::services::deploy_rules_service::config().panel_host;
    let domain =
        !host.is_empty() && host != "localhost" && host.parse::<std::net::IpAddr>().is_err();
    domain || crate::services::tunnel_service::enabled(app)
}

fn facts(found: &Discovery) -> HostFacts {
    HostFacts {
        ssh_ports: found.ssh_ports.clone(),
        docker_ifaces: found.docker_ifaces(),
    }
}

/// Applies the saved policy: the host journals it, arms its revert, loads
/// it; the user keeps it within [`KEEP_WINDOW`] or the host reverts it.
pub async fn apply(
    app: &AppState,
    docker: &Docker,
    by: &Initiator,
    installed: bool,
) -> AppResult<Value> {
    let _guard = LOCK.lock().await;
    let db = app.db();
    settle(app, docker, installed).await?;
    if open_tx(db).await?.is_some() {
        return Err(ValidationErrors::field(
            "firewall",
            "a change is waiting to be kept or reverted",
        )
        .into());
    }
    let image = image(docker, installed).await?;
    let found = discover(docker, &image).await?;
    let mut why = found.unsupported();
    if !reached_by_name(app) {
        why.push("the panel has no domain: give it one (or the tunnel) first".to_owned());
    }
    if found.ssh_ports.is_empty() {
        why.push("no SSH port was found on the server".to_owned());
    }
    if !why.is_empty() {
        return Err(ValidationErrors::field("firewall", why.join("; ")).into());
    }
    let saved = policy(db).await?;
    let host_facts = facts(&found);
    let ruleset = rules::render(&saved, &host_facts);
    // The files and units first (again: they follow the panel's version).
    let install = host_run(
        docker,
        &image,
        &host::install_script(),
        Some(rules::fallback(&host_facts).into_bytes()),
        HOST_TIME,
    )
    .await
    .map_err(AppError::internal)?;
    if install.code != Some(0) {
        return Err(AppError::internal(format!(
            "the firewall could not be installed on the server: {}",
            install.stderr.trim()
        )));
    }
    let txid = crate::services::database_service::random_string(20).to_ascii_lowercase();
    let now = Utc::now();
    let row = firewall_transaction::ActiveModel {
        txid: Set(txid.clone()),
        kind: Set("apply".to_owned()),
        state: Set("intent".to_owned()),
        ruleset: Set(ruleset.clone()),
        summary: Set(summary(&saved).to_string()),
        created_by: Set(by.user_id),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(db)
    .await?;
    let rev = row.id.to_string();
    let out = action_service::run(db, by, "firewall.apply", "firewall", async {
        run_fw(
            docker,
            &image,
            &["apply", &txid, &rev],
            Some(ruleset.into_bytes()),
        )
        .await
    })
    .await;
    let out = match out {
        Ok(out) => out,
        Err(err) => {
            set_state(db, row, "unknown", Some(err.to_string())).await?;
            return Err(err);
        }
    };
    match out.code {
        Some(0) => {
            // `armed=<epoch>`, then the table as the kernel lists it.
            let armed = out
                .stdout
                .lines()
                .find_map(|l| l.strip_prefix("armed="))
                .and_then(|v| v.trim().parse::<i64>().ok())
                .and_then(|t| chrono::DateTime::from_timestamp(t, 0));
            let table = out
                .stdout
                .split_once("== table")
                .map(|(_, t)| t)
                .unwrap_or("");
            let hash = rules::canonical_hash(table.trim()).map_err(AppError::internal)?;
            let mut active: firewall_transaction::ActiveModel = row.into();
            active.state = Set("pending".to_owned());
            active.hash = Set(Some(hash));
            // The host reverts at its arm time + 120 s: Keep closes before.
            active.deadline = Set(Some(
                armed.unwrap_or_else(Utc::now) + chrono::Duration::seconds(KEEP_WINDOW),
            ));
            active.updated_at = Set(Utc::now());
            let row = active.update(db).await?;
            // The bridges as found now (the table has them; the host's list
            // follows, for its reloads).
            let list = host_facts.docker_ifaces.join("\n") + "\n";
            let sent = run_fw(docker, &image, &["ifaces"], Some(list.into_bytes()))
                .await
                .is_ok_and(|o| o.code == Some(0));
            *SENT_IFACES.lock().expect("not poisoned") = sent.then_some(host_facts.docker_ifaces);
            Ok(json!({ "txid": row.txid, "deadline": row.deadline }))
        }
        Some(code) => {
            let why = out.stderr.trim().to_owned();
            set_state(db, row, "failed", Some(why.clone())).await?;
            Err(ValidationErrors::field(
                "firewall",
                format!("the server refused the change ({code}): {why}"),
            )
            .into())
        }
        None => {
            set_state(
                db,
                row,
                "unknown",
                Some("the server did not answer in time".to_owned()),
            )
            .await?;
            Err(AppError::internal(
                "the server did not answer in time: the change reverts by itself unless it is kept",
            ))
        }
    }
}

/// Keeps the change `txid`: still the table it loaded, within its time.
pub async fn keep(
    app: &AppState,
    docker: &Docker,
    by: &Initiator,
    txid: &str,
    installed: bool,
) -> AppResult<Value> {
    let _guard = LOCK.lock().await;
    let db = app.db();
    let row = firewall_transaction::Entity::find()
        .filter(firewall_transaction::Column::Txid.eq(txid))
        .one(db)
        .await?
        .ok_or(AppError::NotFound)?;
    if row.state != "pending" || row.deadline.is_none_or(|d| Utc::now() > d) {
        return Err(ValidationErrors::field("txid", "this change is no longer waiting").into());
    }
    let image = image(docker, installed).await?;
    let status = run_fw(docker, &image, &["status"], None).await?;
    let state = parse_status(&status.stdout);
    let now = state
        .table
        .as_deref()
        .and_then(|t| rules::canonical_hash(t).ok());
    if now.is_none() || now != row.hash {
        return Err(ValidationErrors::field(
            "txid",
            "the server's table is not the one this change loaded (it was reverted or changed)",
        )
        .into());
    }
    let out = action_service::run(db, by, "firewall.keep", "firewall", async {
        run_fw(docker, &image, &["keep", txid], None).await
    })
    .await?;
    if out.code != Some(0) {
        return Err(ValidationErrors::field(
            "txid",
            format!("the server did not keep it: {}", out.stderr.trim()),
        )
        .into());
    }
    let hash = row.hash.clone();
    set_state(db, row, "kept", None).await?;
    if let Some(hash) = hash {
        panel_setting::set(db, CONFIRMED_HASH, &hash).await?;
    }
    Ok(json!({ "kept": txid }))
}

/// Reverts the change `txid` now (the host would at its time).
pub async fn revert(
    app: &AppState,
    docker: &Docker,
    by: &Initiator,
    txid: &str,
    installed: bool,
) -> AppResult<Value> {
    let _guard = LOCK.lock().await;
    let db = app.db();
    let row = firewall_transaction::Entity::find()
        .filter(firewall_transaction::Column::Txid.eq(txid))
        .one(db)
        .await?
        .ok_or(AppError::NotFound)?;
    if !firewall_transaction::open(&row.state) {
        return Err(ValidationErrors::field("txid", "this change is no longer waiting").into());
    }
    let image = image(docker, installed).await?;
    let out = action_service::run(db, by, "firewall.revert", "firewall", async {
        run_fw(docker, &image, &["revert", txid], None).await
    })
    .await?;
    if out.code != Some(0) {
        return Err(AppError::internal(format!(
            "the server did not revert it: {}",
            out.stderr.trim()
        )));
    }
    let (state, _, _) = host_tx_state(docker, &image, txid).await?;
    adopt(app, row, &state, "reverted by the user").await?;
    Ok(json!({ "state": state }))
}

/// Turns the firewall off: the table, its files and units removed (Docker
/// no longer waits for it).
pub async fn disable(
    app: &AppState,
    docker: &Docker,
    by: &Initiator,
    installed: bool,
) -> AppResult<Value> {
    let _guard = LOCK.lock().await;
    let db = app.db();
    settle(app, docker, installed).await?;
    if open_tx(db).await?.is_some() {
        return Err(ValidationErrors::field(
            "firewall",
            "a change is waiting to be kept or reverted",
        )
        .into());
    }
    let image = image(docker, installed).await?;
    let out = action_service::run(db, by, "firewall.disable", "firewall", async {
        run_fw(docker, &image, &["disable"], None).await
    })
    .await?;
    if out.code != Some(0) {
        return Err(AppError::internal(format!(
            "the server did not turn the firewall off: {}",
            out.stderr.trim()
        )));
    }
    panel_setting::set(db, CONFIRMED_HASH, "").await?;
    Ok(json!({ "enabled": false }))
}

/// The scheduled check: an interrupted change settled; the trusted Docker
/// bridges given to the host when they changed; the table compared with
/// the confirmed one, reloaded when it differs (an alert). A line for the
/// schedules' history, or None when the firewall is off.
pub async fn check(app: &AppState, docker: &Docker, installed: bool) -> AppResult<Option<String>> {
    if !installed {
        return Ok(None);
    }
    let Ok(_guard) = LOCK.try_lock() else {
        return Ok(Some("a change is running".to_owned()));
    };
    let db = app.db();
    settle(app, docker, installed).await?;
    let image = image(docker, installed).await?;
    let found = discover(docker, &image).await?;
    if !found.host.confirmed {
        return Ok(None);
    }
    let mut notes = vec![];
    let why = found.unsupported();
    if !why.is_empty() {
        tracing::warn!(reasons = %why.join("; "), "the firewall is on where it is no longer supported");
        notes.push(format!("no longer supported here: {}", why.join("; ")));
    }
    let ifaces = found.docker_ifaces();
    let sent = SENT_IFACES.lock().expect("not poisoned").clone();
    if sent.as_ref() != Some(&ifaces) {
        let list = ifaces.join("\n") + "\n";
        let out = run_fw(docker, &image, &["ifaces"], Some(list.into_bytes())).await?;
        if out.code == Some(0) {
            *SENT_IFACES.lock().expect("not poisoned") = Some(ifaces);
            notes.push("Docker bridges updated".to_owned());
        }
    }
    if open_tx(db).await?.is_some() {
        return Ok(Some(notes.join("; ")));
    }
    let confirmed = panel_setting::get(db, CONFIRMED_HASH)
        .await?
        .unwrap_or_default();
    let now = found
        .host
        .table
        .as_deref()
        .and_then(|t| rules::canonical_hash(t).ok());
    if now.as_deref() != Some(confirmed.as_str()) {
        let out = run_fw(docker, &image, &["load"], None).await?;
        action_service::record(db, "firewall.reconcile", "firewall").await?;
        tracing::warn!(code = ?out.code, "the server's firewall table differed from the confirmed one: reloaded");
        notes.push(if out.code == Some(0) {
            "the table differed: reloaded".to_owned()
        } else {
            format!(
                "the table differed and could not be reloaded: {}",
                out.stderr.trim()
            )
        });
    }
    Ok(Some(if notes.is_empty() {
        "as confirmed".to_owned()
    } else {
        notes.join("; ")
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUTPUT: &str = "
== ssh
22
Listen=
== listen
tcp   LISTEN 0      4096         0.0.0.0:22        0.0.0.0:*    users:((\"sshd\",pid=790,fd=3))
tcp   LISTEN 0      4096         0.0.0.0:443       0.0.0.0:*    users:((\"docker-proxy\",pid=2009,fd=8))
tcp   LISTEN 0      4096            [::]:22           [::]:*    users:((\"sshd\",pid=790,fd=4))
tcp   LISTEN 0      4096               *:2377            *:*    users:((\"dockerd\",pid=823,fd=19))
udp   UNCONN 0      0            0.0.0.0:4789      0.0.0.0:*
tcp   LISTEN 0      4096   127.0.0.53%lo:53        0.0.0.0:*    users:((\"systemd-resolve\",pid=707,fd=14))
== tools
nft=yes
systemd=yes
ufw=inactive
ufw_status=Status: inactive
firewalld=inactive
nftables=disabled
nodes=1
== drivers
bridge
overlay
== bridges
3f2a1b4c5d6e7f8091a2b3c4d5e6f708192a3b4c|docker0||
9e8d7c6b5a4f3e2d1c0b9a8f7e6d5c4b3a291807|<no value>|<no value>|<no value>
aa11bb22cc33dd44ee55ff6677889900aabbccdd|docker_gwbridge|nat|routed
== route
eth0
== fw
installed=yes
confirmed=yes
last=abc123 kept
boot_failed=none
== table
{\"nftables\":[{\"metainfo\":{}},{\"table\":{\"family\":\"inet\",\"name\":\"rdockpanel\",\"handle\":1}}]}
== end
";

    #[test]
    fn the_hosts_answer_is_read() {
        let found = parse_discovery(OUTPUT);
        assert_eq!(found.ssh_ports, [22]);
        assert!(found.nft && found.systemd);
        assert!(!found.ufw_active && !found.firewalld_active && !found.nftables_enabled);
        assert_eq!(found.nodes, Some(1));
        assert_eq!(found.drivers, ["bridge", "overlay"]);
        assert_eq!(
            found.docker_ifaces(),
            ["br-9e8d7c6b5a4f", "docker0", "docker_gwbridge"]
        );
        assert_eq!(found.public_ifaces, ["eth0"]);
        assert_eq!(found.routed, ["docker_gwbridge (routed)"]);
        assert!(found.unsupported().join(";").contains("without NAT"));
        assert!(found.host.installed && found.host.confirmed);
        assert_eq!(
            found.host.last,
            Some(("abc123".to_owned(), "kept".to_owned()))
        );
        assert!(
            found
                .host
                .table
                .as_deref()
                .unwrap()
                .starts_with("{\"nftables\"")
        );
        let ports: Vec<_> = found
            .listeners
            .iter()
            .map(|l| {
                (
                    l.port,
                    l.protocol.as_str(),
                    l.address.as_str(),
                    l.process.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            ports,
            [
                (22, "tcp", "0.0.0.0", Some("sshd")),
                (22, "tcp", "::", Some("sshd")),
                (53, "tcp", "127.0.0.53%lo", Some("systemd-resolve")),
                (443, "tcp", "0.0.0.0", Some("docker-proxy")),
                (2377, "tcp", "*", Some("dockerd")),
                (4789, "udp", "0.0.0.0", None),
            ]
        );
        assert!(found.listeners[2].local());
        assert!(!found.listeners[0].local());
    }

    #[test]
    fn what_stops_it() {
        let found = parse_discovery(
            "== ssh\nListen=[::]:2222 (Stream)\n== tools\nnft=no\nsystemd=yes\nufw_status=Status: active\nnodes=3\n== drivers\nmacvlan\n== bridges\n== route\n== fw\ninstalled=no\n== end\n",
        );
        assert_eq!(found.ssh_ports, [2222]);
        assert!(found.ufw_active);
        let why = found.unsupported().join("; ");
        assert!(why.contains("nft is not installed"), "{why}");
        assert!(why.contains("more than one node"), "{why}");
        assert!(why.contains("macvlan"), "{why}");
        assert!(!found.host.installed);
    }
}
