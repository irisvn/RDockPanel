//! "Move all" and "Clean the old folder" (docs/import.md): a
//! service that uses a host folder in place (a `link` mount) gets a volume of
//! the panel's with the same data. It runs on the import machinery
//! (`source_kind = 'move'`): the same lease, claims and guards; the steps
//! are these. Stop → copy → switch the mounts (in one write, with its
//! audit entry) → deploy → observe. Before the switch a failure undoes by
//! itself (the service starts again on the folder); after, the admin
//! decides. Undoing after a move puts the link back; what was written to the
//! volume meanwhile stays there.

use chrono::Utc;
use rustcms::audit::{self, AuditActor, AuditContext, AuditEntry, RequestMeta};
use rustcms::{AppError, AppResult, AppState, Principal, ValidationErrors};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect, Set,
};
use serde_json::json;

use crate::models::{
    container_import, container_import_item, container_import_volume, project, service,
    service_mount,
};
use crate::services::deploy_rules_service::forbidden_path;
use crate::services::deploy_service::volume_name;
use crate::services::deployment_service::Trigger;
use crate::services::docker_service::HelperMount;
use crate::services::import_run_service::{Cx, Failure, attention, deploy_service, go};
use crate::services::import_service::{link_path, settings};

fn last(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_owned()
}

/// Whether two host paths are the same folder, or one is inside the other
/// (compared by their parts: `/srv/a` and `/srv/ab` do not overlap).
fn overlaps(a: &str, b: &str) -> bool {
    let (a, b) = (std::path::Path::new(a), std::path::Path::new(b));
    a.starts_with(b) || b.starts_with(a)
}

/// Who else uses `folder` (or a folder inside or above it): another service of
/// the panel, or a container of Docker's (stopped ones too). The originals of
/// an import (kept stopped to undo to), the panel's helpers and the tasks of
/// the service itself do not count. A name, said to the admin.
pub(crate) async fn folder_used_elsewhere(
    app: &AppState,
    folder: &str,
    service_id: i64,
) -> AppResult<Option<String>> {
    let others = service_mount::Entity::find()
        .filter(service_mount::Column::ServiceId.ne(service_id))
        .filter(service_mount::Column::Kind.is_in(["bind", "link"]))
        .all(app.db())
        .await?;
    for mount in others {
        let path = match mount.kind.as_str() {
            "link" => mount.link_target,
            _ => mount.source,
        };
        if path.as_deref().is_some_and(|p| overlaps(p, folder)) {
            let name = service::Entity::find_by_id(mount.service_id)
                .one(app.db())
                .await?
                .map(|s| s.name)
                .unwrap_or_default();
            return Ok(Some(format!("the service {name}")));
        }
    }
    let Some(docker) = app.service::<crate::services::docker_service::Docker>() else {
        return Ok(None);
    };
    // Originals kept stopped by an import that can still go back to them; one
    // put back by an undo is the user's again, and counts.
    let mut originals: Vec<String> = Vec::new();
    for item in container_import_item::Entity::find().all(app.db()).await? {
        let kept = container_import::Entity::find_by_id(item.import_id)
            .one(app.db())
            .await?
            .is_some_and(|i| i.state != "reverted");
        if kept && item.state != "restored" {
            originals.push(item.container_id);
        }
    }
    let own = match service::Entity::find_by_id(service_id)
        .one(app.db())
        .await?
    {
        Some(svc) => project::Entity::find_by_id(svc.project_id)
            .one(app.db())
            .await?
            .map(|p| crate::services::deploy_service::service_name(&p.name, &svc.name)),
        None => None,
    };
    let listed = docker
        .engine()
        .containers_detailed()
        .await
        .map_err(|e| AppError::BadRequest(e.to_string()))?;
    for c in listed {
        let id = c["Id"].as_str().unwrap_or_default();
        let name = c["Names"][0]
            .as_str()
            .unwrap_or_default()
            .trim_start_matches('/');
        let task_of = c["Labels"]["com.docker.swarm.service.name"].as_str();
        if originals.iter().any(|o| o == id)
            || name.starts_with("rdp-import-")
            || (own.is_some() && task_of == own.as_deref())
        {
            continue;
        }
        let uses = c["Mounts"].as_array().is_some_and(|mounts| {
            mounts.iter().any(|m| {
                m["Type"] == "bind" && m["Source"].as_str().is_some_and(|s| overlaps(s, folder))
            })
        });
        if uses {
            return Ok(Some(format!("the container {name}")));
        }
    }
    Ok(None)
}

/// Who writes from a step of a move: its job, for the admin who started it.
fn job_context(cx: &Cx<'_>) -> AuditContext {
    AuditContext::new(
        AuditActor::job("import.step", cx.import.created_by),
        RequestMeta::default(),
    )
}

/// The folders of `service_id` used in place.
async fn link_targets(app: &AppState, service_id: i64) -> AppResult<Vec<String>> {
    Ok(service_mount::Entity::find()
        .filter(service_mount::Column::ServiceId.eq(service_id))
        .filter(service_mount::Column::Kind.eq("link"))
        .all(app.db())
        .await?
        .into_iter()
        .filter_map(|m| m.link_target)
        .collect())
}

/// Every host folder `service_id` mounts: its folders in place and its binds.
async fn host_folders<C: sea_orm::ConnectionTrait>(
    db: &C,
    service_id: Option<i64>,
) -> AppResult<Vec<(i64, String)>> {
    let mut query =
        service_mount::Entity::find().filter(service_mount::Column::Kind.is_in(["bind", "link"]));
    if let Some(id) = service_id {
        query = query.filter(service_mount::Column::ServiceId.eq(id));
    }
    Ok(query
        .all(db)
        .await?
        .into_iter()
        .filter_map(|m| {
            let path = if m.kind == "link" {
                m.link_target
            } else {
                m.source
            };
            path.map(|p| (m.service_id, p))
        })
        .collect())
}

/// Moves a service's folders into volumes: the import row, its item and its
/// first step, in one write.
pub async fn start(
    app: &AppState,
    principal: &Principal,
    request: RequestMeta,
    service_id: i64,
    confirm: &str,
) -> AppResult<i64> {
    let svc = service::Entity::find_by_id(service_id)
        .one(app.db())
        .await?
        .ok_or(AppError::NotFound)?;
    if confirm != svc.name {
        return Err(
            ValidationErrors::field("confirm", "type the service's name to confirm").into(),
        );
    }
    // A folder another container uses would be split in two by a copy; one
    // the panel may not touch is never read. Docker is read before the write.
    for folder in link_targets(app, svc.id).await? {
        if forbidden_path(
            std::path::Path::new(&folder),
            &crate::services::deploy_rules_service::config().state_dir,
        ) {
            return Err(ValidationErrors::field(
                "service",
                "it uses a folder the panel may not touch",
            )
            .into());
        }
        if let Some(who) = folder_used_elsewhere(app, &folder, svc.id).await? {
            tracing::info!(folder, who, "a folder to move is used elsewhere");
            return Err(ValidationErrors::field(
                "service",
                "a folder it uses is used by another container too: a copy would split it",
            )
            .into());
        }
    }
    let txn = rustcms::db::begin_write(app.db()).await?;
    let svc = service::Entity::find_by_id(service_id)
        .lock_exclusive()
        .one(&txn)
        .await?
        .ok_or(AppError::NotFound)?;
    // Nothing else holds it, and it has a folder to move.
    crate::services::import_service::refuse_while_importing(&txn, svc.id, None).await?;
    let links = service_mount::Entity::find()
        .filter(service_mount::Column::ServiceId.eq(svc.id))
        .filter(service_mount::Column::Kind.eq("link"))
        .count(&txn)
        .await?;
    if links == 0 {
        return Err(ValidationErrors::field("service", "it has no folder in use in place").into());
    }
    let project = project::Entity::find_by_id(svc.project_id)
        .one(&txn)
        .await?
        .ok_or(AppError::NotFound)?;
    let now = Utc::now();
    let row = container_import::ActiveModel {
        project_id: Set(Some(svc.project_id)),
        project_name: Set(project.name.clone()),
        source_kind: Set("move".to_owned()),
        source_ref: Set(svc.id.to_string()),
        state: Set("queued".to_owned()),
        auto_undo: Set(true),
        undo_requested: Set(false),
        created_by: Set(crate::services::access_service::user_of(principal)),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(&txn)
    .await?;
    container_import_item::ActiveModel {
        import_id: Set(row.id),
        service_id: Set(Some(svc.id)),
        container_id: Set(format!("move:{}", svc.id)),
        container_name: Set(svc.name.clone()),
        level: Set(0),
        choice: Set("bring".to_owned()),
        was_running: Set(svc.desired_state != "stopped"),
        state: Set("pending".to_owned()),
        ..Default::default()
    }
    .insert(&txn)
    .await?;
    let entry = AuditEntry::new("imports.start", "container_imports")
        .key(row.id.to_string())
        .meta(json!({ "move": svc.name }));
    audit::record(
        &txn,
        &AuditContext::new(AuditActor::user(principal), request),
        entry,
    )
    .await?;
    rustcms::queue::dispatch(
        &txn,
        &crate::jobs::import_step::ImportStep { import_id: row.id },
    )
    .await?;
    txn.commit().await?;
    Ok(row.id)
}

/// The latest move of `service_id` that is `active` or waits for the admin.
async fn current_move(app: &AppState, service_id: i64) -> AppResult<container_import::Model> {
    container_import::Entity::find()
        .filter(container_import::Column::SourceKind.eq("move"))
        .filter(container_import::Column::SourceRef.eq(service_id.to_string()))
        .filter(
            container_import::Column::State.is_in(
                crate::services::import_service::RUNNING
                    .iter()
                    .copied()
                    .chain(["active", "needs_attention"]),
            ),
        )
        .order_by_desc(container_import::Column::Id)
        .one(app.db())
        .await?
        .ok_or(AppError::Conflict {
            code: "no_move",
            reason: "state",
            message: "This service has no move to undo",
        })
}

/// Undoes the last move of a service (the folder is used in place again).
pub async fn undo(
    app: &AppState,
    principal: &Principal,
    request: RequestMeta,
    service_id: i64,
) -> AppResult<i64> {
    let current = current_move(app, service_id).await?;
    crate::services::import_run_service::ask_undo(app, principal, request, current.id).await?;
    Ok(current.id)
}

/// A short name for a link: the last part of its symlink.
fn short(mount: &service_mount::Model) -> String {
    mount.source.as_deref().map(last).unwrap_or_default()
}

pub(crate) async fn step(cx: &Cx<'_>) -> AppResult<()> {
    let state = cx.import.state.clone();
    let result = match state.as_str() {
        "queued" | "preparing" => Ok(Some("stopping")),
        "stopping" => stopping(cx).await,
        "copying" => copying(cx).await,
        "deploying" => deploying(cx).await,
        "observing" => observing(cx).await,
        "undoing" => undoing(cx).await,
        _ => return Ok(()),
    };
    match result {
        Ok(Some(next)) => go(cx, next, None).await,
        Ok(None) => Ok(()),
        Err(why) => {
            // Before the mounts are switched the service still uses the folder:
            // started again, the move is undone by itself. After, it waits.
            let switched = matches!(state.as_str(), "deploying" | "observing");
            let undoing = state == "undoing";
            if cx.import.auto_undo && !switched && !undoing {
                go(cx, "undoing", Some(&why)).await
            } else {
                attention(cx, &state, &why).await
            }
        }
    }
}

async fn item_service(cx: &Cx<'_>) -> Result<service::Model, Failure> {
    let item = cx.items.first().ok_or("the move has no service")?;
    cx.service_of(item)
        .await
        .ok_or_else(|| "the service is gone".to_owned())
}

async fn stopping(cx: &Cx<'_>) -> Result<Option<&'static str>, Failure> {
    let svc = item_service(cx).await?;
    if cx.items[0].was_running {
        deploy_service(cx, &svc, Trigger::Stop).await?;
    }
    Ok(Some("copying"))
}

async fn copying(cx: &Cx<'_>) -> Result<Option<&'static str>, Failure> {
    let svc = item_service(cx).await?;
    let project = project::Entity::find_by_id(svc.project_id)
        .one(cx.db())
        .await
        .map_err(|e| e.to_string())?
        .ok_or("the project is gone")?;
    let mounts = service_mount::Entity::find()
        .filter(service_mount::Column::ServiceId.eq(svc.id))
        .filter(service_mount::Column::Kind.eq("link"))
        .all(cx.db())
        .await
        .map_err(|e| e.to_string())?;
    let engine = cx.docker.engine();
    let state_dir = &crate::services::deploy_rules_service::config().state_dir;
    let mut copies = Vec::new();
    for mount in &mounts {
        cx.alive().await?;
        let folder = mount.link_target.clone().ok_or("a link has no folder")?;
        if forbidden_path(std::path::Path::new(&folder), state_dir) {
            return Err(format!("{folder} is a place the panel may not touch"));
        }
        // Checked again now that the service is stopped: nothing else may
        // write it while it is copied.
        if let Some(who) = folder_used_elsewhere(cx.app, &folder, svc.id)
            .await
            .map_err(|e| e.to_string())?
        {
            return Err(format!("{folder} is used by {who} too"));
        }
        let name = short(mount);
        let target = volume_name(&project.name, &svc.name, &name);
        let existing = engine.volumes().await.map_err(|e| e.to_string())?;
        if let Some(found) = existing.iter().find(|v| v.name == target)
            && found.labels.get("rdockpanel.import") != Some(&cx.import.id.to_string())
        {
            return Err(format!(
                "a volume named {target} exists and is not this move's"
            ));
        }
        // Recorded before the volume is made: an undo finds it and removes it.
        let recorded = container_import_volume::Entity::find()
            .filter(container_import_volume::Column::ImportId.eq(cx.import.id))
            .filter(container_import_volume::Column::PanelVolume.eq(target.clone()))
            .one(cx.db())
            .await
            .map_err(|e| e.to_string())?;
        let row = match recorded {
            Some(row) => row,
            None => {
                let txn = rustcms::db::begin_write(cx.db())
                    .await
                    .map_err(|e| e.to_string())?;
                let row = container_import_volume::ActiveModel {
                    import_id: Set(Some(cx.import.id)),
                    original_name: Set(folder.clone()),
                    mode: Set("copy".to_owned()),
                    panel_volume: Set(Some(target.clone())),
                    ..Default::default()
                }
                .insert(&txn)
                .await
                .map_err(|e| e.to_string())?;
                let entry = AuditEntry::new("imports.volume_record", "container_imports")
                    .key(cx.import.id.to_string())
                    .meta(json!({ "volume": target }));
                audit::record(&txn, &job_context(cx), entry)
                    .await
                    .map_err(|e| e.to_string())?;
                txn.commit().await.map_err(|e| e.to_string())?;
                row
            }
        };
        if !existing.iter().any(|v| v.name == target) {
            let labels = std::collections::HashMap::from([
                (
                    crate::services::docker_service::MANAGED_LABEL.to_owned(),
                    "true".to_owned(),
                ),
                ("rdockpanel.project".to_owned(), project.name.clone()),
                ("rdockpanel.service".to_owned(), svc.name.clone()),
                ("rdockpanel.import".to_owned(), cx.import.id.to_string()),
            ]);
            engine
                .create_volume_labelled(&target, labels)
                .await
                .map_err(|e| e.to_string())?;
        }
        let both = || {
            vec![
                HelperMount {
                    source: folder.clone(),
                    target: "/from".to_owned(),
                    read_only: true,
                    kind: "bind".to_owned(),
                },
                HelperMount {
                    source: target.clone(),
                    target: "/to".to_owned(),
                    read_only: false,
                    kind: "volume".to_owned(),
                },
            ]
        };
        // Room for the copy and a margin (10%, at least 2 GiB): a full disk
        // stops the other services and the panel's own database.
        let room = cx
            .helper(
                &format!("room-{}", mount.id),
                vec![
                    "sh",
                    "-c",
                    "need=$(du -sk /from | cut -f1); margin=$((need / 10)); [ \"$margin\" -lt 2097152 ] && margin=2097152; [ \"$(df -Pk /to | awk 'NR==2{print $4}')\" -gt $((need + margin)) ] || exit 6",
                ],
                both(),
                &["DAC_READ_SEARCH"],
            )
            .await?;
        if room != 0 {
            return Err(format!(
                "not enough room to copy {folder} (its size and a margin)"
            ));
        }
        let code = cx
            .helper(
                &format!("move-{}", mount.id),
                vec!["cp", "-a", "/from/.", "/to/"],
                both(),
                &["CHOWN", "DAC_OVERRIDE", "FOWNER"],
            )
            .await?;
        if code != 0 {
            return Err(format!("copying {folder} failed (exit {code})"));
        }
        copies.push((mount.clone(), name, row));
    }
    // Switched in one write: the mounts, the record of what was copied, the
    // audit entry and the next step; only while the lease is still this run's
    // (another may have taken the move over, and undone it).
    let txn = rustcms::db::begin_write(cx.db())
        .await
        .map_err(|e| e.to_string())?;
    let ours = container_import::Entity::find_by_id(cx.import.id)
        .lock_exclusive()
        .one(&txn)
        .await
        .map_err(|e| e.to_string())?
        .is_some_and(|i| i.lease_token.as_deref() == Some(cx.token.as_str()));
    if !ours {
        return Err("the import's lease was lost".to_owned());
    }
    for (mount, name, row) in copies {
        let mut change = mount.into_active_model();
        change.kind = Set("volume".to_owned());
        change.name = Set(Some(name));
        change.source = Set(None);
        change.link_target = Set(None);
        change.update(&txn).await.map_err(|e| e.to_string())?;
        let mut copied = row.into_active_model();
        copied.copied_at = Set(Some(Utc::now()));
        copied.update(&txn).await.map_err(|e| e.to_string())?;
    }
    // Only its own reasons keep a service its administrators' (a local
    // image, a user of its own): the folder is no longer one.
    let mut changed = svc.clone().into_active_model();
    changed.admin_only = Set(crate::models::service::needs_administrator(&svc));
    changed.update(&txn).await.map_err(|e| e.to_string())?;
    let entry = AuditEntry::new("services.move_data", "services")
        .key(svc.id.to_string())
        .meta(json!({ "import": cx.import.id }));
    audit::record(&txn, &job_context(cx), entry)
        .await
        .map_err(|e| e.to_string())?;
    txn.commit().await.map_err(|e| e.to_string())?;
    Ok(Some("deploying"))
}

async fn deploying(cx: &Cx<'_>) -> Result<Option<&'static str>, Failure> {
    let svc = item_service(cx).await?;
    // The service was stopped for the copy: a deployment would keep it stopped.
    // Start applies its configuration as it is now (the volume, or the link).
    let trigger = if cx.items[0].was_running {
        Trigger::Start
    } else {
        Trigger::Deploy
    };
    deploy_service(cx, &svc, trigger).await?;
    Ok(Some("observing"))
}

async fn observing(cx: &Cx<'_>) -> Result<Option<&'static str>, Failure> {
    let svc = item_service(cx).await?;
    let project = project::Entity::find_by_id(svc.project_id)
        .one(cx.db())
        .await
        .map_err(|e| e.to_string())?
        .ok_or("the project is gone")?;
    let until = std::time::Instant::now() + cx.settings.observe;
    let started = Utc::now();
    loop {
        cx.alive().await?;
        let tasks = cx
            .docker
            .engine()
            .list_tasks(&crate::services::deploy_service::service_name(
                &project.name,
                &svc.name,
            ))
            .await
            .map_err(|e| e.to_string())?;
        if tasks
            .iter()
            .any(|t| crate::services::import_run_service::failed_since(t, started))
        {
            return Err(format!("{} has failed tasks", svc.name));
        }
        if std::time::Instant::now() >= until {
            // The way back ends in `reverted`, the way forward in `active`.
            return Ok(Some(if cx.import.has_flag("back") {
                "reverted"
            } else {
                "active"
            }));
        }
        tokio::time::sleep(cx.settings.poll).await;
    }
}

async fn undoing(cx: &Cx<'_>) -> Result<Option<&'static str>, Failure> {
    // The mounts are the links again already (this step ran, and its way back
    // failed after): only the deployment is left. Never guess from the mounts.
    if cx.import.has_flag("back") {
        return Ok(Some("deploying"));
    }
    let svc = item_service(cx).await?;
    let project = project::Entity::find_by_id(svc.project_id)
        .one(cx.db())
        .await
        .map_err(|e| e.to_string())?
        .ok_or("the project is gone")?;
    let volumes = container_import_volume::Entity::find()
        .filter(container_import_volume::Column::ImportId.eq(cx.import.id))
        .all(cx.db())
        .await
        .map_err(|e| e.to_string())?;
    let links = settings(cx.app).links();
    let mounts = service_mount::Entity::find()
        .filter(service_mount::Column::ServiceId.eq(svc.id))
        .all(cx.db())
        .await
        .map_err(|e| e.to_string())?;
    let switched: Vec<_> = mounts
        .iter()
        .filter_map(|m| {
            let name = m.name.clone()?;
            let target = volume_name(&project.name, &svc.name, &name);
            volumes
                .iter()
                .find(|v| v.panel_volume.as_deref() == Some(target.as_str()))
                .map(|v| (m.clone(), name, v.original_name.clone()))
        })
        .filter(|(m, _, _)| m.kind == "volume")
        .collect();
    if switched.is_empty() {
        // Never switched: the service starts again on its folder; the empty
        // volumes this move made are removed.
        if cx.items[0].was_running {
            deploy_service(cx, &svc, Trigger::Start).await?;
        }
        for volume in &volumes {
            if let Some(name) = &volume.panel_volume {
                let _ = cx.docker.engine().remove_volume(name).await;
            }
        }
        return Ok(Some("reverted"));
    }
    // Switched: the link is the mount again, in one write with its audit entry.
    let txn = rustcms::db::begin_write(cx.db())
        .await
        .map_err(|e| e.to_string())?;
    for (mount, name, folder) in switched {
        let mut change = mount.into_active_model();
        change.kind = Set("link".to_owned());
        change.name = Set(None);
        change.source = Set(Some(link_path(&links, &project.name, &svc.name, &name)));
        change.link_target = Set(Some(folder));
        change.update(&txn).await.map_err(|e| e.to_string())?;
    }
    let mut changed = svc.clone().into_active_model();
    changed.admin_only = Set(true);
    changed.update(&txn).await.map_err(|e| e.to_string())?;
    let mut mark = container_import::Entity::find_by_id(cx.import.id)
        .lock_exclusive()
        .one(&txn)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("the import is gone")?
        .into_active_model();
    mark.flags = Set(Some(container_import::with_flag(
        mark.flags.clone().take().flatten().as_deref(),
        "back",
    )));
    mark.update(&txn).await.map_err(|e| e.to_string())?;
    let entry = AuditEntry::new("services.move_data_undo", "services")
        .key(svc.id.to_string())
        .meta(json!({ "import": cx.import.id }));
    audit::record(
        &txn,
        &AuditContext::new(
            AuditActor::job("import.step", cx.import.created_by),
            RequestMeta::default(),
        ),
        entry,
    )
    .await
    .map_err(|e| e.to_string())?;
    txn.commit().await.map_err(|e| e.to_string())?;
    Ok(Some("deploying"))
}

/// Deletes the old folder of a service whose data moved to a volume: its
/// name typed, the rules checked again and the record compared. The volume is
/// marked `cleaning` with the audit entry first (from then the move cannot be
/// undone, and a clean that failed can run again); then one helper, with the
/// parent mounted, refuses a symlink and empties the folder from inside it
/// (`find -delete` never follows a link), and removes it.
pub async fn clean_old_folder(
    app: &AppState,
    principal: &Principal,
    request: RequestMeta,
    service_id: i64,
    folder: &str,
    confirm: &str,
) -> AppResult<()> {
    let bad = |field: &str, why: &str| -> AppError { ValidationErrors::field(field, why).into() };
    let svc = service::Entity::find_by_id(service_id)
        .one(app.db())
        .await?
        .ok_or(AppError::NotFound)?;
    // The record: a move of this service whose folder this was, not released.
    let moves = container_import::Entity::find()
        .filter(container_import::Column::SourceKind.eq("move"))
        .filter(container_import::Column::SourceRef.eq(service_id.to_string()))
        .filter(container_import::Column::State.is_in(["active", "finished"]))
        .all(app.db())
        .await?;
    let mut found = None;
    for import in &moves {
        let volume = container_import_volume::Entity::find()
            .filter(container_import_volume::Column::ImportId.eq(import.id))
            .filter(container_import_volume::Column::OriginalName.eq(folder))
            .filter(container_import_volume::Column::CopiedAt.is_not_null())
            .filter(container_import_volume::Column::ReleasedAt.is_null())
            .one(app.db())
            .await?;
        if let Some(volume) = volume {
            found = Some(volume);
        }
    }
    let Some(volume) = found else {
        return Err(bad(
            "folder",
            "no folder of this service is waiting to be cleaned",
        ));
    };
    if confirm != last(folder) {
        return Err(bad("confirm", "type the folder's name to confirm"));
    }
    // The rules again, as when it was recorded.
    let path = std::path::Path::new(folder);
    let home = std::env::var("HOME").unwrap_or_default();
    if !path.is_absolute()
        || forbidden_path(
            path,
            &crate::services::deploy_rules_service::config().state_dir,
        )
        || path.components().any(|c| c.as_os_str() == "..")
        || (!home.is_empty() && path == std::path::Path::new(&home))
        || path.parent().is_none_or(|p| p == std::path::Path::new("/"))
    {
        return Err(bad("folder", "this folder may not be deleted from here"));
    }
    // Nor this service itself (a move undone puts the link back on it, or a
    // bind was added since).
    if host_folders(app.db(), Some(service_id))
        .await?
        .iter()
        .any(|(_, t)| overlaps(t, folder))
    {
        return Err(bad(
            "folder",
            "the service uses this folder again: move it first",
        ));
    }
    // Nor an import that can still be undone: its original runs on that folder.
    let held = container_import_volume::Entity::find()
        .filter(container_import_volume::Column::Mode.eq("link"))
        .all(app.db())
        .await?;
    for row in held.iter().filter(|r| overlaps(&r.original_name, folder)) {
        let open = match row.import_id {
            Some(id) => container_import::Entity::find_by_id(id)
                .one(app.db())
                .await?
                .is_some_and(|i| !matches!(i.state.as_str(), "finished" | "reverted")),
            None => false,
        };
        if open {
            return Err(bad(
                "folder",
                "the import that brought this service in can still be undone: finish it first",
            ));
        }
    }
    // Nothing else uses it, or a folder inside it (a service, a container).
    if folder_used_elsewhere(app, folder, service_id)
        .await?
        .is_some()
    {
        return Err(bad(
            "folder",
            "another service or container uses this folder",
        ));
    }
    let docker = app
        .service::<crate::services::docker_service::Docker>()
        .ok_or_else(|| AppError::internal("Docker is not configured"))?;
    let image = crate::services::import_service::helper_image(app, &docker)
        .await
        .ok_or_else(|| {
            AppError::BadRequest("no helper image: set `[panel] helper_image`".into())
        })?;
    let import_id = volume.import_id.unwrap_or_default();
    let context = AuditContext::new(AuditActor::user(principal), request);
    // Marked and audited before it happens, under the move's lock: an undo
    // waits for it, or went first (then the move is no longer active).
    let txn = rustcms::db::begin_write(app.db()).await?;
    let open = container_import::Entity::find_by_id(import_id)
        .lock_exclusive()
        .one(&txn)
        .await?
        .is_some_and(|i| matches!(i.state.as_str(), "active" | "finished"));
    let row = container_import_volume::Entity::find_by_id(volume.id)
        .lock_exclusive()
        .one(&txn)
        .await?
        .filter(|v| v.released_at.is_none());
    let (true, Some(row)) = (open, row) else {
        return Err(bad(
            "folder",
            "no folder of this service is waiting to be cleaned",
        ));
    };
    // The services again, in the write: one that took the folder since the
    // first look is seen (a container outside the panel cannot be held).
    if host_folders(&txn, None)
        .await?
        .iter()
        .any(|(_, t)| overlaps(t, folder))
    {
        return Err(bad(
            "folder",
            "another service or container uses this folder",
        ));
    }
    // Run before (and maybe done before a crash): gone is done.
    let again = if row.cleaning_at.is_some() {
        "again"
    } else {
        "first"
    };
    if row.cleaning_at.is_none() {
        let mut mark = row.clone().into_active_model();
        mark.cleaning_at = Set(Some(Utc::now()));
        mark.update(&txn).await?;
    }
    let entry = AuditEntry::new("services.clean_old_folder", "services")
        .key(svc.id.to_string())
        .meta(json!({ "folder": folder }));
    audit::record(&txn, &context, entry).await?;
    txn.commit().await?;
    let name = last(folder);
    let parent = path
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let run = |label: &str, cmd: Vec<String>, mounts: Vec<HelperMount>| {
        let docker = docker.clone();
        let image = image.clone();
        let spec = crate::services::docker_service::HelperSpec {
            name: format!("rdp-import-{import_id}-{label}"),
            image,
            cmd,
            mounts,
            import_id,
            cap_add: vec!["DAC_OVERRIDE".to_owned(), "FOWNER".to_owned()],
            timeout_s: crate::services::import_service::HELPER_TIMEOUT_S,
        };
        async move {
            let _ = docker.engine().remove_container(&spec.name, true).await;
            docker
                .engine()
                .run_helper(spec)
                .await
                .map_err(|e| AppError::BadRequest(e.to_string()))
        }
    };
    let cmd = |words: &[&str]| words.iter().map(|w| (*w).to_owned()).collect::<Vec<_>>();
    // The folder is reached from its parent: refused if it is a symlink, or if
    // the place the shell is in once inside is not it (swapped meanwhile).
    // The name is an argument (`$1`), never part of the script.
    let script = r#"f="/p/$1"
if [ "$2" = again ] && [ ! -e "$f" ] && [ ! -L "$f" ]; then exit 0; fi
[ -d "$f" ] && [ ! -L "$f" ] || exit 4
cd "$f" || exit 4
[ "$(pwd -P)" = "$f" ] || exit 4
find . -mindepth 1 -delete || exit 1
cd / && rmdir "$f""#;
    let code = run(
        "clean",
        cmd(&["sh", "-c", script, "sh", &name, again]),
        vec![HelperMount {
            source: parent,
            target: "/p".to_owned(),
            read_only: false,
            kind: "bind".to_owned(),
        }],
    )
    .await?;
    match code {
        0 => {}
        4 => {
            return Err(AppError::BadRequest(
                "the folder is not a plain folder any more (a symlink?): look at it".into(),
            ));
        }
        _ => {
            return Err(AppError::BadRequest(
                "the folder could not be removed completely: look at it, then clean again".into(),
            ));
        }
    }
    // Its symlink in the panel's links folder, by the name the move gave it.
    let project = project::Entity::find_by_id(svc.project_id)
        .one(app.db())
        .await?
        .ok_or(AppError::NotFound)?;
    let links = settings(app).links();
    let link_name = service_mount::Entity::find()
        .filter(service_mount::Column::ServiceId.eq(svc.id))
        .filter(service_mount::Column::Kind.eq("volume"))
        .all(app.db())
        .await?
        .into_iter()
        .filter_map(|m| m.name)
        .find(|n| Some(volume_name(&project.name, &svc.name, n)) == row.panel_volume);
    if let Some(link_name) = link_name {
        let _ = run(
            "clean-link",
            cmd(&[
                "rm",
                "-f",
                &format!("/links/{}_{}/{link_name}", project.name, svc.name),
            ]),
            vec![HelperMount {
                source: links.to_string_lossy().into_owned(),
                target: "/links".to_owned(),
                read_only: false,
                kind: "bind".to_owned(),
            }],
        )
        .await;
    }
    let txn = rustcms::db::begin_write(app.db()).await?;
    let mut done = row.into_active_model();
    done.released_at = Set(Some(Utc::now()));
    done.update(&txn).await?;
    let entry = AuditEntry::new("services.old_folder_cleaned", "services")
        .key(svc.id.to_string())
        .meta(json!({ "folder": folder }));
    audit::record(&txn, &context, entry).await?;
    txn.commit().await?;
    Ok(())
}
