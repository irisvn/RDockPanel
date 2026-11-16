//! The steps of an import (docs/import.md). Each job runs one
//! short step under the import's lease, then queues the next in the write
//! that records the new state: a job that dies leaves the lease to expire,
//! and `import-tick` queues the step again. Every step can run again.
//!
//! Forward: queued/preparing → stopping → draining → copying →
//! editing_compose → deploying (a level at a time) → observing → active.
//! A failure before traffic could arrive undoes by itself; after, it waits
//! for the admin (`needs_attention`). Undo: undoing → draining_panel →
//! restoring → reverted.

use std::time::Duration;

use chrono::Utc;
use rustcms::audit::{self, AuditActor, AuditContext, AuditEntry, RequestMeta};
use rustcms::{AppError, AppResult, AppState, Principal};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, ConnectionTrait, EntityTrait, IntoActiveModel,
    QueryFilter, QueryOrder, QuerySelect, Set,
};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::jobs::import_step::ImportStep;
use crate::models::{
    container_import, container_import_claim, container_import_file, container_import_item,
    container_import_volume, deployment, project, service, service_mount,
};
use crate::services::deploy_service::service_name;
use crate::services::deployment_service::{self, Timings, Trigger};
use crate::services::docker_service::{ContainerAction, Docker, HelperMount, HelperSpec};
use crate::services::import_service::{ImportSettings, RUNNING, helper_image, settings};

/// The image of the helpers is missing, a wait ran out, Docker refused: the
/// message the admin reads.
pub(crate) type Failure = String;

/// Where a step leaves the import.
enum Next {
    /// To this state, and queue its step.
    Go(&'static str),
    /// The admin must look: the state to resume at, and why.
    Attention(&'static str, String),
}

fn token_for(id: i64) -> String {
    let nanos = Utc::now().timestamp_nanos_opt().unwrap_or_default();
    let seed = format!("{id}:{nanos}:{}", std::process::id());
    Sha256::digest(seed.as_bytes())
        .iter()
        .take(16)
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Takes the import's lease if it is free or expired; its token.
async fn claim<C: ConnectionTrait>(db: &C, id: i64, lease: Duration) -> AppResult<Option<String>> {
    let token = token_for(id);
    let now = Utc::now();
    container_import::Entity::update_many()
        .col_expr(container_import::Column::LeaseToken, token.clone().into())
        .col_expr(
            container_import::Column::LeaseExpiresAt,
            (now + lease_for(lease)).into(),
        )
        .filter(container_import::Column::Id.eq(id))
        .filter(
            Condition::any()
                .add(container_import::Column::LeaseToken.is_null())
                .add(container_import::Column::LeaseExpiresAt.lt(now)),
        )
        .exec(db)
        .await?;
    // MySQL counts no row for an unchanged one: the token read back tells.
    let row = container_import::Entity::find_by_id(id).one(db).await?;
    Ok(row
        .filter(|r| r.lease_token.as_deref() == Some(&token))
        .map(|_| token))
}

/// Pushes the lease on; false when it is no longer ours.
/// A lease as a `chrono` span.
fn lease_for(lease: Duration) -> chrono::Duration {
    chrono::Duration::from_std(lease).unwrap_or_else(|_| chrono::Duration::seconds(90))
}

async fn renew<C: ConnectionTrait>(
    db: &C,
    id: i64,
    token: &str,
    lease: Duration,
) -> AppResult<bool> {
    container_import::Entity::update_many()
        .col_expr(
            container_import::Column::LeaseExpiresAt,
            (Utc::now() + lease_for(lease)).into(),
        )
        .filter(container_import::Column::Id.eq(id))
        .filter(container_import::Column::LeaseToken.eq(token))
        .exec(db)
        .await?;
    let row = container_import::Entity::find_by_id(id).one(db).await?;
    Ok(row.is_some_and(|r| r.lease_token.as_deref() == Some(token)))
}

async fn release<C: ConnectionTrait>(db: &C, id: i64, token: &str) {
    let _ = container_import::Entity::update_many()
        .col_expr(
            container_import::Column::LeaseToken,
            Option::<String>::None.into(),
        )
        .col_expr(
            container_import::Column::LeaseExpiresAt,
            Option::<chrono::DateTime<Utc>>::None.into(),
        )
        .filter(container_import::Column::Id.eq(id))
        .filter(container_import::Column::LeaseToken.eq(token))
        .exec(db)
        .await;
}

/// What a step works with.
pub(crate) struct Cx<'a> {
    pub(crate) app: &'a AppState,
    pub(crate) docker: std::sync::Arc<Docker>,
    pub(crate) settings: ImportSettings,
    pub(crate) import: container_import::Model,
    pub(crate) items: Vec<container_import_item::Model>,
    pub(crate) token: String,
}

impl Cx<'_> {
    /// Runs `work` and keeps the lease while it does: a copy or a deployment
    /// waits longer than a lease, and a step that lets it lapse would be run
    /// again beside itself.
    pub(crate) async fn keeping<T>(&self, work: impl std::future::Future<Output = T>) -> T {
        tokio::pin!(work);
        let beat = (self.settings.lease / 3).max(Duration::from_millis(50));
        let mut ticker = tokio::time::interval(beat);
        ticker.tick().await;
        loop {
            tokio::select! {
                done = &mut work => return done,
                _ = ticker.tick() => {
                    let _ = renew(self.db(), self.import.id, &self.token, self.settings.lease).await;
                }
            }
        }
    }

    pub(crate) fn db(&self) -> &sea_orm::DatabaseConnection {
        self.app.db()
    }

    pub(crate) async fn alive(&self) -> Result<(), Failure> {
        match renew(self.db(), self.import.id, &self.token, self.settings.lease).await {
            Ok(true) => Ok(()),
            _ => Err("the import's lease was lost".to_owned()),
        }
    }

    async fn project(&self) -> Result<project::Model, Failure> {
        let id = self.import.project_id.ok_or("the import has no project")?;
        project::Entity::find_by_id(id)
            .one(self.db())
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "the project is gone".to_owned())
    }

    pub(crate) async fn service_of(
        &self,
        item: &container_import_item::Model,
    ) -> Option<service::Model> {
        match item.service_id {
            Some(id) => service::Entity::find_by_id(id)
                .one(self.db())
                .await
                .ok()
                .flatten(),
            None => None,
        }
    }

    pub(crate) fn timings(&self) -> Timings {
        Timings {
            poll: self.settings.poll,
            settle: if self.settings.observe.is_zero() {
                Duration::ZERO
            } else {
                Duration::from_secs(4)
            },
            timeout: Duration::from_secs(10 * 60),
        }
    }

    /// Runs a helper container to its end: its exit code.
    pub(crate) async fn helper(
        &self,
        name: &str,
        cmd: Vec<&str>,
        mounts: Vec<HelperMount>,
        cap_add: &[&str],
    ) -> Result<i64, Failure> {
        let image = helper_image(self.app, &self.docker)
            .await
            .ok_or("no helper image: set `[panel] helper_image`")?;
        let spec = HelperSpec {
            name: format!("rdp-import-{}-{name}", self.import.id),
            image,
            cmd: cmd.into_iter().map(str::to_owned).collect(),
            mounts,
            import_id: self.import.id,
            cap_add: cap_add.iter().map(|c| (*c).to_owned()).collect(),
            timeout_s: 600,
        };
        // A helper left by a run that lost its lease is in the way.
        let _ = self
            .docker
            .engine()
            .remove_container(&spec.name, true)
            .await;
        self.keeping(self.docker.engine().run_helper(spec))
            .await
            .map_err(|e| e.to_string())
    }
}

/// Runs the step the import is at, if no other holds it.
pub async fn step(state: &AppState, import_id: i64) -> AppResult<()> {
    let Some(token) = claim(state.db(), import_id, settings(state).lease).await? else {
        return Ok(());
    };
    let outcome = run(state, import_id, &token).await;
    release(state.db(), import_id, &token).await;
    outcome
}

async fn run(app: &AppState, import_id: i64, token: &str) -> AppResult<()> {
    let import = container_import::Entity::find_by_id(import_id)
        .one(app.db())
        .await?
        .ok_or(AppError::NotFound)?;
    if !RUNNING.contains(&import.state.as_str()) {
        return Ok(());
    }
    let items = container_import_item::Entity::find()
        .filter(container_import_item::Column::ImportId.eq(import_id))
        .order_by_asc(container_import_item::Column::Level)
        .order_by_asc(container_import_item::Column::Id)
        .all(app.db())
        .await?;
    let docker = app
        .service::<Docker>()
        .ok_or_else(|| AppError::internal("Docker is not configured"))?;
    let state = import.state.clone();
    let cx = Cx {
        app,
        docker,
        settings: settings(app),
        import,
        items,
        token: token.to_owned(),
    };
    let undoing = matches!(state.as_str(), "undoing" | "draining_panel" | "restoring");
    // An undo asked for while a step held the import: taken here, for a move too.
    if cx.import.undo_requested && !undoing {
        return go(&cx, "undoing", None).await;
    }
    // Moving a folder into a volume has its own steps.
    if cx.import.source_kind == "move" {
        return crate::services::import_move_service::step(&cx).await;
    }
    let result = match state.as_str() {
        "queued" | "preparing" => prepare(&cx).await,
        "stopping" => stopping(&cx).await,
        "draining" => draining(&cx).await,
        "copying" => copying(&cx).await,
        "editing_compose" => editing_compose(&cx).await,
        "deploying" => deploying(&cx).await,
        "observing" => observing(&cx).await,
        "undoing" => undoing_step(&cx).await,
        "draining_panel" => draining_panel(&cx).await,
        "restoring" => restoring(&cx).await,
        _ => return Ok(()),
    };
    match result {
        Ok(Next::Go(next)) => go(&cx, next, None).await,
        Ok(Next::Attention(resume, why)) => attention(&cx, resume, &why).await,
        // A failed undo goes on from where it failed, never from a later step.
        Err(why) if undoing => attention(&cx, resume_of(&state), &why).await,
        Err(why) => {
            // Before traffic could arrive the import undoes itself; after, or
            // when it was asked not to, the admin decides. The mark is read
            // now: a step sets it while it runs.
            let fresh = container_import::Entity::find_by_id(cx.import.id)
                .one(cx.db())
                .await?;
            let ingress = fresh.as_ref().is_some_and(|i| i.has_flag("ingress"));
            if cx.import.auto_undo && !ingress {
                go(&cx, "undoing", Some(&why)).await
            } else {
                attention(&cx, resume_of(&state), &why).await
            }
        }
    }
}

/// Whether the plan makes this item a tunnel of the panel.
fn as_tunnel(cx: &Cx<'_>, item: &container_import_item::Model) -> bool {
    cx.import
        .plan
        .as_deref()
        .and_then(|p| serde_json::from_str::<Value>(p).ok())
        .and_then(|plan| {
            plan["items"]
                .as_array()?
                .iter()
                .find(|i| i["container_id"] == item.container_id)
                .map(|i| i["as_tunnel"] == true)
        })
        .unwrap_or(false)
}

fn resume_of(state: &str) -> &'static str {
    match state {
        "queued" | "preparing" => "preparing",
        "stopping" => "stopping",
        "draining" => "draining",
        "copying" => "copying",
        "editing_compose" => "editing_compose",
        "deploying" => "deploying",
        "observing" => "observing",
        "undoing" => "undoing",
        "draining_panel" => "draining_panel",
        _ => "restoring",
    }
}

/// Moves the import to `next` and queues its step, if the lease is still ours.
/// Whether `task` failed or was refused at or after `since`.
pub(crate) fn failed_since(
    task: &crate::services::docker_service::SwarmTask,
    since: chrono::DateTime<Utc>,
) -> bool {
    use crate::services::docker_service::TaskState;
    let Some(status) = task.status.as_ref() else {
        return false;
    };
    matches!(status.state, Some(TaskState::FAILED | TaskState::REJECTED))
        && status.timestamp.is_some_and(|at| at >= since)
}

pub(crate) async fn go(cx: &Cx<'_>, next: &str, why: Option<&str>) -> AppResult<()> {
    let txn = rustcms::db::begin_write(cx.db()).await?;
    let row = container_import::Entity::find_by_id(cx.import.id)
        .lock_exclusive()
        .one(&txn)
        .await?
        .ok_or(AppError::NotFound)?;
    if row.lease_token.as_deref() != Some(&cx.token) {
        return Ok(());
    }
    // An undo asked for meanwhile is not lost to the end of the way forward.
    let next = if next == "active" && row.undo_requested {
        "undoing"
    } else {
        next
    };
    let mut change = row.into_active_model();
    change.state = Set(next.to_owned());
    if next == "undoing" {
        change.undo_requested = Set(false);
    }
    if let Some(why) = why {
        change.error = Set(Some(why.to_owned()));
    }
    if next == "reverted" || next == "active" {
        change.step = Set(None);
    }
    if next == "reverted" {
        change.finished_at = Set(Some(Utc::now()));
    }
    change.updated_at = Set(Utc::now());
    change.update(&txn).await?;
    if !matches!(next, "active" | "reverted") {
        rustcms::queue::dispatch(
            &txn,
            &ImportStep {
                import_id: cx.import.id,
            },
        )
        .await?;
    }
    txn.commit().await?;
    Ok(())
}

pub(crate) async fn attention(cx: &Cx<'_>, resume: &str, why: &str) -> AppResult<()> {
    let row = container_import::Entity::find_by_id(cx.import.id)
        .one(cx.db())
        .await?
        .ok_or(AppError::NotFound)?;
    if row.lease_token.as_deref() != Some(&cx.token) {
        return Ok(());
    }
    let mut change = row.into_active_model();
    change.state = Set("needs_attention".to_owned());
    // What the admin retries from; the flags (`ingress`, `back`) stay.
    change.step = Set(Some(resume.to_owned()));
    change.error = Set(Some(why.to_owned()));
    change.updated_at = Set(Utc::now());
    change.update(cx.db()).await?;
    Ok(())
}

async fn set_item(
    cx: &Cx<'_>,
    item: &container_import_item::Model,
    change: impl FnOnce(&mut container_import_item::ActiveModel),
) -> Result<(), Failure> {
    let mut active = item.clone().into_active_model();
    change(&mut active);
    active.update(cx.db()).await.map_err(|e| e.to_string())?;
    Ok(())
}

fn running(doc: &Value) -> bool {
    doc["State"]["Running"].as_bool().unwrap_or(false)
        || doc["State"]["Status"].as_str() == Some("running")
}

async fn prepare(cx: &Cx<'_>) -> Result<Next, Failure> {
    for item in &cx.items {
        cx.alive().await?;
        let doc = cx
            .docker
            .engine()
            .inspect_container_raw(&item.container_id, false)
            .await
            .map_err(|e| format!("{}: {e}", item.container_name))?;
        let restart = doc["HostConfig"]["RestartPolicy"]["Name"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let was_running = running(&doc);
        set_item(cx, item, |a| {
            a.original_restart = Set(Some(restart));
            a.was_running = Set(was_running);
        })
        .await?;
    }
    Ok(Next::Go("stopping"))
}

pub(crate) async fn wait_until<F, Fut>(cx: &Cx<'_>, mut done: F) -> Result<bool, Failure>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let until = std::time::Instant::now() + cx.settings.deadline;
    loop {
        if done().await {
            return Ok(true);
        }
        if std::time::Instant::now() >= until {
            return Ok(false);
        }
        cx.alive().await?;
        tokio::time::sleep(cx.settings.poll).await;
    }
}

async fn stopping(cx: &Cx<'_>) -> Result<Next, Failure> {
    let mut items: Vec<_> = cx.items.iter().collect();
    // The last level first: what depends on others stops before them.
    items.sort_by_key(|i| std::cmp::Reverse((i.level, i.id)));
    for item in items {
        cx.alive().await?;
        if item.state != "pending" {
            continue;
        }
        let engine = cx.docker.engine();
        let id = item.container_id.clone();
        // Whether it holds data it writes (a volume or a folder, not read only).
        let writes = match item.service_id {
            Some(service_id) => service_mount::Entity::find()
                .filter(service_mount::Column::ServiceId.eq(service_id))
                .all(cx.db())
                .await
                .map_err(|e| e.to_string())?
                .iter()
                .any(|m| matches!(m.kind.as_str(), "volume" | "link") && !m.read_only),
            None => false,
        };
        if item.was_running {
            // Docker restarts what only stopped: the policy first.
            if item
                .original_restart
                .as_deref()
                .is_some_and(|p| p != "no" && !p.is_empty())
            {
                engine
                    .update_container_restart(&id, "no")
                    .await
                    .map_err(|e| e.to_string())?;
            }
            // What it writes needs time to be written: a long wait before the kill.
            engine
                .stop_container(&id, if writes { 120 } else { 30 })
                .await
                .map_err(|e| format!("{}: {e}", item.container_name))?;
            let stopped = wait_until(cx, || async {
                engine
                    .inspect_container_raw(&id, false)
                    .await
                    .is_ok_and(|d| !running(&d))
            })
            .await?;
            if !stopped {
                return Err(format!("{} did not stop in time", item.container_name));
            }
        }
        let doc = engine
            .inspect_container_raw(&id, false)
            .await
            .map_err(|e| e.to_string())?;
        let code = doc["State"]["ExitCode"].as_i64().unwrap_or(0);
        set_item(cx, item, |a| {
            a.stopped_at = Set(Some(Utc::now()));
            a.state = Set("stopped".to_owned());
        })
        .await?;
        // Killed with data it was writing: the admin decides. Without, nothing is lost.
        if code == 137 && writes {
            // On: the stops that remain, then the rest.
            return Ok(Next::Attention(
                "stopping",
                format!(
                    "{} was stopped by force (exit 137): its data may not be consistent. Continue if that is acceptable, or undo.",
                    item.container_name
                ),
            ));
        }
    }
    Ok(Next::Go("draining"))
}

async fn draining(cx: &Cx<'_>) -> Result<Next, Failure> {
    let engine = cx.docker.engine();
    let ours: Vec<&str> = cx.items.iter().map(|i| i.container_id.as_str()).collect();
    let volumes = container_import_volume::Entity::find()
        .filter(container_import_volume::Column::ImportId.eq(cx.import.id))
        .all(cx.db())
        .await
        .map_err(|e| e.to_string())?;
    let held: Vec<String> = volumes.iter().map(|v| v.original_name.clone()).collect();
    let clear = wait_until(cx, || async {
        let Ok(listed) = engine.containers_detailed().await else {
            return false;
        };
        listed.iter().all(|c| {
            let id = c["Id"].as_str().unwrap_or_default();
            let state = c["State"].as_str().unwrap_or_default();
            // No original still runs, and nobody else mounts what is copied.
            if ours.contains(&id) {
                return state != "running";
            }
            c["Mounts"].as_array().is_none_or(|mounts| {
                mounts.iter().all(|m| {
                    let name = m["Name"]
                        .as_str()
                        .or(m["Source"].as_str())
                        .unwrap_or_default();
                    !held.iter().any(|h| h == name)
                })
            })
        })
    })
    .await?;
    if clear {
        Ok(Next::Go("copying"))
    } else {
        Err("a container still uses the data to copy".to_owned())
    }
}

async fn copying(cx: &Cx<'_>) -> Result<Next, Failure> {
    let project = cx.project().await?;
    let volumes = container_import_volume::Entity::find()
        .filter(container_import_volume::Column::ImportId.eq(cx.import.id))
        .order_by_asc(container_import_volume::Column::Id)
        .all(cx.db())
        .await
        .map_err(|e| e.to_string())?;
    let engine = cx.docker.engine();
    for volume in volumes.iter().filter(|v| v.copied_at.is_none()) {
        cx.alive().await?;
        match volume.mode.as_str() {
            "copy" => {
                let target = volume.panel_volume.clone().ok_or("a copy has no target")?;
                let labels = std::collections::HashMap::from([
                    (
                        crate::services::docker_service::MANAGED_LABEL.to_owned(),
                        "true".to_owned(),
                    ),
                    ("rdockpanel.project".to_owned(), project.name.clone()),
                    ("rdockpanel.import".to_owned(), cx.import.id.to_string()),
                ]);
                // A volume of that name that is not this import's: never written over.
                let existing = engine.volumes().await.map_err(|e| e.to_string())?;
                if let Some(found) = existing.iter().find(|v| v.name == target)
                    && found.labels.get("rdockpanel.import") != Some(&cx.import.id.to_string())
                {
                    return Err(format!(
                        "a volume named {target} exists and is not this import's"
                    ));
                }
                if !existing.iter().any(|v| v.name == target) {
                    engine
                        .create_volume_labelled(&target, labels)
                        .await
                        .map_err(|e| e.to_string())?;
                }
                // Room for the copy, with a margin: a full disk stops the other
                // services and the panel's own database.
                let sizes = engine.volume_sizes().await.unwrap_or_default();
                if let Some(size) = sizes
                    .get(&volume.original_name)
                    .copied()
                    .filter(|size| *size > 0)
                {
                    let need_kb = (size + (size / 10).max(2 * 1024 * 1024 * 1024)) / 1024;
                    let room = cx
                        .helper(
                            &format!("space-{}", volume.id),
                            vec![
                                "sh",
                                "-c",
                                "[ \"$(df -Pk /to | awk 'NR==2{print $4}')\" -gt \"$1\" ]",
                                "sh",
                                &need_kb.to_string(),
                            ],
                            vec![HelperMount {
                                source: target.clone(),
                                target: "/to".to_owned(),
                                read_only: false,
                                kind: "volume".to_owned(),
                            }],
                            &[],
                        )
                        .await?;
                    if room != 0 {
                        return Err(format!(
                            "not enough room to copy {} ({} MiB and a margin)",
                            volume.original_name,
                            size / (1024 * 1024)
                        ));
                    }
                }
                let code = cx
                    .helper(
                        &format!("copy-{}", volume.id),
                        vec!["cp", "-a", "/from/.", "/to/"],
                        vec![
                            HelperMount {
                                source: volume.original_name.clone(),
                                target: "/from".to_owned(),
                                read_only: true,
                                kind: "volume".to_owned(),
                            },
                            HelperMount {
                                source: target,
                                target: "/to".to_owned(),
                                read_only: false,
                                kind: "volume".to_owned(),
                            },
                        ],
                        &["CHOWN", "DAC_OVERRIDE", "FOWNER"],
                    )
                    .await?;
                if code != 0 {
                    return Err(format!(
                        "copying {} failed (exit {code})",
                        volume.original_name
                    ));
                }
            }
            _ => link_folder(cx, volume).await?,
        }
        let mut done = volume.clone().into_active_model();
        done.copied_at = Set(Some(Utc::now()));
        done.update(cx.db()).await.map_err(|e| e.to_string())?;
    }
    // Nothing started again meanwhile: a copy of data that moved is not one.
    for item in &cx.items {
        let doc = engine
            .inspect_container_raw(&item.container_id, false)
            .await
            .map_err(|e| e.to_string())?;
        if running(&doc) {
            return Err(format!(
                "{} started again while copying: the copy is void",
                item.container_name
            ));
        }
    }
    let covers_project = cx.import.source_kind == "project";
    Ok(Next::Go(if covers_project {
        "editing_compose"
    } else {
        "deploying"
    }))
}

/// The symlink of a folder, made by a helper (the panel's own filesystem
/// does not hold it), then read back: it must point where it was recorded.
async fn link_folder(cx: &Cx<'_>, volume: &container_import_volume::Model) -> Result<(), Failure> {
    let mounts = service_mount::Entity::find()
        .filter(service_mount::Column::Kind.eq("link"))
        .filter(service_mount::Column::LinkTarget.eq(volume.original_name.clone()))
        .all(cx.db())
        .await
        .map_err(|e| e.to_string())?;
    let links = cx.settings.links();
    for mount in mounts {
        let (Some(source), Some(target)) = (mount.source, mount.link_target) else {
            continue;
        };
        let inside = source
            .strip_prefix(&format!("{}/", links.to_string_lossy()))
            .ok_or("a link is outside the links folder")?
            .to_owned();
        // The folder and the link are arguments (`$1`, `$2`), never part of the
        // script: a path may hold quotes, `$(…)` or a backtick.
        let script = "link=\"/links/$2\" && mkdir -p \"$(dirname \"$link\")\" && ln -sfn \"$1\" \"$link\" && [ \"$(readlink \"$link\")\" = \"$1\" ]";
        let code = cx
            .helper(
                &format!("link-{}", mount.id),
                vec!["sh", "-c", script, "sh", &target, &inside],
                vec![HelperMount {
                    source: links.to_string_lossy().into_owned(),
                    target: "/links".to_owned(),
                    read_only: false,
                    kind: "bind".to_owned(),
                }],
                &[],
            )
            .await?;
        if code != 0 {
            return Err(format!("the link to {target} could not be made"));
        }
    }
    Ok(())
}

async fn editing_compose(cx: &Cx<'_>) -> Result<Next, Failure> {
    let plan: Value = cx
        .import
        .plan
        .as_deref()
        .and_then(|p| serde_json::from_str(p).ok())
        .unwrap_or(Value::Null);
    let engine = cx.docker.engine();
    let image = helper_image(cx.app, &cx.docker).await;
    // Every container of the project goes: its file would only start them again.
    let whole = plan["items"]
        .as_array()
        .is_some_and(|items| items.iter().all(|i| i["choice"] == "bring"));
    for info in plan["compose"].as_array().into_iter().flatten() {
        for file in info["files"].as_array().into_iter().flatten() {
            // Whole: every container of the import comes along and so does every
            // container of this file's project.
            let project_whole = info["containers"] == info["taken"];
            if !whole || !project_whole || file["read"] != true {
                continue;
            }
            cx.alive().await?;
            let path = file["path"].as_str().unwrap_or_default();
            let renamed = format!("{path}.rdockpanel-imported");
            if container_import_file::Entity::find()
                .filter(container_import_file::Column::ImportId.eq(cx.import.id))
                .filter(container_import_file::Column::Path.eq(path))
                .one(cx.db())
                .await
                .map_err(|e| e.to_string())?
                .is_some()
            {
                continue;
            }
            let image = image.clone().ok_or("no helper image")?;
            let archive = engine
                .read_bind_file(
                    &image,
                    path,
                    crate::services::import_compose_service::MAX_FILE,
                )
                .await
                .map_err(|e| e.to_string())?;
            let text = crate::services::import_compose_service::file_of_archive(&archive)?;
            let sha: String = Sha256::digest(text.as_bytes())
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            if Some(sha.as_str()) != file["sha256"].as_str() {
                return Ok(Next::Attention(
                    "editing_compose",
                    format!("{path} changed since it was read: look at it, then retry"),
                ));
            }
            let (dir, name) = path.rsplit_once('/').ok_or("a path without a folder")?;
            let code = cx
                .helper(
                    "compose",
                    vec![
                        "mv",
                        &format!("/d/{name}"),
                        &format!("/d/{name}.rdockpanel-imported"),
                    ],
                    vec![HelperMount {
                        source: dir.to_owned(),
                        target: "/d".to_owned(),
                        read_only: false,
                        kind: "bind".to_owned(),
                    }],
                    // As root without these, a helper cannot rename in a user's folder.
                    &["DAC_OVERRIDE", "FOWNER"],
                )
                .await?;
            if code != 0 {
                return Err(format!("{path} could not be renamed"));
            }
            container_import_file::ActiveModel {
                import_id: Set(cx.import.id),
                path: Set(path.to_owned()),
                backup_path: Set(renamed),
                sha256: Set(sha),
                action: Set("rename".to_owned()),
                ..Default::default()
            }
            .insert(cx.db())
            .await
            .map_err(|e| e.to_string())?;
        }
    }
    Ok(Next::Go("deploying"))
}

async fn deploying(cx: &Cx<'_>) -> Result<Next, Failure> {
    let project = cx.project().await?;
    let pending: Vec<&container_import_item::Model> = cx
        .items
        .iter()
        .filter(|i| i.state != "deployed" && (i.service_id.is_some() || as_tunnel(cx, i)))
        .collect();
    let Some(level) = pending.iter().map(|i| i.level).min() else {
        return Ok(Next::Go("observing"));
    };
    let group: Vec<_> = pending.into_iter().filter(|i| i.level == level).collect();
    for item in group {
        cx.alive().await?;
        if item.service_id.is_none() && as_tunnel(cx, item) {
            deploy_tunnel(cx, item).await?;
            continue;
        }
        let Some(service) = cx.service_of(item).await else {
            return Err(format!("the service of {} is gone", item.container_name));
        };
        // From here a request may reach the service (a port, a domain).
        let ingress = ports_or_domains(cx, &service).await;
        if ingress {
            let current = container_import::Entity::find_by_id(cx.import.id)
                .one(cx.db())
                .await
                .map_err(|e| e.to_string())?
                .ok_or("the import is gone")?;
            if !current.has_flag("ingress") {
                let flags = container_import::with_flag(current.flags.as_deref(), "ingress");
                let mut mark = current.into_active_model();
                mark.flags = Set(Some(flags));
                mark.update(cx.db()).await.map_err(|e| e.to_string())?;
            }
        }
        let deployed = deploy_service(cx, &service, Trigger::Deploy).await?;
        set_item(cx, item, |a| {
            a.state = Set("deployed".to_owned());
            a.deployment_id = Set(Some(deployed));
        })
        .await?;
        let _ = &project;
    }
    Ok(Next::Go(
        if cx.items.iter().any(|i| {
            i.state != "deployed" && i.level > level && (i.service_id.is_some() || as_tunnel(cx, i))
        }) {
            "deploying"
        } else {
            "observing"
        },
    ))
}

/// The original cloudflared becomes a tunnel of the panel's, named after it:
/// it runs with the token of the original, which is read from Docker again
/// (the plan holds none). Traffic can reach the services from here on.
async fn deploy_tunnel(cx: &Cx<'_>, item: &container_import_item::Model) -> Result<(), Failure> {
    use crate::services::tunnel_service;
    let token =
        crate::services::import_service::tunnel_token_of_container(&cx.docker, &item.container_id)
            .await?;
    let current = container_import::Entity::find_by_id(cx.import.id)
        .one(cx.db())
        .await
        .map_err(|e| e.to_string())?
        .ok_or("the import is gone")?;
    let flags = container_import::with_flag(current.flags.as_deref(), "ingress");
    let mut mark = current.into_active_model();
    mark.flags = Set(Some(flags));
    mark.update(cx.db()).await.map_err(|e| e.to_string())?;
    let _changing = tunnel_service::exclusive().await;
    // A retry keeps the tunnel made before: that row (a name can be made
    // again, by the admin), still with the container's token.
    let made = match item.tunnel_id {
        Some(id) => crate::models::tunnel::Entity::find_by_id(id)
            .one(cx.db())
            .await
            .map_err(|e| e.to_string())?
            .filter(|t| {
                t.token
                    .open(cx.app.keyring(), &crate::models::tunnel::token_context())
                    .is_ok_and(|t| t == token)
            })
            .map(|t| t.name),
        None => None,
    };
    let name = match made {
        Some(name) => name,
        None => {
            let taken: Vec<String> = crate::models::tunnel::all(cx.db())
                .await
                .map_err(|e| e.to_string())?
                .into_iter()
                .map(|t| t.name)
                .collect();
            let base = item
                .compose_service
                .as_deref()
                .unwrap_or(&item.container_name);
            let name = tunnel_service::name_for(base, &taken);
            // The row with the item: an undo removes it, and only it.
            let txn = rustcms::db::begin_write(cx.db())
                .await
                .map_err(|e| e.to_string())?;
            let id = tunnel_service::reserve(cx.app, &txn, &name, &token)
                .await
                .map_err(|e| e.to_string())?;
            let mut kept = item.clone().into_active_model();
            kept.tunnel_id = Set(Some(id));
            kept.update(&txn).await.map_err(|e| e.to_string())?;
            audit::record(&txn, &job_audit(cx), tunnel_entry("tunnel.create", &name))
                .await
                .map_err(|e| e.to_string())?;
            txn.commit().await.map_err(|e| e.to_string())?;
            name
        }
    };
    // Another tunnel's token is refused (one cloudflared per token).
    cx.keeping(tunnel_service::enable(
        cx.app,
        &cx.docker,
        &name,
        Some(&token),
    ))
    .await
    .map_err(|e| e.to_string())?;
    audit::record_detached(
        cx.db(),
        &job_audit(cx),
        tunnel_entry("tunnel.enable", &name),
    )
    .await;
    set_item(cx, item, |a| a.state = Set("deployed".to_owned())).await
}

/// Who changes a tunnel from an import: its step, for the admin who started it.
fn job_audit(cx: &Cx<'_>) -> AuditContext {
    AuditContext::new(
        AuditActor::job("import_step", cx.import.created_by),
        RequestMeta::default(),
    )
}

/// As Settings audits a tunnel: by its name, never its token.
fn tunnel_entry(action: &str, name: &str) -> AuditEntry {
    AuditEntry::new(action, "tunnels").key(name)
}

async fn ports_or_domains(cx: &Cx<'_>, service: &service::Model) -> bool {
    use crate::models::{service_domain, service_port};
    let ports = service_port::Entity::find()
        .filter(service_port::Column::ServiceId.eq(service.id))
        .one(cx.db())
        .await
        .ok()
        .flatten()
        .is_some();
    let domains = service_domain::Entity::find()
        .filter(service_domain::Column::ServiceId.eq(service.id))
        .one(cx.db())
        .await
        .ok()
        .flatten()
        .is_some();
    // A tunnel carries traffic with neither a port nor a domain of its own.
    let tunnel = service.image.contains("cloudflared");
    ports || domains || tunnel
}

/// Asks a deployment of `service` as this import's step, and runs it here
/// (a job that waits for another could hold every worker): its id.
pub(crate) async fn deploy_service(
    cx: &Cx<'_>,
    service: &service::Model,
    trigger: Trigger,
) -> Result<i64, Failure> {
    let txn = rustcms::db::begin_write(cx.db())
        .await
        .map_err(|e| e.to_string())?;
    let asked = deployment_service::request_with(
        &txn,
        service,
        trigger,
        None,
        cx.import.created_by,
        cx.app.keyring(),
        Some(cx.import.id),
    )
    .await
    .map_err(|e| e.to_string())?;
    txn.commit().await.map_err(|e| e.to_string())?;
    cx.keeping(deployment_service::run(cx.app, asked.id, cx.timings()))
        .await
        .map_err(|e| e.to_string())?;
    let done = deployment::Entity::find_by_id(asked.id)
        .one(cx.db())
        .await
        .map_err(|e| e.to_string())?
        .ok_or("the deployment is gone")?;
    if done.status != "succeeded" {
        return Err(format!(
            "{} did not deploy: {}",
            service.name,
            done.error.unwrap_or_default()
        ));
    }
    Ok(asked.id)
}

async fn observing(cx: &Cx<'_>) -> Result<Next, Failure> {
    let project = cx.project().await?;
    let until = std::time::Instant::now() + cx.settings.observe;
    // Only what fails while it is watched: Swarm keeps the failures of the
    // start (a database that was not ready yet), which the deployment accepted.
    let started = Utc::now();
    loop {
        cx.alive().await?;
        for item in &cx.items {
            let Some(service) = cx.service_of(item).await else {
                continue;
            };
            let tasks = cx
                .docker
                .engine()
                .list_tasks(&service_name(&project.name, &service.name))
                .await
                .map_err(|e| e.to_string())?;
            if tasks.iter().any(|t| failed_since(t, started)) {
                return Err(format!("{} has failed tasks", service.name));
            }
        }
        if std::time::Instant::now() >= until {
            return Ok(Next::Go("active"));
        }
        tokio::time::sleep(cx.settings.poll).await;
    }
}

async fn undoing_step(cx: &Cx<'_>) -> Result<Next, Failure> {
    // The tunnels this import made go before the originals run again: their
    // rows, not a tunnel the admin made since under the same name.
    for item in cx.items.iter().filter(|i| i.tunnel_id.is_some()) {
        let row = crate::models::tunnel::Entity::find_by_id(item.tunnel_id.unwrap_or_default())
            .one(cx.db())
            .await
            .map_err(|e| e.to_string())?;
        if let Some(row) = &row {
            let _changing = crate::services::tunnel_service::exclusive().await;
            match cx
                .keeping(crate::services::tunnel_service::remove(
                    cx.app, &cx.docker, &row.name,
                ))
                .await
            {
                Ok(()) | Err(AppError::NotFound) => {}
                Err(err) => return Err(err.to_string()),
            }
        }
        let txn = rustcms::db::begin_write(cx.db())
            .await
            .map_err(|e| e.to_string())?;
        let mut gone = item.clone().into_active_model();
        gone.tunnel_id = Set(None);
        gone.update(&txn).await.map_err(|e| e.to_string())?;
        if let Some(row) = &row {
            audit::record(
                &txn,
                &job_audit(cx),
                tunnel_entry("tunnel.remove", &row.name),
            )
            .await
            .map_err(|e| e.to_string())?;
        }
        txn.commit().await.map_err(|e| e.to_string())?;
    }
    // An import of an older release turned `default` on.
    if cx.import.has_flag("tunnel") {
        let _changing = crate::services::tunnel_service::exclusive().await;
        cx.keeping(crate::services::tunnel_service::disable(
            cx.app,
            &cx.docker,
            crate::services::tunnel_service::DEFAULT,
        ))
        .await
        .map_err(|e| e.to_string())?;
    }
    let mut items: Vec<_> = cx.items.iter().collect();
    items.sort_by_key(|i| std::cmp::Reverse((i.level, i.id)));
    for item in items {
        cx.alive().await?;
        let Some(service) = cx.service_of(item).await else {
            continue;
        };
        // A service never deployed has nothing in Swarm to destroy.
        if service.deployed_at.is_none() && item.state != "deployed" {
            continue;
        }
        deploy_service(cx, &service, Trigger::Destroy).await?;
    }
    Ok(Next::Go("draining_panel"))
}

async fn draining_panel(cx: &Cx<'_>) -> Result<Next, Failure> {
    let project = cx.project().await?;
    let engine = cx.docker.engine();
    let mut names = Vec::new();
    for item in &cx.items {
        if let Some(service) = cx.service_of(item).await {
            names.push(service_name(&project.name, &service.name));
        }
    }
    let gone = wait_until(cx, || async {
        for name in &names {
            if engine.inspect_service(name).await.ok().flatten().is_some() {
                return false;
            }
            if engine.list_tasks(name).await.is_ok_and(|t| !t.is_empty()) {
                return false;
            }
        }
        // Task containers by the service's name, not only its tasks' state.
        let Ok(listed) = engine.containers_detailed().await else {
            return false;
        };
        !listed.iter().any(|c| {
            let service = c["Labels"]["com.docker.swarm.service.name"].as_str();
            service.is_some_and(|s| names.iter().any(|n| n == s))
                && matches!(
                    c["State"].as_str(),
                    Some("running" | "restarting" | "removing")
                )
        })
    })
    .await?;
    if gone {
        Ok(Next::Go("restoring"))
    } else {
        Ok(Next::Attention(
            "draining_panel",
            "a task of the new services is still alive: wait, then retry".to_owned(),
        ))
    }
}

async fn restoring(cx: &Cx<'_>) -> Result<Next, Failure> {
    let engine = cx.docker.engine();
    // The Compose files, as they were.
    for file in container_import_file::Entity::find()
        .filter(container_import_file::Column::ImportId.eq(cx.import.id))
        .filter(container_import_file::Column::RestoredAt.is_null())
        .all(cx.db())
        .await
        .map_err(|e| e.to_string())?
    {
        cx.alive().await?;
        let (dir, name) = file
            .path
            .rsplit_once('/')
            .ok_or("a path without a folder")?;
        let code = cx
            .helper(
                "restore-compose",
                vec![
                    "mv",
                    &format!("/d/{name}.rdockpanel-imported"),
                    &format!("/d/{name}"),
                ],
                vec![HelperMount {
                    source: dir.to_owned(),
                    target: "/d".to_owned(),
                    read_only: false,
                    kind: "bind".to_owned(),
                }],
                &["DAC_OVERRIDE", "FOWNER"],
            )
            .await?;
        if code != 0 {
            return Ok(Next::Attention(
                "restoring",
                format!(
                    "{} could not be put back: look at it, then retry",
                    file.path
                ),
            ));
        }
        let mut done = file.into_active_model();
        done.restored_at = Set(Some(Utc::now()));
        done.update(cx.db()).await.map_err(|e| e.to_string())?;
    }
    // The originals: their policy back, running as they were, lowest level first.
    for item in &cx.items {
        cx.alive().await?;
        if item.state == "restored" {
            continue;
        }
        if let Some(policy) = item
            .original_restart
            .as_deref()
            .filter(|p| !p.is_empty() && *p != "no")
        {
            engine
                .update_container_restart(&item.container_id, policy)
                .await
                .map_err(|e| e.to_string())?;
        }
        // One that never stopped runs already: a start would be refused.
        let runs = engine
            .inspect_container_raw(&item.container_id, false)
            .await
            .is_ok_and(|d| running(&d));
        if item.was_running && !runs {
            let started = wait_until(cx, || async {
                // A port may still be held by a task that is going.
                engine
                    .container_action(&item.container_id, ContainerAction::Start)
                    .await
                    .is_ok()
            })
            .await?;
            if !started {
                return Ok(Next::Attention(
                    "restoring",
                    format!(
                        "{} could not start again: free its ports, then retry",
                        item.container_name
                    ),
                ));
            }
        }
        set_item(cx, item, |a| a.state = Set("restored".to_owned())).await?;
    }
    // The services of the import go from the panel, with what made them.
    let txn = rustcms::db::begin_write(cx.db())
        .await
        .map_err(|e| e.to_string())?;
    let ids: Vec<i64> = cx.items.iter().filter_map(|i| i.service_id).collect();
    if !ids.is_empty() {
        deployment::Entity::delete_many()
            .filter(deployment::Column::ServiceId.is_in(ids.clone()))
            .exec(&txn)
            .await
            .map_err(|e| e.to_string())?;
        service::Entity::delete_many()
            .filter(service::Column::Id.is_in(ids))
            .exec(&txn)
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(project_id) = cx.import.project_id {
        let left = service::Entity::find()
            .filter(service::Column::ProjectId.eq(project_id))
            .limit(1)
            .all(&txn)
            .await
            .map_err(|e| e.to_string())?;
        if left.is_empty() {
            project::Entity::delete_by_id(project_id)
                .exec(&txn)
                .await
                .map_err(|e| e.to_string())?;
        }
    }
    container_import_claim::Entity::delete_many()
        .filter(container_import_claim::Column::ImportId.eq(cx.import.id))
        .exec(&txn)
        .await
        .map_err(|e| e.to_string())?;
    txn.commit().await.map_err(|e| e.to_string())?;
    Ok(Next::Go("reverted"))
}

/// Asks to undo an import: at once when it waits (active, needs the admin),
/// else the step that holds it takes it at its next look.
pub async fn ask_undo(
    app: &AppState,
    principal: &Principal,
    request: RequestMeta,
    id: i64,
) -> AppResult<()> {
    let txn = rustcms::db::begin_write(app.db()).await?;
    let row = container_import::Entity::find_by_id(id)
        .lock_exclusive()
        .one(&txn)
        .await?
        .ok_or(AppError::NotFound)?;
    let waiting = matches!(row.state.as_str(), "active" | "needs_attention");
    if !waiting && !RUNNING.contains(&row.state.as_str()) {
        return Err(AppError::Conflict {
            code: "import_not_undoable",
            reason: "state",
            message: "This import is not under way and not active: there is nothing to undo",
        });
    }
    if matches!(
        row.state.as_str(),
        "undoing" | "draining_panel" | "restoring"
    ) {
        return Ok(());
    }
    let mut change = row.into_active_model();
    change.undo_requested = Set(true);
    if waiting {
        change.state = Set("undoing".to_owned());
        change.undo_requested = Set(false);
        change.step = Set(None);
    }
    change.updated_at = Set(Utc::now());
    change.update(&txn).await?;
    if waiting {
        rustcms::queue::dispatch(&txn, &ImportStep { import_id: id }).await?;
    }
    let entry = AuditEntry::new("imports.undo", "container_imports").key(id.to_string());
    audit::record(
        &txn,
        &AuditContext::new(AuditActor::user(principal), request),
        entry,
    )
    .await?;
    txn.commit().await?;
    Ok(())
}

/// What `import-tick` does each minute: queues the step of an import whose
/// lease expired (its job died), and stops the originals that run again.
pub async fn tick(app: &AppState) -> AppResult<usize> {
    let now = Utc::now();
    let stuck = container_import::Entity::find()
        .filter(container_import::Column::State.is_in(RUNNING))
        .filter(
            Condition::any()
                .add(container_import::Column::LeaseToken.is_null())
                .add(container_import::Column::LeaseExpiresAt.lt(now)),
        )
        .all(app.db())
        .await?;
    for import in &stuck {
        let txn = rustcms::db::begin_write(app.db()).await?;
        rustcms::queue::dispatch(
            &txn,
            &ImportStep {
                import_id: import.id,
            },
        )
        .await?;
        txn.commit().await?;
    }
    watch(app).await?;
    Ok(stuck.len())
}

fn unwatched_key(id: i64) -> String {
    format!("import.{id}.unwatched")
}

/// An original that runs again (`docker compose up`, a person) would write
/// the data the new service writes: stopped again, for as long as the
/// import is kept and the admin has not said to stop watching.
async fn watch(app: &AppState) -> AppResult<()> {
    let Some(docker) = app.service::<Docker>() else {
        return Ok(());
    };
    // A container import, a project's or a partial one: all are watched. A
    // move has no originals.
    let imports = container_import::Entity::find()
        .filter(container_import::Column::SourceKind.ne("move"))
        .filter(container_import::Column::State.is_in([
            "stopping",
            "draining",
            "copying",
            "editing_compose",
            "deploying",
            "observing",
            "active",
            "needs_attention",
            "finished",
        ]))
        .all(app.db())
        .await?;
    if imports.is_empty() {
        return Ok(());
    }
    let engine = docker.engine();
    let Ok(listed) = engine.containers_detailed().await else {
        return Ok(());
    };
    for import in imports {
        // Undoing: the originals are being started again on purpose.
        if matches!(
            import.step.as_deref(),
            Some("undoing" | "draining_panel" | "restoring")
        ) {
            continue;
        }
        if crate::models::panel_setting::get(app.db(), &unwatched_key(import.id))
            .await?
            .is_some()
        {
            continue;
        }
        let plan: Value = import
            .plan
            .as_deref()
            .and_then(|p| serde_json::from_str(p).ok())
            .unwrap_or(Value::Null);
        let items = container_import_item::Entity::find()
            .filter(container_import_item::Column::ImportId.eq(import.id))
            .all(app.db())
            .await?;
        // (original id, Compose project, Compose service), as the plan read them.
        let originals: Vec<(String, Option<String>, Option<String>)> = plan["items"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|i| i["choice"] == "bring")
            .filter_map(|i| {
                Some((
                    i["container_id"].as_str()?.to_owned(),
                    i["compose_project"].as_str().map(str::to_owned),
                    i["compose_service"].as_str().map(str::to_owned),
                ))
            })
            .collect();
        for container in &listed {
            let label = |name: &str| container["Labels"][name].as_str();
            let id = container["Id"].as_str().unwrap_or_default();
            if container["State"] != "running" || label("com.docker.swarm.service.id").is_some() {
                continue;
            }
            // The original, or a container Compose made again for its service.
            let matched = originals.iter().find(|(cid, project, service)| {
                cid == id
                    || (project.is_some()
                        && project.as_deref() == label("com.docker.compose.project")
                        && service.as_deref() == label("com.docker.compose.service"))
            });
            let Some((cid, _, _)) = matched else {
                continue;
            };
            // Already put back by an undo: it is the user's again.
            if items
                .iter()
                .any(|i| &i.container_id == cid && i.state == "restored")
            {
                continue;
            }
            tracing::warn!(
                import = import.id,
                container = id,
                "an imported original runs again: stopped"
            );
            let _ = engine.update_container_restart(id, "no").await;
            let _ = engine.container_action(id, ContainerAction::Stop).await;
        }
    }
    Ok(())
}

/// Where a `needs_attention` import goes on (retry the step it stopped at, or
/// continue after a forced stop): both resume the step, kept in `step`.
pub async fn resume(
    app: &AppState,
    principal: &Principal,
    request: RequestMeta,
    id: i64,
    what: &str,
) -> AppResult<()> {
    let txn = rustcms::db::begin_write(app.db()).await?;
    let row = container_import::Entity::find_by_id(id)
        .lock_exclusive()
        .one(&txn)
        .await?
        .ok_or(AppError::NotFound)?;
    if row.state != "needs_attention" {
        return Err(AppError::Conflict {
            code: "import_not_waiting",
            reason: "state",
            message: "This import is not waiting for you",
        });
    }
    let state = row.step.clone().unwrap_or_else(|| "preparing".to_owned());
    let mut change = row.into_active_model();
    change.state = Set(state);
    change.step = Set(None);
    change.error = Set(None);
    change.updated_at = Set(Utc::now());
    change.update(&txn).await?;
    rustcms::queue::dispatch(&txn, &ImportStep { import_id: id }).await?;
    let entry = AuditEntry::new(format!("imports.{what}"), "container_imports").key(id.to_string());
    audit::record(
        &txn,
        &AuditContext::new(AuditActor::user(principal), request),
        entry,
    )
    .await?;
    txn.commit().await?;
    Ok(())
}

/// Takes the services as they are (all running) as the end of the import.
pub async fn accept(
    app: &AppState,
    principal: &Principal,
    request: RequestMeta,
    id: i64,
) -> AppResult<()> {
    let txn = rustcms::db::begin_write(app.db()).await?;
    let row = container_import::Entity::find_by_id(id)
        .lock_exclusive()
        .one(&txn)
        .await?
        .ok_or(AppError::NotFound)?;
    if row.state != "needs_attention" {
        return Err(AppError::Conflict {
            code: "import_not_waiting",
            reason: "state",
            message: "This import is not waiting for you",
        });
    }
    let items = container_import_item::Entity::find()
        .filter(container_import_item::Column::ImportId.eq(id))
        .all(&txn)
        .await?;
    for item in &items {
        let running = match item.service_id {
            Some(sid) => service::Entity::find_by_id(sid)
                .one(&txn)
                .await?
                .is_some_and(|s| s.status == "running"),
            // A tunnel of the panel: running when it was turned on.
            None => item.state == "deployed",
        };
        if !running {
            return Err(AppError::Conflict {
                code: "import_not_running",
                reason: "state",
                message: "Every service must run to accept the import as it is",
            });
        }
    }
    let mut change = row.into_active_model();
    change.state = Set("active".to_owned());
    change.step = Set(None);
    change.error = Set(None);
    change.updated_at = Set(Utc::now());
    change.update(&txn).await?;
    let entry = AuditEntry::new("imports.accept", "container_imports").key(id.to_string());
    audit::record(
        &txn,
        &AuditContext::new(AuditActor::user(principal), request),
        entry,
    )
    .await?;
    txn.commit().await?;
    Ok(())
}

/// Ends the import: the originals (stopped, kept to undo to) are removed,
/// without their volumes. The copied volumes stay until released.
pub async fn finish(
    app: &AppState,
    principal: &Principal,
    request: RequestMeta,
    id: i64,
) -> AppResult<()> {
    let docker = app
        .service::<Docker>()
        .ok_or_else(|| AppError::internal("Docker is not configured"))?;
    let row = container_import::Entity::find_by_id(id)
        .one(app.db())
        .await?
        .ok_or(AppError::NotFound)?;
    if row.state != "active" {
        return Err(AppError::Conflict {
            code: "import_not_active",
            reason: "state",
            message: "Only an active import can be finished",
        });
    }
    let items = container_import_item::Entity::find()
        .filter(container_import_item::Column::ImportId.eq(id))
        .all(app.db())
        .await?;
    for item in &items {
        match docker
            .engine()
            .remove_container(&item.container_id, false)
            .await
        {
            Ok(()) | Err(crate::services::docker_service::DockerError::NotFound) => {}
            Err(e) => {
                return Err(AppError::BadRequest(format!(
                    "{}: {e}",
                    item.container_name
                )));
            }
        }
    }
    let txn = rustcms::db::begin_write(app.db()).await?;
    let mut change = container_import::Entity::find_by_id(id)
        .lock_exclusive()
        .one(&txn)
        .await?
        .ok_or(AppError::NotFound)?
        .into_active_model();
    change.state = Set("finished".to_owned());
    change.finished_at = Set(Some(Utc::now()));
    change.updated_at = Set(Utc::now());
    change.update(&txn).await?;
    container_import_claim::Entity::delete_many()
        .filter(container_import_claim::Column::ImportId.eq(id))
        .exec(&txn)
        .await?;
    let entry = AuditEntry::new("imports.finish", "container_imports").key(id.to_string());
    audit::record(
        &txn,
        &AuditContext::new(AuditActor::user(principal), request),
        entry,
    )
    .await?;
    txn.commit().await?;
    Ok(())
}

/// Stops the originals that run now (kept stopped, restart off).
pub async fn stop_originals(app: &AppState, id: i64) -> AppResult<usize> {
    let docker = app
        .service::<Docker>()
        .ok_or_else(|| AppError::internal("Docker is not configured"))?;
    let items = container_import_item::Entity::find()
        .filter(container_import_item::Column::ImportId.eq(id))
        .all(app.db())
        .await?;
    let mut stopped = 0;
    for item in items {
        let Ok(doc) = docker
            .engine()
            .inspect_container_raw(&item.container_id, false)
            .await
        else {
            continue;
        };
        if running(&doc) {
            let _ = docker
                .engine()
                .update_container_restart(&item.container_id, "no")
                .await;
            docker
                .engine()
                .container_action(&item.container_id, ContainerAction::Stop)
                .await
                .map_err(|e| AppError::BadRequest(e.to_string()))?;
            stopped += 1;
        }
    }
    Ok(stopped)
}

/// Stops the panel watching the originals of an import.
pub async fn stop_watching(app: &AppState, id: i64) -> AppResult<()> {
    crate::models::panel_setting::set(app.db(), &unwatched_key(id), "1").await?;
    Ok(())
}

/// Lets go of a volume an import copied from: the admin typed its name. With
/// `remove`, the original volume goes too.
pub async fn release_volume(
    app: &AppState,
    principal: &Principal,
    request: RequestMeta,
    import_id: i64,
    volume_id: i64,
    remove: bool,
) -> AppResult<()> {
    let volume = container_import_volume::Entity::find_by_id(volume_id)
        .filter(container_import_volume::Column::ImportId.eq(import_id))
        .one(app.db())
        .await?
        .ok_or(AppError::NotFound)?;
    let import = container_import::Entity::find_by_id(import_id)
        .one(app.db())
        .await?
        .ok_or(AppError::NotFound)?;
    if !matches!(import.state.as_str(), "finished" | "reverted") {
        return Err(AppError::Conflict {
            code: "import_not_finished",
            reason: "state",
            message: "The volumes of an import are released once it is finished",
        });
    }
    if remove && let Some(docker) = app.service::<Docker>() {
        match docker.engine().remove_volume(&volume.original_name).await {
            Ok(()) | Err(crate::services::docker_service::DockerError::NotFound) => {}
            Err(e) => return Err(AppError::BadRequest(e.to_string())),
        }
    }
    let txn = rustcms::db::begin_write(app.db()).await?;
    let mut change = volume.into_active_model();
    change.released_at = Set(Some(Utc::now()));
    change.update(&txn).await?;
    let entry = AuditEntry::new("imports.volume_release", "container_imports")
        .key(import_id.to_string())
        .meta(serde_json::json!({ "volume": volume_id, "removed": remove }));
    audit::record(
        &txn,
        &AuditContext::new(AuditActor::user(principal), request),
        entry,
    )
    .await?;
    txn.commit().await?;
    Ok(())
}
