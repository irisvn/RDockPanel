//! Whether the server has room for what is about to run: a service created,
//! a template's services, a Deploy, Start, Restart or rebuild. Advisory: it
//! never stops a deployment; the panel asks before, and logs it after.
//!
//! What a service takes is estimated (in this order): what its own tasks,
//! or the same image in its project, use now (`observed`); its memory limit
//! (`limit`); else a typical figure for its kind (`default`). A Compose file
//! from a repository is not known before it runs (`unknown`).
//!
//! Then the host: its available memory (`MemAvailable`) after that is
//! compared with what should stay free. Memory reservations are Swarm's:
//! a task whose reservation the node no longer has stays pending for ever.

use std::collections::BTreeSet;
use std::sync::Mutex;
use std::time::Duration;

use futures_util::StreamExt;
use rustcms::{AppResult, AppState};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::Serialize;
use serde_json::{Value, json};

use crate::models::service;
use crate::services::database_service::Engine;
use crate::services::deploy_service::{self, Snapshot};
use crate::services::docker_service::{Docker, TaskState};
use crate::services::system_stats_service::SystemStats;

pub const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;
/// A build's own memory and disk (its layers, its cache), while it runs.
pub const BUILD_MEMORY: u64 = GIB;
pub const BUILD_DISK: u64 = 3 * GIB;
/// An image pulled (not here yet, or a newer one under its tag).
pub const PULL_DISK: u64 = GIB;
/// What should stay free: of the memory (the larger), of the disk.
const FREE_PERCENT: u64 = 15;
const FREE_MEMORY: u64 = 256 * MIB;
const DISK_SHORT: u64 = GIB;
const DISK_LOW: u64 = 2 * GIB;
/// Busy: the CPU over the last minutes.
const CPU_BUSY: f64 = 90.0;

/// A service of `kind` takes about this much, by task.
pub fn typical(kind: &str) -> u64 {
    MIB * match kind {
        "mysql" => 450,
        "mongo" => 300,
        "mariadb" => 200,
        "postgres" => 128,
        "redis" => 32,
        _ => 192,
    }
}

/// Where a figure comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Observed,
    Limit,
    Default,
    Unknown,
}

/// What one service takes more, in bytes: `memory` for good, `temporary`
/// while it rolls out or builds, `reservation` more of Swarm's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Need {
    pub name: String,
    pub kind: String,
    pub memory: u64,
    pub temporary: u64,
    pub disk: u64,
    #[serde(skip)]
    pub reservation: u64,
    pub source: Source,
}

/// The host now, in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Host {
    pub mem_total: u64,
    pub mem_available: u64,
    pub swap_used: u64,
    pub swap_total: u64,
    pub disk_free: u64,
    /// The CPU's average over the samples kept (none yet: `None`).
    pub cpu_percent: Option<f64>,
}

/// Swarm's memory reservations: the node's memory, and what its running
/// tasks reserve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reserved {
    pub total: u64,
    pub used: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    /// Room enough (or nothing more to run).
    Ok,
    /// It fits, with little left.
    Tight,
    /// It does not fit: asked before it is deployed.
    Short,
    /// Swarm cannot place it (its reservation).
    Impossible,
    /// The host's figures are not known: nothing said.
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Report {
    pub level: Level,
    pub needs: Vec<Need>,
    /// Memory more at its peak (for good and while rolling out), and disk.
    pub memory: u64,
    pub disk: u64,
    /// Why (`memory_short`, `memory_low`, `swap_high`, `disk_short`,
    /// `disk_low`, `cpu_busy`, `reservation`), worst first.
    pub reasons: Vec<&'static str>,
    pub host: Option<Host>,
    /// Swarm's reservations when one is why.
    #[serde(skip)]
    pub reserved: Option<(Reserved, u64)>,
}

/// What `needs` leave of `host` (and of Swarm's `reserved`).
pub fn assess(host: Option<Host>, reserved: Option<Reserved>, needs: Vec<Need>) -> Report {
    let memory: u64 = needs.iter().map(|n| n.memory + n.temporary).sum();
    let disk: u64 = needs.iter().map(|n| n.disk).sum();
    let reservation: u64 = needs.iter().map(|n| n.reservation).sum();
    let mut reasons = vec![];
    let mut level = Level::Ok;
    let over =
        reserved.filter(|r| reservation > 0 && r.total > 0 && r.used + reservation > r.total);
    if over.is_some() {
        reasons.push("reservation");
        level = Level::Impossible;
    }
    let report = |level, reasons| Report {
        level,
        needs: needs.clone(),
        memory,
        disk,
        reasons,
        host,
        reserved: over.map(|r| (r, reservation)),
    };
    let Some(h) = host.filter(|h| h.mem_total > 0) else {
        let level = if level == Level::Impossible {
            level
        } else {
            Level::Unknown
        };
        return report(level, reasons);
    };
    // Nothing more runs: nothing to say.
    if memory == 0 && disk == 0 {
        return report(level, reasons);
    }
    let left = i128::from(h.mem_available) - i128::from(memory);
    let disk_left = h.disk_free.checked_sub(disk);
    let mut short = vec![];
    if left < 0 {
        short.push("memory_short");
    }
    if h.disk_free > 0 && disk_left.is_none_or(|d| d < DISK_SHORT) {
        short.push("disk_short");
    }
    let mut low = vec![];
    if left >= 0 && left < i128::from((h.mem_total * FREE_PERCENT / 100).max(FREE_MEMORY)) {
        low.push("memory_low");
    }
    if h.swap_total > 0 && h.swap_used * 2 > h.swap_total {
        low.push("swap_high");
    }
    if h.disk_free > 0 && disk_left.is_some_and(|d| (DISK_SHORT..DISK_LOW).contains(&d)) {
        low.push("disk_low");
    }
    if h.cpu_percent.is_some_and(|c| c > CPU_BUSY) {
        low.push("cpu_busy");
    }
    if level == Level::Ok {
        level = if !short.is_empty() {
            Level::Short
        } else if !low.is_empty() {
            Level::Tight
        } else {
            Level::Ok
        };
    }
    reasons.extend(short);
    reasons.extend(low);
    report(level, reasons)
}

/// What a task takes: what it uses (`observed`), else its limit, else its
/// kind's.
pub fn per_task(observed: Option<u64>, limit_mb: Option<u32>, kind: &str) -> (u64, Source) {
    match (observed.filter(|o| *o > 0), limit_mb.filter(|l| *l > 0)) {
        (Some(used), _) => (used, Source::Observed),
        (None, Some(limit)) => (u64::from(limit) * MIB, Source::Limit),
        (None, None) => (typical(kind), Source::Default),
    }
}

/// A service as it runs and as it will: what changes takes more.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Change {
    /// Tasks running now, the memory each uses and reserves.
    pub running: u64,
    pub used_each: u64,
    pub reserved_each: u64,
    /// Tasks it will run, the memory each takes and reserves.
    pub wanted: u64,
    pub per_task: u64,
    pub reservation_each: u64,
    /// Rolled one task at a time, the new one started before the old stops.
    pub start_first: bool,
    pub build: bool,
    /// Its image pulled (not here yet, or maybe newer).
    pub pull: bool,
}

/// The most `running` tasks of `old` each, replaced by `wanted` of `new`,
/// hold at once: start-first holds a new task and the old one it replaces
/// (one at a time, the larger ones first or last, whichever is worse).
pub fn peak(running: u64, old: u64, wanted: u64, new: u64, start_first: bool) -> u64 {
    let (now, after) = (running * old, wanted * new);
    if !start_first || running == 0 || wanted == 0 {
        return now.max(after);
    }
    let replaced = running.min(wanted);
    let rolling = replaced * old.max(new) + old.min(new) + (wanted - replaced) * new;
    now.max(after).max(rolling)
}

pub fn need_of(name: &str, kind: &str, change: Change, source: Source) -> Need {
    let c = change;
    let (now, after) = (c.running * c.used_each, c.wanted * c.per_task);
    let most = peak(c.running, c.used_each, c.wanted, c.per_task, c.start_first);
    let reserved = peak(
        c.running,
        c.reserved_each,
        c.wanted,
        c.reservation_each,
        c.start_first,
    );
    Need {
        name: name.to_owned(),
        kind: kind.to_owned(),
        memory: after.saturating_sub(now),
        temporary: most.saturating_sub(now.max(after)) + if c.build { BUILD_MEMORY } else { 0 },
        disk: if c.build { BUILD_DISK } else { 0 }
            + if c.pull && c.wanted > 0 { PULL_DISK } else { 0 },
        reservation: reserved.saturating_sub(c.running * c.reserved_each),
        source,
    }
}

/// The services of a Compose file (its `services`), `None` when unreadable.
pub fn compose_services(file: &str) -> Option<u64> {
    let root: serde_yaml_ng::Value = serde_yaml_ng::from_str(file).ok()?;
    let services = root.get("services")?.as_mapping()?;
    Some(services.len() as u64)
}

/// A Compose service: by its file, run by `docker compose` (no Swarm
/// reservation, not rolled: what it runs again is replaced).
fn compose_need(name: &str, file: Option<&str>, running: bool) -> Need {
    let count = file.and_then(compose_services);
    let memory = if running {
        0
    } else {
        count.map_or(0, |n| n * typical("app"))
    };
    Need {
        name: name.to_owned(),
        kind: "compose".to_owned(),
        memory,
        temporary: 0,
        disk: 0,
        reservation: 0,
        source: if count.is_some() {
            Source::Default
        } else {
            Source::Unknown
        },
    }
}

/// The host from the panel's samples (one read now if none yet).
pub async fn host(app: &AppState) -> Option<Host> {
    let stats = app.service::<SystemStats>()?;
    let history = stats.history();
    let last = match history.last() {
        Some(last) => last.clone(),
        None => {
            let stats = stats.clone();
            tokio::task::spawn_blocking(move || stats.sample_now())
                .await
                .ok()?
        }
    };
    if last.mem_total == 0 {
        return None;
    }
    // The first sample reads no CPU (it needs two).
    let cpu: Vec<f64> = history.iter().skip(1).map(|s| s.cpu_percent).collect();
    Some(Host {
        mem_total: last.mem_total,
        mem_available: last.mem_total.saturating_sub(last.mem_used),
        swap_used: last.swap_used,
        swap_total: last.swap_total,
        disk_free: last.disk_total.saturating_sub(last.disk_used),
        cpu_percent: (!cpu.is_empty()).then(|| cpu.iter().sum::<f64>() / cpu.len() as f64),
    })
}

async fn reserved(docker: &Docker) -> Option<Reserved> {
    let total = u64::try_from(docker.info().await.ok()?.mem_total).ok()?;
    let used = docker.engine().reserved_memory().await.ok()?;
    Some(Reserved { total, used })
}

/// The running tasks of Swarm service `name`: how many, and the memory
/// one uses (the largest of a few).
async fn observed(docker: &Docker, name: &str) -> (u64, Option<u64>) {
    let Ok(tasks) = docker.engine().list_tasks(name).await else {
        return (0, None);
    };
    let running: Vec<_> = tasks
        .iter()
        .filter(|t| t.status.as_ref().and_then(|s| s.state) == Some(TaskState::RUNNING))
        .collect();
    let containers: Vec<String> = running
        .iter()
        .filter_map(|t| {
            t.status
                .as_ref()?
                .container_status
                .as_ref()?
                .container_id
                .clone()
        })
        .take(3)
        .collect();
    let samples = futures_util::future::join_all(containers.iter().map(|id| {
        let stream = docker.engine().stats(id);
        async move {
            tokio::time::timeout(
                Duration::from_secs(3),
                stream.take(2).fold(None, |_, s| async move { s.ok() }),
            )
            .await
            .ok()
            .flatten()
            .map(|s| s.mem_used)
        }
    }))
    .await;
    (running.len() as u64, samples.into_iter().flatten().max())
}

/// What `image` uses in project `project_id` (another of its services).
async fn observed_image(
    app: &AppState,
    docker: &Docker,
    project_id: i64,
    project: &str,
    image: &str,
) -> Option<u64> {
    let image = image.trim();
    if image.is_empty() {
        return None;
    }
    let wanted = same_image(image);
    let same = service::Entity::find()
        .filter(service::Column::ProjectId.eq(project_id))
        .filter(service::Column::DesiredState.ne("stopped"))
        .all(app.db())
        .await
        .ok()?;
    for row in same.iter().filter(|r| same_image(&r.image) == wanted) {
        let name = deploy_service::service_name(project, &row.name);
        if let (_, Some(used)) = observed(docker, &name).await {
            return Some(used);
        }
    }
    None
}

/// An image as its tasks name it: without its digest, `:latest` when it
/// has no tag (`whoami` and `whoami:latest` are one).
pub fn same_image(image: &str) -> String {
    let image = image.trim();
    let image = image.split('@').next().unwrap_or(image);
    let name = image.rsplit('/').next().unwrap_or(image);
    if name.contains(':') || image.is_empty() {
        image.to_owned()
    } else {
        format!("{image}:latest")
    }
}

async fn present(docker: &Docker, image: &str) -> bool {
    !image.trim().is_empty() && docker.engine().inspect_image(image.trim()).await.is_ok()
}

/// A service to be created (a form's, a template's).
#[derive(Debug, Clone, Default)]
pub struct Planned {
    pub name: String,
    pub kind: String,
    pub image: Option<String>,
    pub replicas: u32,
    pub mem_limit_mb: Option<u32>,
    pub mem_reservation_mb: Option<u32>,
    pub build: bool,
    /// A Compose service's file (`None` with `kind` compose: a repository's).
    pub compose: Option<String>,
}

impl Planned {
    pub fn of(new: &crate::services::creation_service::NewService) -> Self {
        Self {
            name: new.name.clone(),
            kind: new.kind.clone(),
            image: new.image.clone(),
            replicas: u32::try_from(new.replicas.max(1)).unwrap_or(1),
            mem_limit_mb: new.mem_limit_mb,
            mem_reservation_mb: new.mem_reservation_mb,
            build: new.build.is_some(),
            compose: new.compose.clone(),
        }
    }
}

/// Room for `planned`, new in project `project_id` (deployed); `observe`:
/// the caller sees its services (what the same image uses there).
pub async fn for_new(
    app: &AppState,
    project_id: i64,
    project: &str,
    planned: &[Planned],
    observe: bool,
) -> Report {
    let docker = app.service::<Docker>();
    let mut needs = vec![];
    for p in planned {
        if p.kind == "compose" {
            needs.push(compose_need(&p.name, p.compose.as_deref(), false));
            continue;
        }
        let image = p
            .image
            .clone()
            .filter(|i| !i.trim().is_empty())
            .or_else(|| Engine::parse(&p.kind).map(|e| e.default_image().to_owned()))
            .unwrap_or_default();
        let (seen, here) = match (&docker, p.build) {
            (Some(d), false) => (
                if observe {
                    observed_image(app, d, project_id, project, &image).await
                } else {
                    None
                },
                present(d, &image).await,
            ),
            _ => (None, false),
        };
        let (each, source) = per_task(seen, p.mem_limit_mb, &p.kind);
        let change = Change {
            wanted: u64::from(p.replicas.max(1)),
            reservation_each: u64::from(p.mem_reservation_mb.unwrap_or(0)) * MIB,
            per_task: each,
            build: p.build,
            pull: !p.build && !here,
            ..Change::default()
        };
        needs.push(need_of(&p.name, &p.kind, change, source));
    }
    let reserved = match &docker {
        Some(d) => reserved(d).await,
        None => None,
    };
    assess(host(app).await, reserved, needs)
}

/// What an existing service is asked to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Deploy,
    Start,
    Restart,
    Rebuild,
}

impl Action {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "deploy" => Some(Self::Deploy),
            "start" => Some(Self::Start),
            "restart" => Some(Self::Restart),
            "rebuild" => Some(Self::Rebuild),
            _ => None,
        }
    }
}

/// Room for `action` on `row`, configured as `snapshot`.
pub async fn for_action(
    app: &AppState,
    row: &service::Model,
    snapshot: &Snapshot,
    action: Action,
) -> Report {
    let docker = app.service::<Docker>();
    let name = snapshot.service_name();
    let running_wanted = snapshot.running || action == Action::Start;
    if let Some(compose) = &snapshot.compose {
        // Running, it is replaced in place; started, it all comes up.
        let up = row.status == "running" && action != Action::Start;
        let need = compose_need(&row.name, compose.inline.as_deref(), up);
        return assess(host(app).await, None, vec![need]);
    }
    let deployed = match &docker {
        Some(d) => d
            .managed_service(&name, &snapshot.project, &snapshot.service)
            .await
            .ok()
            .flatten(),
        None => None,
    };
    let spec = deployed.as_ref().and_then(|s| s.spec.as_ref());
    let template = spec.and_then(|s| s.task_template.as_ref());
    let reserved_each = template
        .and_then(|t| t.resources.as_ref()?.reservations.as_ref()?.memory_bytes)
        .map_or(0, |b| u64::try_from(b).unwrap_or(0));
    // What runs (not what is asked for: a failing task holds nothing).
    let (running, seen) = match (&docker, &deployed) {
        (Some(d), Some(_)) => observed(d, &name).await,
        _ => (0, None),
    };
    let image = snapshot.image.trim();
    // Another image: what the old one uses says nothing of it.
    let same_image = snapshot.build.is_some()
        || template
            .and_then(|t| t.container_spec.as_ref()?.image.as_deref())
            .is_some_and(|i| same_image(i) == same_image(image));
    // Built when deployed, rebuilt, or never built yet.
    let build = snapshot.build.is_some()
        && (matches!(action, Action::Deploy | Action::Rebuild) || deployed.is_none());
    let here = match &docker {
        Some(d) if !image.is_empty() => present(d, image).await,
        _ => false,
    };
    let (each, source) = per_task(
        seen.filter(|_| same_image),
        snapshot.mem_limit_mb,
        &row.kind,
    );
    let change = Change {
        running,
        used_each: seen.unwrap_or(each),
        reserved_each,
        wanted: if running_wanted {
            u64::from(snapshot.replicas)
        } else {
            0
        },
        per_task: each,
        reservation_each: u64::from(snapshot.mem_reservation_mb.unwrap_or(0)) * MIB,
        start_first: snapshot.zero_downtime && snapshot.zero_downtime_possible(),
        build,
        pull: !build && (!here || action == Action::Rebuild),
    };
    let need = need_of(&row.name, &row.kind, change, source);
    let reserved = match &docker {
        Some(d) => reserved(d).await,
        None => None,
    };
    assess(host(app).await, reserved, vec![need])
}

/// `report` as the panel shows it: the host's figures to who sees the
/// server (`server.view`) only.
pub fn json(report: &Report, show_host: bool) -> Value {
    let mut out = serde_json::to_value(report).unwrap_or(Value::Null);
    if !show_host {
        out["host"] = Value::Null;
        if let Some(reasons) = out.get_mut("reasons").and_then(Value::as_array_mut) {
            // Only what it is (not the server's state).
            reasons.retain(|r| {
                matches!(
                    r.as_str(),
                    Some("memory_short" | "disk_short" | "reservation" | "memory_low")
                )
            });
        }
    }
    if let Some((reserved, more)) = report.reserved {
        out["reservation"] = json!({
            "total": reserved.total,
            "used": reserved.used,
            "more": more,
        });
        if !show_host {
            out["reservation"]["total"] = Value::Null;
            out["reservation"]["used"] = Value::Null;
        }
    }
    out
}

/// One line for a deployment's log (`None`: room enough or not known):
/// what it needs, not the server's figures (who reads it may not see them).
pub fn log_line(report: &Report) -> Option<String> {
    let what = match report.level {
        Level::Ok | Level::Unknown => return None,
        Level::Tight => "memory is low",
        Level::Short => "not enough memory or disk",
        Level::Impossible => "its memory reservation does not fit on the node",
    };
    let reasons = json(report, false)["reasons"].clone();
    let reasons: Vec<&str> = reasons
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    Some(format!(
        "capacity: {what} (it needs about {} MB more{}{})",
        report.memory / MIB,
        if reasons.is_empty() { "" } else { "; " },
        reasons.join(", ")
    ))
}

/// Services said short (notified once until they are not).
static SAID: Mutex<BTreeSet<i64>> = Mutex::new(BTreeSet::new());

/// Whether `service` going `short` is news (said once until it is not).
pub fn news(service: i64, short: bool) -> bool {
    let mut said = SAID.lock().unwrap_or_else(|e| e.into_inner());
    if short {
        said.insert(service)
    } else {
        said.remove(&service);
        false
    }
}

/// Checks a deployment as it runs: its log says it, and the channels hear a
/// service short of room (once).
pub async fn check_run(
    app: &AppState,
    row: &service::Model,
    snapshot: &Snapshot,
    trigger: &str,
) -> AppResult<Option<String>> {
    let action = match trigger {
        "start" => Action::Start,
        "restart" => Action::Restart,
        "deploy" | "rollback" => Action::Deploy,
        _ => return Ok(None),
    };
    let report = for_action(app, row, snapshot, action).await;
    let short = matches!(report.level, Level::Short | Level::Impossible);
    // Not known: neither news nor over.
    if report.level != Level::Unknown
        && news(row.id, short)
        && let Some(host) = report.host
    {
        let project = crate::models::project::Entity::find_by_id(row.project_id)
            .one(app.db())
            .await?
            .map(|p| p.name)
            .unwrap_or_default();
        crate::services::notify_service::notify(
            app,
            crate::services::notify_service::Event::Capacity {
                project,
                service: row.name.clone(),
                need_mb: report.memory / MIB,
                available_mb: host.mem_available / MIB,
                total_mb: host.mem_total / MIB,
            },
        );
    }
    Ok(log_line(&report))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(available_mb: u64) -> Host {
        Host {
            mem_total: 957 * MIB,
            mem_available: available_mb * MIB,
            swap_used: 0,
            swap_total: 512 * MIB,
            disk_free: 23 * GIB,
            cpu_percent: Some(10.0),
        }
    }

    fn new(kind: &str, each: u64) -> Need {
        need_of(
            "s",
            kind,
            Change {
                wanted: 1,
                per_task: each,
                ..Change::default()
            },
            Source::Default,
        )
    }

    #[test]
    fn the_levels_follow_what_is_left() {
        // 450 MB in 333: short.
        let short = assess(Some(host(333)), None, vec![new("mysql", typical("mysql"))]);
        assert_eq!(short.level, Level::Short);
        assert_eq!(short.reasons, ["memory_short"]);
        // 320 in 333: 13 left, under 15%.
        let tight = assess(Some(host(333)), None, vec![new("app", 320 * MIB)]);
        assert_eq!(tight.level, Level::Tight);
        assert_eq!(tight.reasons, ["memory_low"]);
        // 32 in 600: room.
        let ok = assess(Some(host(600)), None, vec![new("redis", typical("redis"))]);
        assert_eq!(ok.level, Level::Ok);
        assert!(ok.reasons.is_empty());
    }

    #[test]
    fn swap_cpu_and_disk_make_it_tight() {
        let mut h = host(800);
        h.swap_used = 347 * MIB;
        assert_eq!(
            assess(Some(h), None, vec![new("redis", MIB)]).reasons,
            ["swap_high"]
        );
        let mut h = host(800);
        h.cpu_percent = Some(95.0);
        assert_eq!(
            assess(Some(h), None, vec![new("redis", MIB)]).reasons,
            ["cpu_busy"]
        );
        let mut h = host(800);
        h.disk_free = 2 * GIB + 512 * MIB;
        let mut pulled = new("app", MIB);
        pulled.disk = PULL_DISK;
        let report = assess(Some(h), None, vec![pulled.clone()]);
        assert_eq!(
            (report.level, report.reasons),
            (Level::Tight, vec!["disk_low"])
        );
        h.disk_free = GIB + 512 * MIB;
        let report = assess(Some(h), None, vec![pulled]);
        assert_eq!(
            (report.level, report.reasons),
            (Level::Short, vec!["disk_short"])
        );
    }

    #[test]
    fn nothing_more_or_no_figures_says_nothing() {
        let mut h = host(10);
        h.swap_used = 500 * MIB;
        assert_eq!(assess(Some(h), None, vec![new("app", 0)]).level, Level::Ok);
        assert_eq!(
            assess(None, None, vec![new("mysql", GIB)]).level,
            Level::Unknown
        );
        let mut zero = host(0);
        zero.mem_total = 0;
        assert_eq!(
            assess(Some(zero), None, vec![new("mysql", GIB)]).level,
            Level::Unknown
        );
    }

    #[test]
    fn a_reservation_the_node_lacks_cannot_run() {
        let mut need = new("app", 10 * MIB);
        need.reservation = 1024 * MIB;
        let reserved = Reserved {
            total: 957 * MIB,
            used: 256 * MIB,
        };
        let report = assess(Some(host(900)), Some(reserved), vec![need.clone()]);
        assert_eq!(report.level, Level::Impossible);
        assert_eq!(report.reasons[0], "reservation");
        // Unknown host figures: still impossible.
        assert_eq!(
            assess(None, Some(reserved), vec![need]).level,
            Level::Impossible
        );
        // Within the node: fine.
        let mut fits = new("app", 10 * MIB);
        fits.reservation = 256 * MIB;
        assert_eq!(
            assess(Some(host(900)), Some(reserved), vec![fits]).level,
            Level::Ok
        );
    }

    #[test]
    fn a_task_takes_what_it_uses_else_its_limit_else_its_kinds() {
        assert_eq!(
            per_task(Some(50 * MIB), Some(512), "app"),
            (50 * MIB, Source::Observed)
        );
        assert_eq!(per_task(None, Some(512), "app"), (512 * MIB, Source::Limit));
        assert_eq!(
            per_task(Some(0), None, "mysql"),
            (450 * MIB, Source::Default)
        );
    }

    #[test]
    fn what_a_change_takes() {
        let each = 100 * MIB;
        // Redeployed without downtime: one task more while it rolls.
        let rolled = need_of(
            "web",
            "app",
            Change {
                running: 2,
                used_each: each,
                wanted: 2,
                start_first: true,
                per_task: each,
                ..Change::default()
            },
            Source::Observed,
        );
        assert_eq!((rolled.memory, rolled.temporary), (0, each));
        // Stop-first: nothing more.
        let stopped_first = need_of(
            "web",
            "app",
            Change {
                running: 2,
                used_each: each,
                wanted: 2,
                per_task: each,
                ..Change::default()
            },
            Source::Observed,
        );
        assert_eq!((stopped_first.memory, stopped_first.temporary), (0, 0));
        // Scaled from 1 to 3, built: two more, the build's while it builds.
        let scaled = need_of(
            "web",
            "app",
            Change {
                running: 1,
                used_each: each,
                wanted: 3,
                per_task: each,
                build: true,
                ..Change::default()
            },
            Source::Observed,
        );
        assert_eq!(scaled.memory, 2 * each);
        assert_eq!(scaled.temporary, BUILD_MEMORY);
        assert_eq!(scaled.disk, BUILD_DISK);
        // Reservations: what it reserves after, less what it does now.
        let reserving = need_of(
            "web",
            "app",
            Change {
                running: 1,
                reserved_each: 100 * MIB,
                wanted: 2,
                reservation_each: 200 * MIB,
                start_first: true,
                per_task: each,
                ..Change::default()
            },
            Source::Limit,
        );
        // One old (100) and two new (200): 500 at most, 400 more.
        assert_eq!(reserving.reservation, 400 * MIB);
        // One task, 100 → 600 then 800 → 100, start-first: both at once.
        let grow = |old: u64, new: u64| {
            need_of(
                "web",
                "app",
                Change {
                    running: 1,
                    reserved_each: old * MIB,
                    wanted: 1,
                    reservation_each: new * MIB,
                    start_first: true,
                    ..Change::default()
                },
                Source::Limit,
            )
            .reservation
                / MIB
        };
        assert_eq!((grow(100, 600), grow(800, 100)), (600, 100));
        // Another image: from what the old one uses to what the new takes.
        let bigger = need_of(
            "web",
            "app",
            Change {
                running: 1,
                used_each: 50 * MIB,
                wanted: 1,
                per_task: 192 * MIB,
                ..Change::default()
            },
            Source::Default,
        );
        assert_eq!(bigger.memory, 142 * MIB);
    }

    #[test]
    fn an_image_is_the_same_with_or_without_latest_and_its_digest() {
        assert_eq!(
            same_image("ghcr.io/traefik/whoami"),
            "ghcr.io/traefik/whoami:latest"
        );
        assert_eq!(
            same_image("ghcr.io/traefik/whoami:latest@sha256:abc"),
            "ghcr.io/traefik/whoami:latest"
        );
        assert_eq!(
            same_image("localhost:5000/app"),
            "localhost:5000/app:latest"
        );
        assert_eq!(same_image("mysql:9"), "mysql:9");
    }

    #[test]
    fn a_compose_file_counts_its_services() {
        assert_eq!(
            compose_services("services:\n  web:\n    image: a\n  db:\n    image: b\n"),
            Some(2)
        );
        assert_eq!(compose_services(": nope"), None);
        let need = compose_need("stack", Some("services:\n  a:\n    image: x\n"), false);
        assert_eq!(
            (need.memory, need.source),
            (typical("app"), Source::Default)
        );
        let need = compose_need("stack", None, false);
        assert_eq!((need.memory, need.source), (0, Source::Unknown));
    }

    #[test]
    fn who_does_not_see_the_server_sees_only_the_level_and_the_need() {
        let mut h = host(333);
        h.swap_used = 400 * MIB;
        let report = assess(Some(h), None, vec![new("mysql", typical("mysql"))]);
        let shown = json(&report, false);
        assert!(shown["host"].is_null());
        assert_eq!(shown["reasons"], json!(["memory_short"]));
        assert_eq!(shown["memory"], json!(450 * MIB));
        let full = json(&report, true);
        assert_eq!(full["host"]["mem_available"], json!(333 * MIB));
        assert_eq!(full["reasons"], json!(["memory_short", "swap_high"]));
    }

    #[test]
    fn a_short_service_is_said_once_until_it_is_not() {
        assert!(news(-7, true));
        assert!(!news(-7, true));
        assert!(!news(-7, false));
        assert!(news(-7, true));
        news(-7, false);
    }

    #[test]
    fn the_log_says_it() {
        let report = assess(Some(host(333)), None, vec![new("mysql", typical("mysql"))]);
        assert_eq!(
            log_line(&report).unwrap(),
            "capacity: not enough memory or disk (it needs about 450 MB more; memory_short)"
        );
        let ok = assess(Some(host(900)), None, vec![new("redis", MIB)]);
        assert_eq!(log_line(&ok), None);
    }
}
