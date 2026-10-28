//! Settings → Version's **Update** button: the panel's own image moved to a
//! newer release, only when an administrator asks (the panel never updates
//! by itself).
//!
//! 1. The update is recorded first (`panel.update`, from → to, the digest):
//!    one at a time; asked twice, refused.
//! 2. A maintenance gate stops new deployments, backups and restores
//!    ([`refuse_while_updating`]); the update waits until those running end.
//! 3. The panel's database is copied to the backups directory: `mysqldump`
//!    (MySQL) or `VACUUM INTO` (SQLite).
//! 4. Swarm moves the service to the release's image, pinned by the digest
//!    the registry gave, with a rollback when the new task does not become
//!    healthy (`update_config.failure_action = rollback`).
//! 5. The panel that starts next (the new one, or the old one rolled back)
//!    reconciles: the service's update state and the version it runs say
//!    how it ended; the gate is lifted. A gate left by a panel that never
//!    comes back expires ([`GATE_FOR`]).
//!
//! The registry is trusted (images are not signed): the reference is always
//! `ghcr.io/irisvn/rdockpanel:<version>`, the digest asked by the panel.

use std::time::Duration;

use chrono::{DateTime, Utc};
use rustcms::{AppError, AppResult, AppState, ValidationErrors};
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::models::{action, backup_run, deployment, panel_setting};
use crate::services::action_service::{self, Initiator};
use crate::services::docker_service::{Docker, ExecChunk};
use crate::services::system_service::SELF_SERVICE;
use crate::services::version_service;

/// The action of an update.
pub const KIND: &str = "panel.update";
/// Until when new work waits (RFC 3339).
const GATE_KEY: &str = "panel.update.gate";
/// How long the gate holds at most.
const GATE_FOR: chrono::Duration = chrono::Duration::minutes(30);
/// How long the update waits for running work to end.
const DRAIN_FOR: Duration = Duration::from_secs(10 * 60);
/// How long the next panel waits for Swarm to settle the update.
const SETTLE_FOR: Duration = Duration::from_secs(20 * 60);
/// Where the database is dumped first (the installer's backups directory,
/// the same path in the panel's container).
const DUMPS: &str = "/var/lib/rdockpanel/backups/panel";

/// What an update records (its action's log, JSON).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Plan {
    pub from: String,
    pub to: String,
    /// `ghcr.io/irisvn/rdockpanel:<to>@<digest>`.
    pub image: String,
    /// The image it ran before (a rollback's).
    pub previous: Option<String>,
    /// The database's dump, once made.
    #[serde(default)]
    pub dump: Option<String>,
    #[serde(default)]
    pub stage: String,
}

/// The gate's row, made once (at boot): admissions lock it shared, an update
/// exclusively, so none slips between the gate closing and the wait for
/// what runs.
pub async fn ensure_gate<C: sea_orm::ConnectionTrait>(db: &C) -> AppResult<()> {
    // Inserted when missing, in one statement: an existing (closed) gate
    // is never reset (`ON DUPLICATE KEY UPDATE key = key` on MySQL).
    use sea_orm::Set;
    use sea_orm::sea_query::OnConflict;
    panel_setting::Entity::insert(panel_setting::ActiveModel {
        key: Set(GATE_KEY.to_owned()),
        value: Set(String::new()),
        updated_at: Set(Utc::now()),
    })
    .on_conflict(
        OnConflict::column(panel_setting::Column::Key)
            .do_nothing_on([panel_setting::Column::Key])
            .to_owned(),
    )
    .exec_without_returning(db)
    .await?;
    Ok(())
}

/// Starts a restart or a reboot (`kind`) under the gate's lock: refused
/// while an update runs; an update asked meanwhile sees it running.
pub async fn announce_disruptive(
    db: &sea_orm::DatabaseConnection,
    by: &Initiator,
    kind: &str,
    target: &str,
) -> AppResult<action::Model> {
    use sea_orm::QuerySelect;
    ensure_gate(db).await?;
    let txn = rustcms::db::begin_write(db).await?;
    let gate = panel_setting::Entity::find_by_id(GATE_KEY.to_owned())
        .lock_exclusive()
        .one(&txn)
        .await?;
    if gate.is_some_and(|g| gate_until(&g.value).is_some()) || running(&txn).await?.is_some() {
        return Err(AppError::Conflict {
            code: "panel_updating",
            reason: "updating",
            message: "The panel is updating: wait until it ends",
        });
    }
    let row = action_service::start(&txn, by, kind, target, action::RUNNING).await?;
    let entry = rustcms::audit::AuditEntry::new(format!("system.{kind}"), "system")
        .key(target)
        .meta(json!({ "action": row.id }));
    rustcms::audit::record(&txn, &by.audit, entry).await?;
    txn.commit().await?;
    Ok(row)
}

fn gate_until(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|t| t.with_timezone(&Utc))
        .filter(|until| *until > Utc::now())
}

/// Refuses new deployments, backups and restores while the panel updates.
/// In the admission's transaction: the gate's row locked (shared) until it
/// commits, so an update closing the gate waits for it, then sees it.
pub async fn refuse_while_updating<C: sea_orm::ConnectionTrait>(txn: &C) -> AppResult<()> {
    use sea_orm::QuerySelect;
    let until = panel_setting::Entity::find_by_id(GATE_KEY.to_owned())
        .lock_shared()
        .one(txn)
        .await?
        .and_then(|row| gate_until(&row.value));
    match until {
        Some(until) if until > Utc::now() => Err(AppError::Conflict {
            code: "panel_updating",
            reason: "updating",
            message: "The panel is updating: try again in a few minutes",
        }),
        _ => Ok(()),
    }
}

async fn open_gate<C: sea_orm::ConnectionTrait>(db: &C) {
    if let Err(err) = panel_setting::set(db, GATE_KEY, "").await {
        tracing::warn!(%err, "the update's gate not lifted (it expires)");
    }
}

/// The running update, if any.
pub async fn running<C: sea_orm::ConnectionTrait>(db: &C) -> AppResult<Option<action::Model>> {
    Ok(action::Entity::find()
        .filter(action::Column::Kind.eq(KIND))
        .filter(action::Column::Status.is_in([action::QUEUED, action::RUNNING]))
        .one(db)
        .await?)
}

fn plan_of(row: &action::Model) -> Option<Plan> {
    serde_json::from_str(row.log.as_deref()?).ok()
}

async fn save_plan<C: sea_orm::ConnectionTrait>(
    db: &C,
    row: action::Model,
    plan: &Plan,
) -> AppResult<action::Model> {
    let log = serde_json::to_string(plan).map_err(AppError::internal)?;
    action_service::note(db, row, &log).await
}

/// Starts the update to `version` (the newest release, as the last check
/// found it): recorded, then run in the background. Its action.
pub async fn start(
    app: &AppState,
    docker: &Docker,
    by: &Initiator,
    version: &str,
    installed: bool,
) -> AppResult<action::Model> {
    let refuse = |message: &str| -> AppError { ValidationErrors::field("version", message).into() };
    if !installed {
        return Err(refuse(
            "the panel runs from source: update it with git pull and a new build",
        ));
    }
    let Some((image, _)) = crate::services::system_service::own_image(docker).await? else {
        return Err(refuse("the panel's own service is not found"));
    };
    let check = version_service::last(app.db()).await;
    let update = version_service::update_of(&check, version_service::channel(Some(&image)));
    if update.as_deref() != Some(version) {
        return Err(refuse(
            "is not the newest release, newer than this one (check again)",
        ));
    }
    if running(app.db()).await?.is_some() {
        return Err(busy_updating());
    }
    // Pinned by what the registry serves now (the panel asks; never the
    // browser's).
    let reference = format!("ghcr.io/{}:{version}", version_service::REPOSITORY);
    let digest = docker
        .engine()
        .registry_digest(&reference)
        .await
        .map_err(|e| AppError::BadRequest(format!("the registry: {e}")))?
        .ok_or_else(|| {
            AppError::BadRequest("the registry did not give the release's digest".into())
        })?;
    let plan = Plan {
        from: env!("RDP_VERSION").to_owned(),
        to: version.to_owned(),
        image: format!("{reference}@{digest}"),
        previous: Some(image),
        dump: None,
        stage: "queued".to_owned(),
    };
    // Reserved at once: the gate's row locked, no other update running,
    // the action recorded (and audited) and the gate closed together.
    ensure_gate(app.db()).await?;
    let txn = rustcms::db::begin_write(app.db()).await?;
    {
        use sea_orm::QuerySelect;
        let gate = panel_setting::Entity::find_by_id(GATE_KEY.to_owned())
            .lock_exclusive()
            .one(&txn)
            .await?;
        if gate.is_some_and(|g| gate_until(&g.value).is_some()) || running(&txn).await?.is_some() {
            return Err(busy_updating());
        }
        // A restart or a reboot running: it would cut the update (one older
        // than a quarter of an hour did not happen: a failed reboot command;
        // its helper runs `sleep 3; systemctl reboot` and ends at once — one
        // hung longer is a risk taken).
        let disruptive = action::Entity::find()
            .filter(action::Column::Kind.is_in(crate::services::system_service::DISRUPTIVE))
            .filter(action::Column::Status.eq(action::RUNNING))
            .filter(action::Column::CreatedAt.gt(Utc::now() - chrono::Duration::minutes(15)))
            .one(&txn)
            .await?;
        if disruptive.is_some() {
            return Err(AppError::Conflict {
                code: "panel_restarting",
                reason: "busy",
                message: "The panel or the server is restarting: update once it is back",
            });
        }
    }
    let target = format!("{} → {}", plan.from, plan.to);
    let row = action_service::start(&txn, by, KIND, &target, action::RUNNING).await?;
    let entry = rustcms::audit::AuditEntry::new(format!("system.{KIND}"), "system")
        .key(target)
        .meta(json!({ "action": row.id, "image": plan.image }));
    rustcms::audit::record(&txn, &by.audit, entry).await?;
    let row = save_plan(&txn, row, &plan).await?;
    panel_setting::set(&txn, GATE_KEY, &(Utc::now() + GATE_FOR).to_rfc3339()).await?;
    txn.commit().await?;
    let (app, id) = (app.clone(), row.id);
    tokio::spawn(async move {
        if let Err(why) = run(&app, id).await {
            tracing::warn!(error = why, "the panel's update stopped");
            if let Ok(Some(row)) = action::Entity::find_by_id(id).one(app.db()).await {
                let _ = action_service::finish(app.db(), row, Some(why), None).await;
            }
            open_gate(app.db()).await;
        }
    });
    Ok(row)
}

fn busy_updating() -> AppError {
    AppError::Conflict {
        code: "panel_update_running",
        reason: "busy",
        message: "An update is already running",
    }
}

/// Deployments and backup runs not ended.
async fn busy<C: sea_orm::ConnectionTrait>(db: &C) -> AppResult<u64> {
    let deployments = deployment::Entity::find()
        .filter(deployment::Column::Status.is_in([
            crate::services::deployment_service::QUEUED,
            crate::services::deployment_service::RUNNING,
        ]))
        .count(db)
        .await?;
    let backups = backup_run::Entity::find()
        .filter(backup_run::Column::Status.is_in(["queued", "running", "killing"]))
        .count(db)
        .await?;
    Ok(deployments + backups)
}

/// The update's steps once recorded (an error: why it stopped).
async fn run(app: &AppState, id: i64) -> Result<(), String> {
    let db = app.db();
    let docker = app
        .service::<Docker>()
        .ok_or_else(|| "Docker is not bound".to_owned())?;
    let row = action::Entity::find_by_id(id)
        .one(db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "the update's action is gone".to_owned())?;
    let mut plan = plan_of(&row).ok_or_else(|| "the update's plan is unreadable".to_owned())?;
    // 2. What runs ends first (nothing new starts: the gate).
    plan.stage = "waiting".to_owned();
    let mut row = save_plan(db, row, &plan).await.map_err(|e| e.to_string())?;
    let deadline = tokio::time::Instant::now() + DRAIN_FOR;
    let mut idle = 0;
    while idle < 2 {
        let left = busy(db).await.map_err(|e| e.to_string())?;
        idle = if left == 0 { idle + 1 } else { 0 };
        if idle < 2 {
            if tokio::time::Instant::now() >= deadline {
                return Err(format!(
                    "{left} deployments or backups still running after 10 minutes: the panel was not updated"
                ));
            }
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    }
    // 3. The database, dumped.
    plan.stage = "backing up".to_owned();
    row = save_plan(db, row, &plan).await.map_err(|e| e.to_string())?;
    plan.dump = Some(
        if db.get_database_backend() == sea_orm::DatabaseBackend::Sqlite {
            snapshot(db, DUMPS, &plan.from, id).await?
        } else {
            dump(&docker, &plan.from, id).await?
        },
    );
    // 4. The service moved to the release, rolled back if it fails.
    plan.stage = "updating".to_owned();
    save_plan(db, row, &plan).await.map_err(|e| e.to_string())?;
    move_to(&docker, &plan.image).await
}

/// A copy of the panel's SQLite database in `dir` (`VACUUM INTO`: what is
/// committed, the write-ahead log's too, in one consistent file), checked
/// whole before the update goes on: the file's path.
pub async fn snapshot(
    db: &sea_orm::DatabaseConnection,
    dir: &str,
    from: &str,
    id: i64,
) -> Result<String, String> {
    use sea_orm::ConnectionTrait;
    if !from
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))
    {
        return Err(format!("{from:?} is not a version"));
    }
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|e| format!("{dir}: {e}"))?;
    let path = format!(
        "{dir}/rdockpanel-{from}-{}-{id}.sqlite",
        Utc::now().format("%Y%m%d-%H%M%S")
    );
    // Outside a transaction (VACUUM cannot run in one); a file already
    // there is refused, never overwritten.
    db.execute_unprepared(&format!("VACUUM INTO '{}'", path.replace('\'', "''")))
        .await
        .map_err(|e| format!("the database's copy failed: {e}"))?;
    if let Err(e) = intact(&path).await {
        let _ = tokio::fs::remove_file(&path).await;
        return Err(format!("the database's copy is not whole: {e}"));
    }
    Ok(path)
}

/// `PRAGMA integrity_check` of the SQLite file at `path`, opened read-only.
async fn intact(path: &str) -> Result<(), String> {
    use sea_orm::ConnectionTrait;
    let copy = sea_orm::Database::connect(format!("sqlite://{path}?mode=ro"))
        .await
        .map_err(|e| e.to_string())?;
    let row = copy
        .query_one_raw(sea_orm::Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "PRAGMA integrity_check",
        ))
        .await
        .map_err(|e| e.to_string());
    let _ = copy.close().await;
    let verdict: String = row?
        .ok_or("no answer")?
        .try_get_by_index(0)
        .map_err(|e| e.to_string())?;
    if verdict == "ok" {
        Ok(())
    } else {
        Err(verdict)
    }
}

/// `mysqldump` of the panel's database (in `rdp-mysql`), to [`DUMPS`]: the
/// file's path.
async fn dump(docker: &Docker, from: &str, id: i64) -> Result<String, String> {
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;
    let containers = docker
        .engine()
        .containers()
        .await
        .map_err(|e| e.to_string())?;
    let mysql = containers
        .iter()
        .find(|c| {
            c.state == "running"
                && c.labels
                    .get("com.docker.swarm.service.name")
                    .map(String::as_str)
                    == Some("rdp-mysql")
        })
        .ok_or_else(|| "the panel's database (rdp-mysql) is not running".to_owned())?;
    tokio::fs::create_dir_all(DUMPS)
        .await
        .map_err(|e| format!("{DUMPS}: {e}"))?;
    let path = format!(
        "{DUMPS}/rdockpanel-{from}-{}-{id}.sql",
        Utc::now().format("%Y%m%d-%H%M%S")
    );
    let cmd = [
        "sh",
        "-c",
        r#"MYSQL_PWD="$(cat /run/secrets/rdp_mysql_root)" exec mysqldump -uroot --single-transaction --routines --triggers --databases "$MYSQL_DATABASE""#,
    ]
    .map(str::to_owned)
    .to_vec();
    let mut out = docker
        .engine()
        .run_in_container(&mysql.id, cmd, "rdp-panel-update", None)
        .await
        .map_err(|e| e.to_string())?;
    let mut file = tokio::fs::File::create(&path)
        .await
        .map_err(|e| format!("{path}: {e}"))?;
    let mut errors = Vec::new();
    let mut code = None;
    while let Some(chunk) = out.next().await {
        match chunk.map_err(|e| e.to_string())? {
            ExecChunk::Out(bytes) => file
                .write_all(&bytes)
                .await
                .map_err(|e| format!("{path}: {e}"))?,
            ExecChunk::Err(bytes) => errors.extend(bytes),
            ExecChunk::Exit(c) => code = Some(c),
        }
    }
    file.flush().await.map_err(|e| format!("{path}: {e}"))?;
    if code != Some(0) {
        let _ = tokio::fs::remove_file(&path).await;
        return Err(format!(
            "the database's dump failed: {}",
            String::from_utf8_lossy(&errors).trim()
        ));
    }
    Ok(path)
}

/// The panel's service on `image`, with a rollback when its new task does
/// not become healthy (Swarm decides; this process is stopped meanwhile).
async fn move_to(docker: &Docker, image: &str) -> Result<(), String> {
    let engine = docker.engine();
    let service = engine
        .inspect_service(SELF_SERVICE)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "the panel's service is gone".to_owned())?;
    let version = service.version.and_then(|v| v.index).unwrap_or(0);
    let spec = update_spec(
        serde_json::to_value(service.spec.unwrap_or_default()).map_err(|e| e.to_string())?,
        image,
    );
    let spec = serde_json::from_value(spec).map_err(|e| e.to_string())?;
    engine
        .update_service(SELF_SERVICE, version, spec, None)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

const SECOND: i64 = 1_000_000_000;

/// How the panel's health is checked and its update watched (seconds).
pub struct Timing {
    pub interval: i64,
    pub retries: i64,
    /// Migrations may run that long before failures count.
    pub start_period: i64,
    /// Longer than a failure takes to be seen (interval × retries).
    pub monitor: i64,
}

/// The panel's (the installer gives the same).
pub const PANEL: Timing = Timing {
    interval: 5,
    retries: 6,
    start_period: 180,
    monitor: 90,
};

/// `spec` (Docker's JSON) on `image`: its health checked often enough for
/// Swarm to see a failure within the update's monitor (migrations given
/// their start period), rolled back on failure.
pub fn update_spec(spec: Value, image: &str) -> Value {
    update_spec_with(spec, image, &PANEL)
}

/// [`update_spec`] with its `timing`.
pub fn update_spec_with(mut spec: Value, image: &str, timing: &Timing) -> Value {
    let container = &mut spec["TaskTemplate"]["ContainerSpec"];
    container["Image"] = json!(image);
    let test = container["HealthCheck"]["Test"].clone();
    container["HealthCheck"] = json!({
        "Test": if test.is_array() { test } else { json!(["CMD", "curl", "-fsS", "http://127.0.0.1:3000/health"]) },
        "Interval": timing.interval * SECOND,
        "Timeout": 3 * SECOND,
        "Retries": timing.retries,
        "StartPeriod": timing.start_period * SECOND,
    });
    spec["UpdateConfig"] = json!({
        "Parallelism": 1,
        "FailureAction": "rollback",
        // Longer than a failure takes to be seen (6 × 5 s after a start).
        "Monitor": timing.monitor * SECOND,
        "MaxFailureRatio": 0,
        "Order": "stop-first",
    });
    spec["RollbackConfig"] = json!({
        "Parallelism": 1,
        "FailureAction": "pause",
        "Monitor": timing.monitor * SECOND,
        "MaxFailureRatio": 0,
        "Order": "stop-first",
    });
    spec
}

/// How Swarm left an update (`UpdateStatus.State`), as the history says it.
pub fn outcome(state: Option<&str>, runs: &str, to: &str) -> Option<Result<String, String>> {
    match state {
        Some("updating" | "rollback_started") => None,
        Some("rollback_completed") => Some(Err(format!(
            "the new version did not start: rolled back to {runs}"
        ))),
        Some("paused" | "rollback_paused") => Some(Err(format!(
            "Swarm paused the update ({}): see docker service ps {SELF_SERVICE}",
            state.unwrap_or_default()
        ))),
        _ if runs == to => Some(Ok(format!("updated to {to}"))),
        _ => Some(Err(format!("{runs} runs, not {to}"))),
    }
}

/// At boot: an update left running is settled — by what Swarm says, once
/// it is done with it — and the gate lifted.
pub fn reconcile(app: &AppState) {
    let app = app.clone();
    tokio::spawn(async move {
        let db = app.db();
        let Ok(Some(row)) = running(db).await else {
            return;
        };
        let Some(plan) = plan_of(&row) else {
            let _ = action_service::finish(db, row, Some("unreadable".to_owned()), None).await;
            open_gate(db).await;
            return;
        };
        // The update not yet given to Swarm: the panel stopped before.
        if plan.stage != "updating" {
            let _ = action_service::finish(
                db,
                row,
                Some(format!("interrupted ({}): not updated", plan.stage)),
                None,
            )
            .await;
            open_gate(db).await;
            return;
        }
        let deadline = tokio::time::Instant::now() + SETTLE_FOR;
        let runs = env!("RDP_VERSION");
        let ended = loop {
            // Docker not answering (or the service not found): asked again,
            // never taken for an end.
            let inspected = match app.service::<Docker>() {
                Some(docker) => docker
                    .engine()
                    .inspect_service(SELF_SERVICE)
                    .await
                    .ok()
                    .flatten(),
                None => None,
            };
            if let Some(service) = inspected {
                let state = service
                    .update_status
                    .and_then(|u| u.state)
                    .map(|s| s.to_string());
                if let Some(ended) = outcome(state.as_deref(), runs, &plan.to) {
                    break ended;
                }
            }
            if tokio::time::Instant::now() >= deadline {
                break Err("Swarm did not settle the update in 20 minutes".to_owned());
            }
            tokio::time::sleep(Duration::from_secs(10)).await;
        };
        let (error, log) = match ended {
            Ok(done) => (None, done),
            Err(why) => (Some(why.clone()), why),
        };
        let note = match &plan.dump {
            Some(dump) => format!("{log} (the database before it: {dump})"),
            None => log,
        };
        if let Err(err) = action_service::finish(db, row, error, Some(&note)).await {
            tracing::warn!(%err, "the update's end not recorded");
        }
        open_gate(db).await;
    });
}

/// The running update's stage, for Settings → Version.
pub async fn progress<C: sea_orm::ConnectionTrait>(db: &C) -> AppResult<Option<Value>> {
    Ok(running(db).await?.and_then(|row| {
        let plan = plan_of(&row)?;
        Some(json!({ "from": plan.from, "to": plan.to, "stage": plan.stage, "at": row.created_at }))
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_update_rolls_back_and_its_health_is_seen_in_time() {
        let spec = json!({
            "Name": "rdockpanel",
            "TaskTemplate": {"ContainerSpec": {"Image": "ghcr.io/irisvn/rdockpanel:0.5.0@sha256:a",
                "HealthCheck": {"Test": ["CMD-SHELL", "curl -fsS http://127.0.0.1:3000/health"], "Interval": 5 * SECOND, "Retries": 60}}},
        });
        let spec = update_spec(spec, "ghcr.io/irisvn/rdockpanel:0.5.1@sha256:b");
        let c = &spec["TaskTemplate"]["ContainerSpec"];
        assert_eq!(c["Image"], "ghcr.io/irisvn/rdockpanel:0.5.1@sha256:b");
        assert_eq!(c["HealthCheck"]["Test"][0], "CMD-SHELL");
        let seen = c["HealthCheck"]["Interval"].as_i64().unwrap()
            * c["HealthCheck"]["Retries"].as_i64().unwrap();
        assert!(seen < spec["UpdateConfig"]["Monitor"].as_i64().unwrap());
        assert_eq!(spec["UpdateConfig"]["FailureAction"], "rollback");
        assert_eq!(spec["RollbackConfig"]["FailureAction"], "pause");
        assert_eq!(spec["Name"], "rdockpanel");
    }

    #[test]
    fn how_an_update_ended() {
        assert_eq!(outcome(Some("updating"), "0.5.1", "0.5.1"), None);
        assert_eq!(
            outcome(Some("completed"), "0.5.1", "0.5.1"),
            Some(Ok("updated to 0.5.1".to_owned()))
        );
        assert!(
            matches!(outcome(Some("rollback_completed"), "0.5.0", "0.5.1"), Some(Err(why)) if why.contains("rolled back to 0.5.0"))
        );
        assert!(matches!(
            outcome(Some("paused"), "0.5.0", "0.5.1"),
            Some(Err(_))
        ));
        assert!(matches!(outcome(None, "0.5.0", "0.5.1"), Some(Err(_))));
        assert_eq!(
            outcome(None, "0.5.1", "0.5.1"),
            Some(Ok("updated to 0.5.1".to_owned()))
        );
    }
}
