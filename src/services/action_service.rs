//! The actions log (`actions` table): every operation the panel runs on
//! Docker gets a row, and the ones a user asked for are audited too.

use std::future::Future;

use chrono::Utc;
use rustcms::Principal;
use rustcms::audit::{self, AuditActor, AuditContext, AuditEntry, RequestMeta};
use rustcms::{AppError, AppResult};
use sea_orm::{ActiveModelTrait, ConnectionTrait, EntityTrait, IntoActiveModel, Set};
use serde_json::json;

use crate::models::action;

/// Who asked for an action, for the log and the audit trail.
#[derive(Clone, Debug)]
pub struct Initiator {
    pub user_id: Option<i64>,
    pub audit: AuditContext,
}

impl Initiator {
    pub fn user(principal: &Principal, request: RequestMeta) -> Self {
        Self {
            user_id: principal.id.parse().ok(),
            audit: AuditContext::new(AuditActor::user(principal), request),
        }
    }
}

/// Logs `kind` on `target` as running, runs `work`, then records how it
/// ended (and audits it when a user asked). The work's error is returned
/// as is; a failure to write the log is only reported, never hides it.
pub async fn run<C, T, E, F>(
    db: &C,
    by: &Initiator,
    kind: &str,
    target: &str,
    work: F,
) -> AppResult<T>
where
    C: ConnectionTrait,
    E: Into<AppError> + std::fmt::Display,
    F: Future<Output = Result<T, E>>,
{
    let row = start(db, by, kind, target, action::RUNNING).await?;
    let outcome = work.await;
    let error = outcome.as_ref().err().map(ToString::to_string);
    if let Err(err) = finish(db, row, error.clone(), None).await {
        tracing::warn!(%err, kind, target, "could not finish an action row");
    }
    if by.user_id.is_some() {
        let entry = AuditEntry::new(format!("docker.{kind}"), "docker")
            .key(target)
            .meta(json!({ "ok": error.is_none(), "error": error }));
        audit::record_detached(db, &by.audit, entry).await;
    }
    outcome.map_err(Into::into)
}

/// For work that may stop the panel itself (its restart, the server's
/// reboot): its row (running) and its audit entry, committed before the
/// work starts. A row such work leaves running is closed at the next boot
/// ([`came_back`]).
pub async fn announce(
    db: &sea_orm::DatabaseConnection,
    by: &Initiator,
    kind: &str,
    target: &str,
) -> AppResult<action::Model> {
    use sea_orm::TransactionTrait;
    let txn = db.begin().await?;
    let row = start(&txn, by, kind, target, action::RUNNING).await?;
    let entry = AuditEntry::new(format!("system.{kind}"), "system")
        .key(target)
        .meta(json!({ "action": row.id }));
    audit::record(&txn, &by.audit, entry).await?;
    txn.commit().await?;
    Ok(row)
}

/// The panel's restarts and the server's reboots still running when it
/// starts: done (it came back).
pub async fn came_back<C: ConnectionTrait>(db: &C, kinds: &[&str]) -> AppResult<usize> {
    use sea_orm::{ColumnTrait, QueryFilter};
    let rows = action::Entity::find()
        .filter(action::Column::Kind.is_in(kinds.iter().copied()))
        .filter(action::Column::Status.eq(action::RUNNING))
        .all(db)
        .await?;
    let n = rows.len();
    for row in rows {
        finish(db, row, None, Some("the panel started again")).await?;
    }
    Ok(n)
}

/// A new row in `status` (`queued` for work a job does later).
pub async fn start<C: ConnectionTrait>(
    db: &C,
    by: &Initiator,
    kind: &str,
    target: &str,
    status: &str,
) -> AppResult<action::Model> {
    let now = Utc::now();
    let row = action::ActiveModel {
        kind: Set(kind.to_owned()),
        target: Set(target.chars().take(255).collect()),
        status: Set(status.to_owned()),
        user_id: Set(by.user_id),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    };
    Ok(row.insert(db).await?)
}

/// Replaces `row`'s log (while it runs).
pub async fn note<C: ConnectionTrait>(
    db: &C,
    row: action::Model,
    log: &str,
) -> AppResult<action::Model> {
    let mut active = row.into_active_model();
    active.log = Set(Some(action::tail(log).to_owned()));
    active.updated_at = Set(Utc::now());
    Ok(active.update(db).await?)
}

/// Terminals still running (none can, at boot: the panel stopped before
/// they ended): marked failed; their containers and session marks, for
/// ending their shells.
pub async fn interrupted_terminals<C: ConnectionTrait>(
    db: &C,
    prefix: &str,
) -> AppResult<Vec<(String, String)>> {
    use sea_orm::{ColumnTrait, QueryFilter};
    let rows = action::Entity::find()
        .filter(action::Column::Kind.eq("container.exec"))
        .filter(action::Column::Status.eq(action::RUNNING))
        .all(db)
        .await?;
    let mut ended = Vec::new();
    for row in rows {
        let token = row
            .log
            .as_deref()
            .and_then(|log| log.strip_prefix(prefix))
            .map(str::to_owned);
        let target = row.target.clone();
        finish(
            db,
            row,
            Some("interrupted: the panel stopped before it ended".to_owned()),
            None,
        )
        .await?;
        if let Some(token) = token {
            ended.push((target, token));
        }
    }
    Ok(ended)
}

/// Script runs left running (their run lasts seconds: older than `after`,
/// the request that ran them is gone): each with its container and mark,
/// for killing them before they are finished.
pub async fn abandoned_scripts<C: ConnectionTrait>(
    db: &C,
    kind: &str,
    prefix: &str,
    after: chrono::Duration,
) -> AppResult<Vec<(action::Model, Option<(String, String)>)>> {
    use sea_orm::{ColumnTrait, QueryFilter};
    let rows = action::Entity::find()
        .filter(action::Column::Kind.eq(kind))
        .filter(action::Column::Status.eq(action::RUNNING))
        .filter(action::Column::UpdatedAt.lt(Utc::now() - after))
        .all(db)
        .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let mark = row
                .log
                .as_deref()
                .and_then(|log| log.strip_prefix(prefix))
                .map(|mark| (row.target.clone(), mark.to_owned()));
            (row, mark)
        })
        .collect())
}

/// Marks the row of `id` running (a job picked it up).
pub async fn running<C: ConnectionTrait>(db: &C, id: i64) -> AppResult<Option<action::Model>> {
    let Some(row) = action::Entity::find_by_id(id).one(db).await? else {
        return Ok(None);
    };
    let mut active = row.into_active_model();
    active.status = Set(action::RUNNING.to_owned());
    active.updated_at = Set(Utc::now());
    Ok(Some(active.update(db).await?))
}

/// Ends `row`: ok unless `error`; `log` replaces the row's log (its tail).
pub async fn finish<C: ConnectionTrait>(
    db: &C,
    row: action::Model,
    error: Option<String>,
    log: Option<&str>,
) -> AppResult<action::Model> {
    let now = Utc::now();
    let mut active = row.into_active_model();
    active.status = Set(if error.is_some() {
        action::FAILED
    } else {
        action::OK
    }
    .to_owned());
    active.error = Set(error.map(|e| action::tail(&e).to_owned()));
    if let Some(log) = log {
        active.log = Set(Some(action::tail(log).to_owned()));
    }
    active.finished_at = Set(Some(now));
    active.updated_at = Set(now);
    Ok(active.update(db).await?)
}

/// After this long without an update, a queued or running action is taken
/// for abandoned (a crashed worker, a timed-out job, a request dropped
/// mid-way); longer than any job's timeout.
pub const STALE_AFTER: chrono::Duration = chrono::Duration::hours(1);

/// Marks abandoned actions failed; the number marked. Runs at boot (and on
/// the scheduler once the framework has one).
pub async fn reap_stale<C: ConnectionTrait>(db: &C) -> AppResult<u64> {
    use sea_orm::sea_query::Expr;
    use sea_orm::{ColumnTrait, QueryFilter};
    let now = Utc::now();
    let result = action::Entity::update_many()
        .col_expr(action::Column::Status, Expr::value(action::FAILED))
        .col_expr(
            action::Column::Error,
            Expr::value("interrupted: the panel stopped before it ended"),
        )
        .col_expr(action::Column::FinishedAt, Expr::value(now))
        .col_expr(action::Column::UpdatedAt, Expr::value(now))
        .filter(action::Column::Status.is_in([action::QUEUED, action::RUNNING]))
        .filter(action::Column::UpdatedAt.lt(now - STALE_AFTER))
        // A script's run ends once its processes are known dead (the
        // reaper's `abandoned_scripts`), never here.
        .filter(action::Column::Kind.ne("script.run"))
        .exec(db)
        .await?;
    Ok(result.rows_affected)
}
