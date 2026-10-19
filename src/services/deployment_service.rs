//! Deployments: asking for one (in the transaction of the user's action) and
//! running it (in the `deploy` queue).
//!
//! **One at a time per service.** Asking takes the service's lock in the
//! same transaction (`current_deployment_id` + `lock_expires_at`): a second
//! request gets 409 at once. The deployment row stores the configuration as
//! it was asked for (later edits never leak into it). Running renews the
//! lock's lease while it works and stops as soon as the lock is no longer its
//! own (fencing, checked before each change to Docker), reads what Docker has
//! now (a run repeated after a crash continues from there), and releases the
//! lock only if it is still its own. A run cut short leaves its lease to
//! expire ([`LEASE`]); [`recover_interrupted`] (at boot, then every minute)
//! marks such deployments `interrupted`.

use std::time::Duration;

use chrono::{DateTime, Utc};
use rustcms::{AppError, AppResult, AppState};
use sea_orm::sea_query::{Condition, Expr};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait,
    IntoActiveModel, QueryFilter, QuerySelect, Set, TransactionTrait,
};
use serde_json::json;

use std::collections::{BTreeMap, HashMap};

use rustcms::crypto::{Encrypted, Keyring, Unreadable};

use crate::models::{deployment, project, service, service_domain, service_mount, service_port};
use crate::services::deploy_rules_service::{bind_source, config};
use crate::services::deploy_service::{
    ConfigRef, DeployOptions, PinnedImage, Runtime, Snapshot, TEMPLATE_LABEL, desired,
    network_name, pin_image, pulled_digest,
};
use crate::services::docker_service::{
    Docker, DockerError, MANAGED_LABEL, NetworkRequest, RegistryAuth, SwarmService, SwarmTask,
    TaskState,
};
use crate::services::placeholders;
use crate::services::traefik_service::{self, Traefik};

/// How long a deployment holds its service without renewing: a run renews it
/// every few seconds, so only a run that died (or hangs on Docker) loses it.
pub const LEASE: Duration = Duration::from_secs(5 * 60);

/// How long a queued deployment holds its service before a worker starts it
/// (workers may be busy with other deployments meanwhile).
pub const QUEUED_LEASE: Duration = Duration::from_secs(60 * 60);

/// What a deployment does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    Deploy,
    Start,
    Stop,
    Restart,
    Destroy,
    Rollback,
}

impl Trigger {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Deploy => "deploy",
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Restart => "restart",
            Self::Destroy => "destroy",
            Self::Rollback => "rollback",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        [
            Self::Deploy,
            Self::Start,
            Self::Stop,
            Self::Restart,
            Self::Destroy,
            Self::Rollback,
        ]
        .into_iter()
        .find(|t| t.as_str() == text)
    }
}

pub const QUEUED: &str = "queued";
pub const RUNNING: &str = "running";
pub const SUCCEEDED: &str = "succeeded";
pub const FAILED: &str = "failed";
pub const ROLLED_BACK: &str = "rolled_back";
pub const INTERRUPTED: &str = "interrupted";

fn busy() -> AppError {
    AppError::Conflict {
        code: "deployment_in_progress",
        reason: "busy",
        message: "A deployment of this service is already in progress",
    }
}

/// The key of environment digests ([`Snapshot::env_digest`]), from `APP_KEY`.
fn env_digest_key(keyring: &Keyring) -> [u8; 32] {
    keyring.current().derive("rdockpanel.env-digest")
}

/// A service's environment, opened (empty when it has none).
pub fn service_env(
    service: &service::Model,
    keyring: &Keyring,
) -> Result<BTreeMap<String, String>, Unreadable> {
    match &service.env {
        Some(sealed) => sealed.open(keyring, &service::env_context()),
        None => Ok(BTreeMap::new()),
    }
}

fn unreadable_env() -> AppError {
    AppError::BadRequest(
        "the service's environment cannot be read (sealed with another APP_KEY): set its values again"
            .into(),
    )
}

/// The service's configuration as it is now (rows read in `db`), its
/// environment opened with `keyring` (a digest only: never its values).
pub async fn snapshot_of<C: ConnectionTrait>(
    db: &C,
    service: &service::Model,
    keyring: &Keyring,
) -> AppResult<Snapshot> {
    let snapshot = snapshot_without_env(db, service).await?;
    let env = service_env(service, keyring).map_err(|_| unreadable_env())?;
    let files = service_files(db, service, keyring).await?;
    let key = env_digest_key(keyring);
    Ok(snapshot.with_env(&env, &key).with_files(&files, &key))
}

/// A service's file mounts' contents by path, opened.
pub async fn service_files<C: ConnectionTrait>(
    db: &C,
    service: &service::Model,
    keyring: &Keyring,
) -> AppResult<BTreeMap<String, String>> {
    let mounts = service_mount::Entity::find()
        .filter(service_mount::Column::ServiceId.eq(service.id))
        .filter(service_mount::Column::Kind.eq("file"))
        .all(db)
        .await?;
    let mut files = BTreeMap::new();
    for mount in mounts {
        let Some(sealed) = &mount.sealed_content else {
            continue;
        };
        let content = sealed
            .open(keyring, &service_mount::content_context())
            .map_err(|_| {
                AppError::BadRequest(format!(
                    "the file {} cannot be read (sealed with another APP_KEY): set it again",
                    mount.target
                ))
            })?;
        files.insert(mount.target, content);
    }
    Ok(files)
}

/// [`snapshot_of`] without the environment (names, rules: no key needed).
pub async fn snapshot_without_env<C: ConnectionTrait>(
    db: &C,
    service: &service::Model,
) -> AppResult<Snapshot> {
    let project = project::Entity::find_by_id(service.project_id)
        .one(db)
        .await?
        .ok_or(AppError::NotFound)?;
    let domains = service_domain::Entity::find()
        .filter(service_domain::Column::ServiceId.eq(service.id))
        .all(db)
        .await?;
    let ports = service_port::Entity::find()
        .filter(service_port::Column::ServiceId.eq(service.id))
        .all(db)
        .await?;
    let mounts = service_mount::Entity::find()
        .filter(service_mount::Column::ServiceId.eq(service.id))
        .all(db)
        .await?;
    use sea_orm::QueryOrder;
    let redirects = crate::models::service_redirect::Entity::find()
        .filter(crate::models::service_redirect::Column::ServiceId.eq(service.id))
        .order_by_asc(crate::models::service_redirect::Column::Id)
        .all(db)
        .await?;
    let users = crate::models::service_basic_auth::Entity::find()
        .filter(crate::models::service_basic_auth::Column::ServiceId.eq(service.id))
        .all(db)
        .await?;
    Ok(Snapshot::of(&project, service, &domains, &ports, &mounts).with_access(&redirects, &users))
}

/// Whether the configuration changed since the last successful deployment.
/// An environment that cannot be opened counts as a change.
pub async fn has_pending_changes<C: ConnectionTrait>(
    db: &C,
    service: &service::Model,
    keyring: &Keyring,
) -> AppResult<bool> {
    let current = match snapshot_of(db, service, keyring).await {
        Ok(current) => current,
        Err(AppError::BadRequest(_)) => return Ok(true),
        Err(err) => return Err(err),
    };
    let Some(deployed_hash) = service.deployed_spec_hash.as_deref() else {
        return Ok(true);
    };
    if deployed_hash == current.hash() {
        return Ok(false);
    }
    // The digests may only differ by their key (APP_KEY rotated since): the
    // deployed environment, digested with today's key, tells.
    use sea_orm::QueryOrder;
    let Some(deployed) = deployment::Entity::find()
        .filter(deployment::Column::ServiceId.eq(service.id))
        .filter(deployment::Column::SpecHash.eq(deployed_hash))
        .filter(deployment::Column::Status.eq(SUCCEEDED))
        .order_by_desc(deployment::Column::Id)
        .one(db)
        .await?
    else {
        return Ok(true);
    };
    // A snapshot from before `primary_domain` was recorded: as today's.
    let legacy = deployed.spec.get("primary_domain").is_none();
    let Ok(mut then) = serde_json::from_value::<Snapshot>(deployed.spec) else {
        return Ok(true);
    };
    if legacy {
        then.primary_domain = current.primary_domain.clone();
    }
    let env_then = match &deployed.env {
        Some(sealed) => match sealed.open(keyring, &deployment::env_context()) {
            Ok(env) => env,
            Err(_) => return Ok(true),
        },
        None => BTreeMap::new(),
    };
    let files_then = match &deployed.files {
        Some(sealed) => match sealed.open(keyring, &deployment::files_context()) {
            Ok(files) => files,
            Err(_) => return Ok(true),
        },
        None => BTreeMap::new(),
    };
    let key = env_digest_key(keyring);
    Ok(then
        .with_env(&env_then, &key)
        .with_files(&files_then, &key)
        .hash()
        != current.hash())
}

/// Asks for a deployment of `service`, in `txn` (the caller queues
/// [`crate::jobs::run_deployment::RunDeployment`] in it): takes the
/// service's lock (409 when another deployment holds it) and records what
/// to apply. `rollback_to` is the earlier deployment a rollback re-applies.
pub async fn request<C: ConnectionTrait>(
    txn: &C,
    service: &service::Model,
    trigger: Trigger,
    rollback_to: Option<&deployment::Model>,
    user_id: Option<i64>,
    keyring: &Keyring,
) -> AppResult<deployment::Model> {
    let now = Utc::now();
    // The service and its project as they are now, locked until the
    // transaction ends: a rename waits for it, or went first (then the
    // snapshot below has the new names).
    project::Entity::find_by_id(service.project_id)
        .lock_shared()
        .one(txn)
        .await?;
    let service = &service::Entity::find_by_id(service.id)
        .lock_exclusive()
        .one(txn)
        .await?
        .ok_or(AppError::NotFound)?;
    // The environment it applies, sealed apart from the snapshot.
    let (mut snapshot, image, env, files) = match (trigger, rollback_to) {
        (Trigger::Rollback, Some(earlier)) => {
            let snapshot: Snapshot =
                serde_json::from_value(earlier.spec.clone()).map_err(|_| {
                    AppError::BadRequest("that deployment has no configuration to return to".into())
                })?;
            if earlier.service_id != service.id {
                return Err(AppError::BadRequest(
                    "that deployment is of another service".into(),
                ));
            }
            let Some(image) = earlier.image_digest.clone() else {
                return Err(AppError::BadRequest(
                    "that deployment ran no recorded image".into(),
                ));
            };
            // The service's names now: an earlier name may have been
            // destroyed, then taken by another service.
            let current = snapshot_without_env(txn, service).await?;
            let mut snapshot = snapshot;
            snapshot.project = current.project;
            snapshot.service = current.service;
            // Its environment as it was (credentials: the service's at run).
            (
                snapshot,
                Some(image),
                earlier.env.clone(),
                earlier.files.clone(),
            )
        }
        (Trigger::Rollback, None) => {
            return Err(AppError::BadRequest("a rollback needs a deployment".into()));
        }
        // Removing needs no secret (one sealed with a lost key included).
        (Trigger::Destroy, _) => (snapshot_without_env(txn, service).await?, None, None, None),
        _ => {
            let env = service_env(service, keyring).map_err(|_| unreadable_env())?;
            let files = service_files(txn, service, keyring).await?;
            let key = env_digest_key(keyring);
            let snapshot = snapshot_without_env(txn, service)
                .await?
                .with_env(&env, &key)
                .with_files(&files, &key);
            let sealed = (!env.is_empty())
                .then(|| Encrypted::seal(keyring, &deployment::env_context(), &env))
                .transpose()?;
            let files = (!files.is_empty())
                .then(|| Encrypted::seal(keyring, &deployment::files_context(), &files))
                .transpose()?;
            (snapshot, None, sealed, files)
        }
    };
    match trigger {
        Trigger::Start | Trigger::Deploy | Trigger::Restart | Trigger::Rollback => {
            snapshot.running = trigger != Trigger::Deploy || service.desired_state != "stopped";
        }
        Trigger::Stop | Trigger::Destroy => snapshot.running = false,
    }
    let row = deployment::ActiveModel {
        service_id: Set(service.id),
        trigger: Set(trigger.as_str().to_owned()),
        status: Set(QUEUED.to_owned()),
        spec: Set(serde_json::to_value(&snapshot).map_err(AppError::internal)?),
        spec_hash: Set(Some(snapshot.hash())),
        image_digest: Set(image),
        env: Set(env),
        files: Set(files),
        published: Set(false),
        user_id: Set(user_id),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(txn)
    .await?;
    let lease = now + chrono::Duration::from_std(QUEUED_LEASE).unwrap_or_default();
    let claimed = service::Entity::update_many()
        .col_expr(service::Column::CurrentDeploymentId, Expr::value(row.id))
        .col_expr(service::Column::LockExpiresAt, Expr::value(lease))
        .col_expr(service::Column::Status, Expr::value("deploying"))
        .col_expr(
            service::Column::DesiredState,
            Expr::value(if snapshot.running {
                "running"
            } else {
                "stopped"
            }),
        )
        .col_expr(service::Column::UpdatedAt, Expr::value(now))
        .filter(service::Column::Id.eq(service.id))
        .filter(
            Condition::any()
                .add(service::Column::CurrentDeploymentId.is_null())
                .add(service::Column::LockExpiresAt.lt(now)),
        )
        .exec(txn)
        .await?;
    if claimed.rows_affected == 0 {
        // The caller's transaction rolls back: the row above goes with it.
        return Err(busy());
    }
    // A deployment whose lease ran out and was taken over: interrupted (its
    // run, if it ever comes, stops at the lock).
    deployment::Entity::update_many()
        .col_expr(deployment::Column::Status, Expr::value(INTERRUPTED))
        .col_expr(
            deployment::Column::Error,
            Expr::value("interrupted: its lock expired and another deployment took over"),
        )
        .col_expr(deployment::Column::FinishedAt, Expr::value(now))
        .col_expr(deployment::Column::UpdatedAt, Expr::value(now))
        .filter(deployment::Column::ServiceId.eq(service.id))
        .filter(deployment::Column::Id.ne(row.id))
        .filter(deployment::Column::Status.is_in([QUEUED, RUNNING]))
        .exec(txn)
        .await?;
    Ok(row)
}

/// How a run waits for Swarm.
#[derive(Debug, Clone, Copy)]
pub struct Timings {
    /// Between two looks at the service.
    pub poll: Duration,
    /// After an update is sent, before tasks alone may tell it is done
    /// (Swarm shows the update as started within a second).
    pub settle: Duration,
    /// Until the run gives up.
    pub timeout: Duration,
}

impl Default for Timings {
    fn default() -> Self {
        Self {
            poll: Duration::from_secs(2),
            settle: Duration::from_secs(4),
            timeout: Duration::from_secs(10 * 60),
        }
    }
}

/// Lines of a run, kept on the deployment row (its tail), written at most
/// every second.
/// It also holds the service's lock ([`RunLog::hold`]).
pub(crate) struct RunLog<'a> {
    db: &'a DatabaseConnection,
    id: i64,
    service_id: i64,
    text: String,
    saved: std::time::Instant,
    held: std::time::Instant,
}

impl<'a> RunLog<'a> {
    fn new(db: &'a DatabaseConnection, row: &deployment::Model) -> Self {
        Self {
            db,
            id: row.id,
            service_id: row.service_id,
            text: row.log.clone().unwrap_or_default(),
            saved: std::time::Instant::now(),
            held: std::time::Instant::now(),
        }
    }

    /// Renews the lease: the run goes on only while the lock is its own.
    pub(crate) async fn hold(&mut self) -> Result<(), Outcome> {
        self.held = std::time::Instant::now();
        match hold(self.db, self.service_id, self.id).await {
            Ok(true) => Ok(()),
            Ok(false) => Err(Outcome::Failed(
                "the service's lock was lost (another deployment took over)".into(),
            )),
            Err(err) => Err(Outcome::Failed(format!(
                "the lock could not be renewed: {err}"
            ))),
        }
    }

    /// [`Self::hold`], at most every few seconds (while pulling).
    pub(crate) async fn hold_now_and_then(&mut self) -> Result<(), Outcome> {
        if self.held.elapsed() >= Duration::from_secs(5) {
            self.hold().await?;
        }
        Ok(())
    }

    pub(crate) async fn line(&mut self, message: impl AsRef<str>) {
        self.text.push_str(&format!(
            "{} {}\n",
            Utc::now().format("%H:%M:%S"),
            message.as_ref()
        ));
        if self.text.len() > 2 * crate::models::action::LOG_LIMIT {
            self.text = crate::models::action::tail(&self.text).to_owned();
        }
        if self.saved.elapsed() >= Duration::from_secs(1) {
            self.save().await;
        }
    }

    async fn save(&mut self) {
        self.saved = std::time::Instant::now();
        let log = crate::models::action::tail(&self.text).to_owned();
        let _ = deployment::Entity::update_many()
            .col_expr(deployment::Column::Log, Expr::value(log))
            .col_expr(deployment::Column::UpdatedAt, Expr::value(Utc::now()))
            .filter(deployment::Column::Id.eq(self.id))
            .exec(self.db)
            .await;
    }
}

/// Renews the lease of `deployment` on its service: whether the lock is
/// still its own and its lease had not run out.
async fn hold(db: &DatabaseConnection, service_id: i64, deployment: i64) -> AppResult<bool> {
    let now = Utc::now();
    let lease = now + chrono::Duration::from_std(LEASE).unwrap_or_default();
    service::Entity::update_many()
        .col_expr(service::Column::LockExpiresAt, Expr::value(lease))
        .filter(service::Column::Id.eq(service_id))
        .filter(service::Column::CurrentDeploymentId.eq(deployment))
        .filter(service::Column::LockExpiresAt.gte(now))
        .exec(db)
        .await?;
    // Read back (MySQL counts an unchanged row as not affected).
    Ok(service::Entity::find_by_id(service_id)
        .one(db)
        .await?
        .is_some_and(|s| {
            s.current_deployment_id == Some(deployment)
                && s.lock_expires_at.is_some_and(|at| at >= now)
        }))
}

/// How a run ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Outcome {
    Succeeded,
    /// Swarm rolled back: why, and whether the restored spec runs tasks.
    RolledBack(String, bool),
    Failed(String),
}

/// Runs deployment `id` (the job's body). Safe to run again: a deployment
/// that is over, or that lost its lock, is left alone.
pub async fn run(state: &AppState, id: i64, timings: Timings) -> AppResult<()> {
    let db = state.db();
    let Some(row) = deployment::Entity::find_by_id(id).one(db).await? else {
        return Ok(());
    };
    if row.status != QUEUED && row.status != RUNNING {
        return Ok(());
    }
    let Some(service) = service::Entity::find_by_id(row.service_id).one(db).await? else {
        return finish(
            db,
            row,
            Outcome::Failed("the service no longer exists".into()),
            None,
        )
        .await;
    };
    // Fencing: only the deployment holding the lock (not expired) touches Docker.
    if service.current_deployment_id != Some(row.id) || !hold(db, service.id, row.id).await? {
        return finish(
            db,
            row,
            Outcome::Failed("another deployment took over, or the lock expired".into()),
            None,
        )
        .await;
    }
    let mut active = row.clone().into_active_model();
    active.status = Set(RUNNING.to_owned());
    active.started_at = Set(row.started_at.or(Some(Utc::now())));
    active.updated_at = Set(Utc::now());
    let row = active.update(db).await?;
    let mut log = RunLog::new(db, &row);
    let traefik = state.service::<Traefik>();
    let outcome = match state.service::<Docker>() {
        Some(docker) => {
            let secrets = Secrets::of(&row, &service, state.keyring());
            if let Ok(secrets) = &secrets
                && let Some(note) = secrets.unmatched(&row)
            {
                log.line(note).await;
            }
            match secrets {
                Ok(secrets) => {
                    apply(
                        &docker,
                        traefik.as_deref(),
                        &row,
                        &secrets,
                        &mut log,
                        timings,
                    )
                    .await
                }
                Err(outcome) => Err(outcome),
            }
        }
        None => Err(Outcome::Failed("Docker is not available".into())),
    };
    let (outcome, image) = match outcome {
        Ok((outcome, image)) => (outcome, image),
        Err(outcome) => (outcome, None),
    };
    match &outcome {
        Outcome::Succeeded => log.line("done").await,
        Outcome::RolledBack(why, _) => log.line(format!("rolled back: {why}")).await,
        Outcome::Failed(why) => log.line(format!("failed: {why}")).await,
    }
    log.save().await;
    let row = deployment::Entity::find_by_id(row.id)
        .one(db)
        .await?
        .ok_or(AppError::NotFound)?;
    let id = row.id;
    finish(db, row, outcome, image).await?;
    // Traefik follows what the service is now (this deployment's routes if
    // it succeeded, the previous ones otherwise).
    let published = match traefik {
        Some(traefik) => publish(db, &traefik, service.id, id).await.map(drop),
        None => Ok(()),
    };
    // A push came meanwhile: deployed again (claimed once).
    match redeploy_if_pending(state, service.id).await {
        Ok(()) | Err(AppError::Conflict { .. }) => {}
        Err(err) => {
            tracing::warn!(%err, service = service.id, "the pending deployment not asked for");
        }
    }
    // Said in the background: never in the way of the routes.
    let notify = state.clone();
    tokio::spawn(async move {
        crate::services::notify_service::deployment_finished(&notify, id).await;
    });
    published
}

/// Every service whose pending Deploy was left (its deployment cut short):
/// asked for now when it is idle. Every minute, after the reaper.
pub async fn drain_pending(state: &AppState) -> AppResult<()> {
    let pending = service::Entity::find()
        .filter(service::Column::RedeployPending.eq(true))
        .all(state.db())
        .await?;
    for row in pending {
        match redeploy_if_pending(state, row.id).await {
            Ok(()) | Err(AppError::Conflict { .. }) => {}
            Err(err) => {
                tracing::warn!(%err, service = row.id, "the pending deployment not asked for")
            }
        }
    }
    Ok(())
}

/// Asks for the Deploy a push asked for while another one ran (the flag
/// cleared in the same transaction: kept when one still runs).
pub async fn redeploy_if_pending(state: &AppState, service_id: i64) -> AppResult<()> {
    let db = state.db();
    let txn = rustcms::db::begin_write(db).await?;
    let claimed = service::Entity::update_many()
        .col_expr(service::Column::RedeployPending, Expr::value(false))
        .filter(service::Column::Id.eq(service_id))
        .filter(service::Column::RedeployPending.eq(true))
        .exec(&txn)
        .await?;
    if claimed.rows_affected == 0 {
        return Ok(());
    }
    let row = service::Entity::find_by_id(service_id)
        .one(&txn)
        .await?
        .ok_or(AppError::NotFound)?;
    // Privileged since the push: its administrators deploy it.
    if crate::admin::resources::service_resource::privileged(&row) {
        txn.commit().await?;
        return Ok(());
    }
    let deployment = request(&txn, &row, Trigger::Deploy, None, None, state.keyring()).await?;
    rustcms::queue::dispatch(
        &txn,
        &crate::jobs::run_deployment::RunDeployment {
            deployment_id: deployment.id,
        },
    )
    .await?;
    txn.commit().await?;
    Ok(())
}

/// Writes the routes of the service as deployed now; records on deployment
/// `id` whether that worked ([`deployment::Model::published`]).
pub async fn publish(
    db: &DatabaseConnection,
    traefik: &Traefik,
    service_id: i64,
    id: i64,
) -> AppResult<bool> {
    let service = service::Entity::find_by_id(service_id)
        .one(db)
        .await?
        .ok_or(AppError::NotFound)?;
    let written = traefik_service::republish(db, traefik, &service, &deploy_options(0)).await?;
    deployment::Entity::update_many()
        .col_expr(deployment::Column::Published, Expr::value(written))
        .filter(deployment::Column::Id.eq(id))
        .exec(db)
        .await?;
    Ok(written)
}

/// The routes' and spec's options from the `[panel]` configuration.
pub fn deploy_options(force_update: u64) -> DeployOptions {
    let cfg = config();
    DeployOptions {
        force_update,
        cert_resolver: cfg.cert_resolver.clone(),
        https_port: cfg.https_port,
        internal_url: cfg.internal_url.clone(),
    }
}

/// The project's network (made when missing), and Traefik on it when the
/// service runs with domains: Traefik reaches the service there.
pub(crate) async fn project_network(
    docker: &Docker,
    traefik: Option<&Traefik>,
    snapshot: &Snapshot,
    log: &mut RunLog<'_>,
) -> Result<(), Outcome> {
    let fail = |err: DockerError| Outcome::Failed(err.to_string());
    let network = NetworkRequest {
        name: network_name(&snapshot.project),
        driver: "overlay".to_owned(),
        labels: [
            (MANAGED_LABEL.to_owned(), "true".to_owned()),
            ("rdockpanel.project".to_owned(), snapshot.project.clone()),
            // This panel's: only its own networks are ever removed.
            (
                "rdockpanel.instance".to_owned(),
                crate::services::deploy_service::instance()
                    .unwrap_or_default()
                    .to_owned(),
            ),
        ]
        .into(),
        attachable: true,
        internal: false,
    };
    log.hold().await?;
    let network = docker.ensure_network(network).await.map_err(fail)?;
    // Traefik reaches the service on its project's network.
    if snapshot.running
        && !snapshot.domains.is_empty()
        && let Some(traefik) = traefik
    {
        match traefik.attach(docker, &network.name, &network.id).await {
            Ok(true) => {
                log.line(format!(
                    "attached Traefik to {} (it restarts)",
                    network.name
                ))
                .await;
            }
            Ok(false) => {}
            // Its domains would lead nowhere: not deployed.
            Err(err) => {
                return Err(Outcome::Failed(format!(
                    "Traefik ({}) could not join {}: {err}",
                    traefik.service, network.name
                )));
            }
        }
    }
    Ok(())
}

/// Applies `row` to Docker: the outcome and the image it ran, or how it failed.
/// What a run applies besides its snapshot, opened: never logged.
pub(crate) struct Secrets {
    pub(crate) env: BTreeMap<String, String>,
    /// The service's deploy key (a private repository's).
    pub(crate) deploy_key: Option<String>,
    /// Its file mounts' contents by path.
    files: BTreeMap<String, String>,
    /// Keys the digest of the environment as resolved (the task template's).
    digest_key: [u8; 32],
    /// The service's registry credentials now (server, user, password),
    /// for the registry of its image now.
    registry: Option<(String, String, String)>,
}

impl Secrets {
    /// Nothing for a Destroy: it needs no secret, even one that cannot be
    /// opened any more.
    fn of(
        row: &deployment::Model,
        service: &service::Model,
        keyring: &Keyring,
    ) -> Result<Self, Outcome> {
        let digest_key = env_digest_key(keyring);
        if Trigger::parse(&row.trigger) == Some(Trigger::Destroy) {
            return Ok(Self {
                env: BTreeMap::new(),
                deploy_key: None,
                files: BTreeMap::new(),
                digest_key,
                registry: None,
            });
        }
        let env = match &row.env {
            Some(sealed) => sealed
                .open(keyring, &deployment::env_context())
                .map_err(|_| {
                    Outcome::Failed(
                        "its environment cannot be read (sealed with another APP_KEY)".into(),
                    )
                })?,
            None => BTreeMap::new(),
        };
        let username = service
            .registry_username
            .as_deref()
            .map(str::trim)
            .filter(|u| !u.is_empty());
        let registry = match (username, &service.registry_password) {
            (Some(username), Some(sealed)) => {
                let password = sealed
                    .open(keyring, &service::registry_password_context())
                    .map_err(|_| {
                        Outcome::Failed(
                            "the registry password cannot be read (sealed with another APP_KEY)"
                                .into(),
                        )
                    })?;
                Some((registry_of(&service.image), username.to_owned(), password))
            }
            _ => None,
        };
        let files = match &row.files {
            Some(sealed) => sealed
                .open(keyring, &deployment::files_context())
                .map_err(|_| {
                    Outcome::Failed("its files cannot be read (sealed with another APP_KEY)".into())
                })?,
            None => BTreeMap::new(),
        };
        let deploy_key = match &service.deploy_key {
            Some(sealed) => Some(
                sealed
                    .open(keyring, &service::deploy_key_context())
                    .map_err(|_| {
                        Outcome::Failed(
                            "the deploy key cannot be read (sealed with another APP_KEY)".into(),
                        )
                    })?,
            ),
            None => None,
        };
        Ok(Self {
            env,
            deploy_key,
            files,
            digest_key,
            registry,
        })
    }

    /// Credentials for `snapshot`'s image: the service's, only when they are
    /// for that image's registry (a rollback to an image of another registry
    /// never gets them).
    fn auth(&self, snapshot: &Snapshot) -> Option<RegistryAuth> {
        let (server, username, password) = self.registry.as_ref()?;
        (registry_of(&snapshot.image) == *server).then(|| RegistryAuth {
            server: server.clone(),
            username: username.clone(),
            password: password.clone(),
        })
    }

    /// A note for the log when the service has credentials, but for another
    /// registry than the image this run deploys.
    fn unmatched(&self, row: &deployment::Model) -> Option<String> {
        let (server, _, _) = self.registry.as_ref()?;
        let snapshot: Snapshot = serde_json::from_value(row.spec.clone()).ok()?;
        let wanted = registry_of(&snapshot.image);
        (wanted != *server).then(|| {
            format!("the registry credentials are for {server}, not {wanted}: pulling without them")
        })
    }
}

/// The label of a service's Swarm configs: `<project>_<service>`.
const CONFIG_OF_LABEL: &str = "rdockpanel.config-of";

/// The Swarm configs of `snapshot`'s file mounts, created when missing (a
/// config never changes: a new content is a new config), by path.
async fn file_configs(
    docker: &Docker,
    snapshot: &Snapshot,
    contents: &BTreeMap<String, String>,
    log: &mut RunLog<'_>,
) -> Result<BTreeMap<String, ConfigRef>, Outcome> {
    let mut out = BTreeMap::new();
    let files: Vec<_> = snapshot
        .mounts
        .iter()
        .filter(|m| m.kind == "file")
        .collect();
    if files.is_empty() {
        return Ok(out);
    }
    let api = docker.engine();
    let owner = snapshot.service_name();
    let fail = |err: DockerError| Outcome::Failed(format!("file mounts: {err}"));
    let mut existing = api
        .list_configs(CONFIG_OF_LABEL, &owner)
        .await
        .map_err(fail)?;
    for file in files {
        let (Some(name), Some(content)) = (file.config_name(snapshot), contents.get(&file.target))
        else {
            continue;
        };
        let id = match existing.iter().find(|(_, n)| *n == name) {
            Some((id, _)) => id.clone(),
            None => {
                log.hold().await?;
                let labels = HashMap::from([
                    (MANAGED_LABEL.to_owned(), "true".to_owned()),
                    ("rdockpanel.project".to_owned(), snapshot.project.clone()),
                    ("rdockpanel.service".to_owned(), snapshot.service.clone()),
                    (CONFIG_OF_LABEL.to_owned(), owner.clone()),
                ]);
                match api.create_config(&name, content.as_bytes(), labels).await {
                    Ok(id) => {
                        log.line(format!("file {} ({name})", file.target)).await;
                        id
                    }
                    // Made meanwhile (a run before this one): use it.
                    Err(DockerError::Conflict(_)) => {
                        existing = api
                            .list_configs(CONFIG_OF_LABEL, &owner)
                            .await
                            .map_err(fail)?;
                        existing
                            .iter()
                            .find(|(_, n)| *n == name)
                            .map(|(id, _)| id.clone())
                            .ok_or_else(|| {
                                Outcome::Failed(format!("file mounts: {name} vanished"))
                            })?
                    }
                    Err(err) => return Err(fail(err)),
                }
            }
        };
        out.insert(file.target.clone(), ConfigRef { id, name });
    }
    Ok(out)
}

/// Removes the service's configs it no longer uses (best effort: one still
/// in use, by tasks shutting down, stays for the next run), each only while
/// the lock is still this run's.
async fn prune_configs(
    docker: &Docker,
    owner: &str,
    used: &BTreeMap<String, ConfigRef>,
    log: &mut RunLog<'_>,
) -> Result<(), Outcome> {
    let api = docker.engine();
    let Ok(configs) = api.list_configs(CONFIG_OF_LABEL, owner).await else {
        return Ok(());
    };
    for (id, _) in configs {
        if !used.values().any(|c| c.id == id) {
            log.hold().await?;
            let _ = api.delete_config(&id).await;
        }
    }
    Ok(())
}

/// What placeholders stand for in `snapshot`'s environment.
pub fn placeholder_vars(snapshot: &Snapshot) -> placeholders::Vars {
    let cfg = config();
    let set = |s: &str| (!s.trim().is_empty()).then(|| s.trim().to_owned());
    placeholders::Vars {
        project: snapshot.project.clone(),
        service: snapshot.service.clone(),
        primary_domain: snapshot.primary_domain.clone(),
        host: set(&cfg.service_domain),
        host_ip: set(&cfg.public_ip),
    }
}

/// The registry an image reference names (`ghcr.io/org/app:v1` →
/// `ghcr.io`); Docker Hub's address for the others.
pub fn registry_of(image: &str) -> String {
    match image.split_once('/') {
        Some((first, _)) if first.contains('.') || first.contains(':') || first == "localhost" => {
            first.to_owned()
        }
        _ => "https://index.docker.io/v1/".to_owned(),
    }
}

async fn apply(
    docker: &Docker,
    traefik: Option<&Traefik>,
    row: &deployment::Model,
    secrets: &Secrets,
    log: &mut RunLog<'_>,
    timings: Timings,
) -> Result<(Outcome, Option<String>), Outcome> {
    let fail = |err: DockerError| Outcome::Failed(err.to_string());
    let snapshot: Snapshot =
        serde_json::from_value(row.spec.clone()).map_err(|e| Outcome::Failed(e.to_string()))?;
    let trigger =
        Trigger::parse(&row.trigger).ok_or_else(|| Outcome::Failed("unknown trigger".into()))?;
    let name = snapshot.service_name();
    // Not routed any more before its containers go (a Compose service too).
    if let Some(source) = &snapshot.compose {
        if !snapshot.running
            && let Some(traefik) = traefik
            && let Err(err) = traefik.remove(&name).await
        {
            log.line(format!("warning: routes not removed: {err}"))
                .await;
        }
        return crate::services::compose_service::apply(
            docker, traefik, &snapshot, source, trigger, secrets, log,
        )
        .await;
    }
    let existing = docker
        .managed_service(&name, &snapshot.project, &snapshot.service)
        .await
        .map_err(fail)?;
    let api = docker.engine();

    // Not routed any more before its tasks go.
    if !snapshot.running
        && let Some(traefik) = traefik
        && let Err(err) = traefik.remove(&name).await
    {
        log.line(format!("warning: routes not removed: {err}"))
            .await;
    }
    if trigger == Trigger::Destroy {
        if existing.is_some() {
            log.line(format!("removing {name}")).await;
            log.hold().await?;
            api.delete_service(&name).await.map_err(fail)?;
        } else {
            log.line(format!("{name} is not on Docker")).await;
        }
        // Its files too (also those of a run that never made the service).
        prune_configs(docker, &name, &BTreeMap::new(), log).await?;
        return Ok((Outcome::Succeeded, None));
    }
    if existing.is_none() && !snapshot.running {
        log.line(format!("{name} is not on Docker: nothing to stop"))
            .await;
        return Ok((Outcome::Succeeded, None));
    }

    // Host directories are checked again: they may have changed since saved.
    for mount in snapshot.mounts.iter().filter(|m| m.kind == "bind") {
        match bind_source(&mount.source, &config().bind_roots) {
            Ok(resolved) if resolved.to_string_lossy() == mount.source => {}
            _ => {
                return Err(Outcome::Failed(format!(
                    "the host directory {} is no longer allowed",
                    mount.source
                )));
            }
        }
    }

    project_network(docker, traefik, &snapshot, log).await?;

    let auth = secrets.auth(&snapshot);
    let image = match &snapshot.build {
        // Built here: Deploy builds; Start, Stop and Restart reuse the image
        // running (when there is one); Rollback its recorded one.
        Some(build)
            if trigger == Trigger::Deploy
                || (trigger != Trigger::Rollback
                    && existing.as_ref().and_then(running_image).is_none()) =>
        {
            let tag = format!("rdp/{name}:d{}", row.id);
            let built = crate::services::build_service::build(
                docker,
                build,
                &tag,
                secrets.deploy_key.as_deref(),
                log,
            )
            .await?;
            PinnedImage::Local(built)
        }
        // Start, Stop, Restart of a built image: the one running.
        Some(_) if trigger != Trigger::Rollback => pinned_as(
            &existing
                .as_ref()
                .and_then(running_image)
                .unwrap_or_default(),
        ),
        _ => {
            image_for(
                docker,
                trigger,
                &snapshot,
                row,
                existing.as_ref(),
                auth.clone(),
                log,
            )
            .await?
        }
    };
    log.hold().await?;
    if let Some(warning) = image.warning() {
        log.line(format!("warning: {warning}")).await;
    }
    let options =
        deploy_options(force_update_of(existing.as_ref()) + u64::from(trigger == Trigger::Restart));
    // Placeholders (`$(PROJECT_NAME)`, …) in the environment, as of now.
    let vars = placeholder_vars(&snapshot);
    let env: BTreeMap<String, String> = secrets
        .env
        .iter()
        .map(|(k, v)| (k.clone(), placeholders::expand(v, &vars)))
        .collect();
    let configs = file_configs(docker, &snapshot, &secrets.files, log).await?;
    let env_digest = crate::services::deploy_service::env_digest(&env, &secrets.digest_key);
    let runtime = Runtime {
        env,
        env_digest,
        configs,
    };
    let out = desired(&snapshot, &image, &runtime, &options);
    let sent = Utc::now();
    match existing {
        None => {
            log.line(format!("creating {name} ({})", image.reference()))
                .await;
            log.hold().await?;
            api.create_service(out.spec.clone(), auth.clone())
                .await
                .map_err(fail)?;
        }
        Some(_) => {
            log.line(format!("updating {name} ({})", image.reference()))
                .await;
            update(
                docker,
                &name,
                &snapshot,
                out.spec.clone(),
                auth.clone(),
                log,
            )
            .await?;
        }
    }
    let replicas = if snapshot.running {
        snapshot.replicas as usize
    } else {
        0
    };
    let outcome = converge(api, &name, replicas, &out.template, sent, log, timings).await;
    if outcome == Outcome::Succeeded {
        prune_configs(docker, &name, &runtime.configs, log).await?;
    }
    Ok((outcome, Some(image.reference().to_owned())))
}

/// The image a run uses: a rollback's recorded one, the running one for a
/// start or stop, else a fresh pull of the configured image, pinned.
async fn image_for(
    docker: &Docker,
    trigger: Trigger,
    snapshot: &Snapshot,
    row: &deployment::Model,
    existing: Option<&SwarmService>,
    auth: Option<RegistryAuth>,
    log: &mut RunLog<'_>,
) -> Result<PinnedImage, Outcome> {
    if trigger == Trigger::Rollback {
        return match row.image_digest.as_deref() {
            Some(recorded) => Ok(pinned_as(recorded)),
            None => Err(Outcome::Failed("no recorded image to return to".into())),
        };
    }
    let running = existing
        .and_then(|s| s.spec.as_ref())
        .and_then(|s| s.task_template.as_ref())
        .and_then(|t| t.container_spec.as_ref())
        .and_then(|c| c.image.clone());
    if let (Some(running), Trigger::Start | Trigger::Stop) = (running, trigger) {
        return Ok(pinned_as(&running));
    }
    log.line(format!("pulling {}", snapshot.image)).await;
    let mut progress = docker.pull_image_with(&snapshot.image, auth);
    let mut lines = Vec::new();
    use futures_util::StreamExt;
    while let Some(line) = progress.next().await {
        let line = line.map_err(|e| Outcome::Failed(format!("pull failed: {e}")))?;
        log.hold_now_and_then().await?;
        if line.starts_with("Digest:") || line.starts_with("Status:") {
            log.line(&line).await;
        }
        lines.push(line);
    }
    let details = docker
        .engine()
        .inspect_image(&snapshot.image)
        .await
        .map_err(|e| Outcome::Failed(e.to_string()))?;
    let digest = pulled_digest(lines.iter().map(String::as_str));
    Ok(pin_image(&snapshot.image, digest.as_deref(), &details))
}

/// The image a service's spec runs.
fn running_image(service: &SwarmService) -> Option<String> {
    service
        .spec
        .as_ref()?
        .task_template
        .as_ref()?
        .container_spec
        .as_ref()?
        .image
        .clone()
}

fn pinned_as(reference: &str) -> PinnedImage {
    if reference.contains("@sha256:") {
        PinnedImage::Digest(reference.to_owned())
    } else {
        PinnedImage::Local(reference.to_owned())
    }
}

fn force_update_of(service: Option<&SwarmService>) -> u64 {
    service
        .and_then(|s| s.spec.as_ref())
        .and_then(|s| s.task_template.as_ref())
        .and_then(|t| t.force_update)
        .unwrap_or(0)
}

/// Updates at the version Docker has now, again when Swarm moved it meanwhile.
async fn update(
    docker: &Docker,
    name: &str,
    snapshot: &Snapshot,
    spec: crate::services::docker_service::ServiceSpec,
    auth: Option<RegistryAuth>,
    log: &mut RunLog<'_>,
) -> Result<(), Outcome> {
    let fail = |err: DockerError| Outcome::Failed(err.to_string());
    for _ in 0..5 {
        let current = docker
            .managed_service(name, &snapshot.project, &snapshot.service)
            .await
            .map_err(fail)?
            .ok_or(Outcome::Failed(format!("{name} disappeared")))?;
        let version = current.version.and_then(|v| v.index).unwrap_or(0);
        // Each try only while the lock is still this run's.
        log.hold().await?;
        match docker
            .engine()
            .update_service(name, version, spec.clone(), auth.clone())
            .await
        {
            Err(DockerError::Conflict(_)) => tokio::time::sleep(Duration::from_millis(200)).await,
            other => return other.map(drop).map_err(fail),
        }
    }
    Err(Outcome::Failed("the service kept changing".into()))
}

/// When an update status time is from this run (`sent`, with a little slack
/// for clocks).
fn since(at: &impl serde::Serialize, sent: DateTime<Utc>) -> bool {
    serde_json::to_value(at)
        .ok()
        .and_then(|v| {
            v.as_str()
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        })
        .is_some_and(|at| at.with_timezone(&Utc) >= sent - chrono::Duration::seconds(2))
}

/// The template a task runs ([`TEMPLATE_LABEL`]).
fn task_template(task: &SwarmTask) -> Option<&str> {
    task.spec
        .as_ref()?
        .container_spec
        .as_ref()?
        .labels
        .as_ref()?
        .get(TEMPLATE_LABEL)
        .map(String::as_str)
}

/// A task that may still run a container (not ended, whatever Swarm wants).
fn is_live(task: &SwarmTask) -> bool {
    !matches!(
        task_state(task),
        Some(
            TaskState::COMPLETE
                | TaskState::SHUTDOWN
                | TaskState::FAILED
                | TaskState::REJECTED
                | TaskState::REMOVE
                | TaskState::ORPHANED
        )
    )
}

/// The replicas of a service's spec (after Swarm rolled it back).
fn spec_replicas(service: &SwarmService) -> Option<i64> {
    service
        .spec
        .as_ref()?
        .mode
        .as_ref()?
        .replicated
        .as_ref()?
        .replicas
}

fn task_state(task: &SwarmTask) -> Option<TaskState> {
    task.status.as_ref().and_then(|s| s.state)
}

/// Waits until `name` runs `replicas` tasks of `template` (none at all when
/// stopped): Swarm's update status when this run started one, else the tasks.
async fn converge(
    api: &dyn crate::services::docker_service::DockerApi,
    name: &str,
    replicas: usize,
    template: &str,
    sent: DateTime<Utc>,
    log: &mut RunLog<'_>,
    timings: Timings,
) -> Outcome {
    use crate::services::docker_service::ServiceUpdateStatusStateEnum as Update;
    let started = std::time::Instant::now();
    let mut reported = None;
    let mut stable = 0;
    while started.elapsed() < timings.timeout {
        if let Err(lost) = log.hold().await {
            return lost;
        }
        let service = match api.inspect_service(name).await {
            Ok(Some(service)) => service,
            Ok(None) => return Outcome::Failed(format!("{name} disappeared")),
            Err(err) => return Outcome::Failed(err.to_string()),
        };
        let update = service
            .update_status
            .as_ref()
            .filter(|u| u.started_at.as_ref().is_some_and(|at| since(at, sent)));
        let state = update.and_then(|u| u.state);
        let message = update.and_then(|u| u.message.clone()).unwrap_or_default();
        if state != reported && state.is_some() {
            log.line(format!(
                "update: {}",
                state.map(|s| s.to_string()).unwrap_or_default()
            ))
            .await;
            reported = state;
        }
        match state {
            Some(Update::ROLLBACK_COMPLETED) => {
                let running = spec_replicas(&service).is_none_or(|n| n > 0);
                return Outcome::RolledBack(message, running);
            }
            Some(Update::PAUSED | Update::ROLLBACK_PAUSED) => return Outcome::Failed(message),
            Some(Update::UPDATING | Update::ROLLBACK_STARTED) => stable = 0,
            _ => {
                let tasks = match api.list_tasks(name).await {
                    Ok(tasks) => tasks,
                    Err(err) => return Outcome::Failed(err.to_string()),
                };
                let wanted: Vec<&SwarmTask> = tasks
                    .iter()
                    .filter(|t| t.desired_state == Some(TaskState::RUNNING))
                    .collect();
                let running = wanted
                    .iter()
                    .filter(|t| {
                        task_state(t) == Some(TaskState::RUNNING)
                            && task_template(t) == Some(template)
                    })
                    .count();
                let failed: Vec<&SwarmTask> = tasks
                    .iter()
                    .filter(|t| {
                        matches!(task_state(t), Some(TaskState::FAILED | TaskState::REJECTED))
                    })
                    // Swarm refuses tasks for a while when a network was
                    // just removed and its addresses given again ("Pool
                    // overlaps"); it retries them: only the timeout ends it.
                    .filter(|t| {
                        !t.status
                            .as_ref()
                            .and_then(|s| s.err.as_deref())
                            .is_some_and(|e| e.contains("Pool overlaps"))
                    })
                    .filter(|t| {
                        t.status
                            .as_ref()
                            .and_then(|s| s.timestamp.as_ref())
                            .is_some_and(|at| since(at, sent))
                    })
                    .collect();
                // Stopped: every task ended, also those Swarm already
                // wants shut down. Else only this template's tasks run.
                let done = if replicas == 0 {
                    !tasks.iter().any(is_live)
                } else {
                    running == replicas && wanted.len() == replicas
                };
                // Tasks alone decide once Swarm had time to show an update.
                if done && (state == Some(Update::COMPLETED) || started.elapsed() >= timings.settle)
                {
                    stable += 1;
                    if stable >= 2 || state == Some(Update::COMPLETED) {
                        return Outcome::Succeeded;
                    }
                } else {
                    stable = 0;
                }
                if failed.len() >= 3 && running < replicas {
                    let why = failed
                        .iter()
                        .filter_map(|t| t.status.as_ref()?.err.clone())
                        .next()
                        .unwrap_or_else(|| "its tasks keep failing".into());
                    return Outcome::Failed(why);
                }
            }
        }
        tokio::time::sleep(timings.poll).await;
    }
    Outcome::Failed("timed out waiting for the service".into())
}

/// Ends a run: the deployment's status, the service's state, the lock
/// released if still this deployment's.
async fn finish(
    db: &DatabaseConnection,
    row: deployment::Model,
    outcome: Outcome,
    image: Option<String>,
) -> AppResult<()> {
    let now = Utc::now();
    let (status, error) = match &outcome {
        Outcome::Succeeded => (SUCCEEDED, None),
        Outcome::RolledBack(why, _) => (ROLLED_BACK, Some(why.clone())),
        Outcome::Failed(why) => (FAILED, Some(why.clone())),
    };
    let trigger = Trigger::parse(&row.trigger);
    let spec_hash = row.spec_hash.clone();
    let running = row
        .spec
        .get("running")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true);
    let (service_id, deployment_id) = (row.service_id, row.id);
    // The deployment's end and the lock's release go together.
    let txn = db.begin().await?;
    let mut active = row.into_active_model();
    active.status = Set(status.to_owned());
    active.error = Set(error);
    if image.is_some() {
        active.image_digest = Set(image);
    }
    active.finished_at = Set(Some(now));
    active.updated_at = Set(now);
    active.update(&txn).await?;

    // The service, only while this deployment holds it.
    let mut update = service::Entity::update_many()
        .col_expr(
            service::Column::CurrentDeploymentId,
            Expr::value(Option::<i64>::None),
        )
        .col_expr(
            service::Column::LockExpiresAt,
            Expr::value(Option::<DateTime<Utc>>::None),
        )
        .col_expr(service::Column::UpdatedAt, Expr::value(now));
    update = match (&outcome, trigger) {
        (Outcome::Succeeded, Some(Trigger::Destroy)) => update
            .col_expr(service::Column::Status, Expr::value(service::NEW))
            .col_expr(
                service::Column::DeployedAt,
                Expr::value(Option::<DateTime<Utc>>::None),
            )
            .col_expr(
                service::Column::DeployedSpecHash,
                Expr::value(Option::<String>::None),
            ),
        (Outcome::Succeeded, _) => update
            .col_expr(
                service::Column::Status,
                Expr::value(if running { "running" } else { "stopped" }),
            )
            .col_expr(
                service::Column::DeployedAt,
                Expr::cust_with_values("COALESCE(deployed_at, ?)", [sea_orm::Value::from(now)]),
            )
            .col_expr(service::Column::DeployedSpecHash, Expr::value(spec_hash)),
        (Outcome::RolledBack(_, running), _) => update
            .col_expr(
                service::Column::Status,
                Expr::value(if *running { "running" } else { "stopped" }),
            )
            .col_expr(
                service::Column::DesiredState,
                Expr::value(if *running { "running" } else { "stopped" }),
            ),
        (Outcome::Failed(_), _) => update.col_expr(service::Column::Status, Expr::value("failed")),
    };
    update
        .filter(service::Column::Id.eq(service_id))
        .filter(service::Column::CurrentDeploymentId.eq(deployment_id))
        .exec(&txn)
        .await?;
    txn.commit().await?;
    Ok(())
}

/// Deployments still queued or running whose service lock expired (their run
/// died, or its job timed out) are `interrupted`, and the lock released. At
/// boot, then every minute (the `reap-deployments` schedule).
pub async fn recover_interrupted(db: &DatabaseConnection) -> AppResult<u64> {
    let now = Utc::now();
    let expired = service::Entity::find()
        .filter(service::Column::CurrentDeploymentId.is_not_null())
        .filter(service::Column::LockExpiresAt.lt(now))
        .all(db)
        .await?;
    let mut count = 0;
    for svc in expired {
        let Some(id) = svc.current_deployment_id else {
            continue;
        };
        deployment::Entity::update_many()
            .col_expr(deployment::Column::Status, Expr::value(INTERRUPTED))
            .col_expr(
                deployment::Column::Error,
                Expr::value("interrupted: the panel stopped before it ended"),
            )
            .col_expr(deployment::Column::FinishedAt, Expr::value(now))
            .col_expr(deployment::Column::UpdatedAt, Expr::value(now))
            .filter(deployment::Column::Id.eq(id))
            .filter(deployment::Column::Status.is_in([QUEUED, RUNNING]))
            .exec(db)
            .await?;
        let released = service::Entity::update_many()
            .col_expr(
                service::Column::CurrentDeploymentId,
                Expr::value(Option::<i64>::None),
            )
            .col_expr(
                service::Column::LockExpiresAt,
                Expr::value(Option::<DateTime<Utc>>::None),
            )
            .col_expr(service::Column::Status, Expr::value("failed"))
            .filter(service::Column::Id.eq(svc.id))
            .filter(service::Column::CurrentDeploymentId.eq(id))
            .filter(service::Column::LockExpiresAt.lt(now))
            .exec(db)
            .await?;
        count += released.rows_affected;
    }
    Ok(count)
}

/// The latest deployment of a service, for its page.
pub async fn latest(
    db: &DatabaseConnection,
    service_id: i64,
) -> AppResult<Option<deployment::Model>> {
    use sea_orm::QueryOrder;
    Ok(deployment::Entity::find()
        .filter(deployment::Column::ServiceId.eq(service_id))
        .order_by_desc(deployment::Column::Id)
        .one(db)
        .await?)
}

/// What a deployment's page shows of it.
pub fn summary(row: &deployment::Model) -> serde_json::Value {
    json!({
        "id": row.id,
        "trigger": row.trigger,
        "status": row.status,
        "error": row.error,
        "image": row.image_digest,
        "published": row.published,
        "log": row.log,
        "finished": !matches!(row.status.as_str(), QUEUED | RUNNING),
    })
}
