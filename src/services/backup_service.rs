//! Database backups (as EasyPanel's): a database service's dump, taken in its
//! running container with the engine's own tool (`pg_dump`, `mysqldump`,
//! `mariadb-dump`, `mongodump`) and the credentials of its environment (never
//! passed by the panel), gzipped, written to a storage provider as
//! `<path>/<project>_<service>.<database>.<UTC time>.r<run>.<ext>`; the oldest
//! files of that service's database in that path deleted beyond the
//! retention. A restore feeds a file back to the engine's client, in place.
//! Each run is a `backup_runs` row with its log, its file, provider and
//! retention fixed when it is asked for; one run of a service at a time;
//! scheduled runs come from each configuration's cron ([`tick`]).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use rustcms::queue;
use rustcms::{AppError, AppResult, AppState};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, DatabaseTransaction, EntityTrait,
    IntoActiveModel, PaginatorTrait, QueryFilter, QuerySelect, Set, TransactionTrait,
};

use crate::jobs::run_backup::RunBackup;
use crate::models::{backup_run, database_backup, project, service, storage_provider};
use crate::services::database_service::{Engine, random_string};
use crate::services::deploy_service::service_name;
use crate::services::docker_service::{Docker, ExecChunk, TaskState};
use crate::services::storage_service::{Store, valid_key};

/// A running run says it is alive this often…
const HEARTBEAT: Duration = Duration::from_secs(30);
/// …and is taken for dead (its process stopped) after this long without.
const STALE: Duration = Duration::from_secs(150);
/// A run taken for dead whose command is being killed: its service stays
/// blocked until it is.
const KILLING: &str = "killing";

/// A database name as the engines take it (passed as an argument anyway;
/// never an option).
pub fn valid_database(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('-')
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '$'))
}

/// The engines whose databases are backed up (Redis keeps its own files).
pub fn supported(engine: Engine) -> bool {
    !matches!(engine, Engine::Redis)
}

/// A five-field cron expression (UTC), as the scheduler takes it.
pub fn parse_schedule(expr: &str) -> Option<croner::Cron> {
    // The same rules as RustCMS's scheduler (no seconds, no years).
    rustcms::schedule::Schedule::cron("rdp-backup-check", expr).ok()?;
    croner::parser::CronParser::builder()
        .seconds(croner::parser::Seconds::Disallowed)
        .year(croner::parser::Year::Disallowed)
        .dom_and_dow(false)
        .build()
        .parse(expr.trim())
        .ok()
}

/// A backup file's extension, and whether the panel gzips the dump
/// (`mongodump` does it).
fn format_of(engine: Engine) -> Option<(&'static str, bool)> {
    match engine {
        Engine::Redis => None,
        Engine::Mongo => Some(("archive.gz", false)),
        _ => Some(("sql.gz", true)),
    }
}

/// The command dumping `database`.
fn dump_command(engine: Engine, database: &str) -> Option<Vec<String>> {
    let script = match engine {
        Engine::Postgres => {
            r#"PGPASSWORD="$POSTGRES_PASSWORD" exec pg_dump -h 127.0.0.1 -U "$POSTGRES_USER" --clean --if-exists --no-owner -d "$1""#
        }
        Engine::Mysql => {
            r#"exec mysqldump -h 127.0.0.1 -uroot -p"$MYSQL_ROOT_PASSWORD" --single-transaction --routines --triggers --events "$1""#
        }
        Engine::Mariadb => {
            r#"exec mariadb-dump -h 127.0.0.1 -uroot -p"$MARIADB_ROOT_PASSWORD" --single-transaction --routines --triggers --events "$1""#
        }
        Engine::Mongo => {
            r#"exec mongodump --archive --gzip --db="$1" --username="$MONGO_INITDB_ROOT_USERNAME" --password="$MONGO_INITDB_ROOT_PASSWORD" --authenticationDatabase=admin"#
        }
        Engine::Redis => return None,
    };
    Some(sh(script, &[database]))
}

/// The command restoring into `database` (a Mongo archive of `source`
/// renamed to it). PostgreSQL's in one transaction: a restore cut short
/// changes nothing.
fn restore_command(engine: Engine, database: &str, source: Option<&str>) -> Option<Vec<String>> {
    let script = match engine {
        Engine::Postgres => {
            r#"PGPASSWORD="$POSTGRES_PASSWORD" exec psql -q -v ON_ERROR_STOP=1 --single-transaction -h 127.0.0.1 -U "$POSTGRES_USER" -d "$1" -f -"#
        }
        Engine::Mysql => r#"exec mysql -h 127.0.0.1 -uroot -p"$MYSQL_ROOT_PASSWORD" "$1""#,
        Engine::Mariadb => r#"exec mariadb -h 127.0.0.1 -uroot -p"$MARIADB_ROOT_PASSWORD" "$1""#,
        Engine::Mongo => {
            r#"exec mongorestore --archive --gzip --drop --nsFrom="$2.*" --nsTo="$1.*" --username="$MONGO_INITDB_ROOT_USERNAME" --password="$MONGO_INITDB_ROOT_PASSWORD" --authenticationDatabase=admin"#
        }
        Engine::Redis => return None,
    };
    Some(sh(script, &[database, source.unwrap_or(database)]))
}

/// `sh -c script sh args…`: the arguments as `$1`, `$2` (never parsed).
fn sh(script: &str, args: &[&str]) -> Vec<String> {
    let mut cmd = vec![
        "sh".to_owned(),
        "-c".to_owned(),
        script.to_owned(),
        "sh".to_owned(),
    ];
    cmd.extend(args.iter().map(|a| (*a).to_owned()));
    cmd
}

/// `<project>_<service>.<database>.`: what a service's files of a database
/// start with (names have no `.`).
fn file_prefix(owner: &str, database: &str) -> String {
    format!("{owner}.{database}.")
}

/// `<owner>.<database>.<YYYYmmddTHHMMSSZ>.r<run>.<ext>` (the run padded:
/// names sort as they were made).
fn file_name(owner: &str, database: &str, at: DateTime<Utc>, run: i64, ext: &str) -> String {
    format!(
        "{}{}.r{run:010}.{ext}",
        file_prefix(owner, database),
        at.format("%Y%m%dT%H%M%SZ")
    )
}

/// The database a file of [`file_name`] holds.
fn database_of(file: &str) -> Option<&str> {
    let name = file.rsplit('/').next()?;
    let parts: Vec<&str> = name.split('.').collect();
    (parts.len() >= 5 && parts[2].len() == 16 && parts[3].starts_with('r')).then(|| parts[1])
}

async fn owner_of<C: sea_orm::ConnectionTrait>(
    db: &C,
    service_id: i64,
) -> AppResult<(service::Model, String)> {
    let service = service::Entity::find_by_id(service_id)
        .one(db)
        .await?
        .ok_or(AppError::NotFound)?;
    let project = project::Entity::find_by_id(service.project_id)
        .one(db)
        .await?
        .ok_or(AppError::NotFound)?;
    let owner = service_name(&project.name, &service.name);
    Ok((service, owner))
}

/// Asks for a run of `backup` (in `txn`, with its job): its file, provider
/// and retention fixed now (a later edit changes later runs only).
pub async fn request(
    txn: &DatabaseTransaction,
    backup: &database_backup::Model,
    trigger: &str,
    user_id: Option<i64>,
) -> AppResult<backup_run::Model> {
    let (service, owner) = owner_of(txn, backup.service_id).await?;
    let (ext, _) = Engine::parse(&service.kind)
        .and_then(format_of)
        .ok_or_else(|| AppError::BadRequest("this service has no database to back up".into()))?;
    let now = Utc::now();
    let run = backup_run::ActiveModel {
        service_id: Set(backup.service_id),
        backup_id: Set(Some(backup.id)),
        storage_provider_id: Set(Some(backup.storage_provider_id)),
        kind: Set("backup".to_owned()),
        trigger: Set(trigger.to_owned()),
        status: Set("queued".to_owned()),
        database_name: Set(backup.database_name.clone()),
        retention: Set(Some(backup.retention)),
        user_id: Set(user_id),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(txn)
    .await?;
    let key = format!(
        "{}/{}",
        backup.path.trim_matches('/'),
        file_name(&owner, &backup.database_name, now, run.id, ext)
    );
    let mut active = run.into_active_model();
    active.file = Set(Some(key));
    let run = active.update(txn).await?;
    queue::dispatch(txn, &RunBackup { run_id: run.id }).await?;
    Ok(run)
}

/// Asks for a restore of `file` (from `provider`) into `database`.
pub async fn request_restore(
    txn: &DatabaseTransaction,
    service: &service::Model,
    provider: &storage_provider::Model,
    file: &str,
    database: &str,
    user_id: Option<i64>,
) -> AppResult<backup_run::Model> {
    let now = Utc::now();
    let run = backup_run::ActiveModel {
        service_id: Set(service.id),
        storage_provider_id: Set(Some(provider.id)),
        kind: Set("restore".to_owned()),
        trigger: Set("manual".to_owned()),
        status: Set("queued".to_owned()),
        database_name: Set(database.to_owned()),
        file: Set(Some(file.to_owned())),
        user_id: Set(user_id),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(txn)
    .await?;
    queue::dispatch(txn, &RunBackup { run_id: run.id }).await?;
    Ok(run)
}

/// A run's log, saved as it goes.
struct Log<'a> {
    db: &'a DatabaseConnection,
    id: i64,
    text: String,
    saved: std::time::Instant,
}

impl Log<'_> {
    async fn line(&mut self, message: impl AsRef<str>) {
        for line in message.as_ref().lines().filter(|l| !l.trim().is_empty()) {
            self.text.push_str(&format!(
                "{} {}\n",
                Utc::now().format("%H:%M:%S"),
                line.trim_end()
            ));
        }
        if self.text.len() > 2 * crate::models::action::LOG_LIMIT {
            self.text = crate::models::action::tail(&self.text).to_owned();
        }
        if self.saved.elapsed() >= Duration::from_secs(1) {
            self.save().await;
        }
    }

    async fn save(&mut self) {
        self.saved = std::time::Instant::now();
        let _ = backup_run::Entity::update_many()
            .col_expr(
                backup_run::Column::Log,
                Expr::value(crate::models::action::tail(&self.text).to_owned()),
            )
            .filter(backup_run::Column::Id.eq(self.id))
            .exec(self.db)
            .await;
    }
}

/// A private scratch directory (`0700`), removed when dropped.
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl Scratch {
    fn new() -> Result<Self, String> {
        use std::os::unix::fs::DirBuilderExt;
        let dir =
            std::env::temp_dir().join(format!("rdp-backup-{}", random_string(16).to_lowercase()));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&dir)
            .map_err(|e| format!("the scratch directory: {e}"))?;
        Ok(Self(dir))
    }

    fn file(&self) -> PathBuf {
        self.0.join("dump")
    }
}

/// What a finished run reports.
struct Done {
    size: Option<i64>,
}

/// Runs the queued run `id` (a job): once; a run not queued is left alone.
/// One run of a service at a time: another one running fails this one.
pub async fn run(state: &AppState, id: i64) -> AppResult<()> {
    let db = state.db();
    let Some(row) = backup_run::Entity::find_by_id(id).one(db).await? else {
        return Ok(());
    };
    if row.status != "queued" {
        return Ok(());
    }
    // A dead run's command killed before another one is let in.
    recover_interrupted(db, state.service::<Docker>().as_deref()).await?;
    let now = Utc::now();
    let txn = db.begin().await?;
    // The service locked: two runs claim one after the other.
    service::Entity::find_by_id(row.service_id)
        .lock_exclusive()
        .one(&txn)
        .await?;
    let busy = backup_run::Entity::find()
        .filter(backup_run::Column::ServiceId.eq(row.service_id))
        .filter(
            sea_orm::Condition::any()
                .add(backup_run::Column::Status.eq(KILLING))
                .add(
                    sea_orm::Condition::all()
                        .add(backup_run::Column::Status.eq("running"))
                        .add(backup_run::Column::UpdatedAt.gt(now - stale())),
                ),
        )
        .count(&txn)
        .await?
        > 0;
    let claimed = backup_run::Entity::update_many()
        .col_expr(
            backup_run::Column::Status,
            Expr::value(if busy { "failed" } else { "running" }),
        )
        .col_expr(
            backup_run::Column::Error,
            Expr::value(busy.then_some("another backup or restore of this service is running")),
        )
        .col_expr(backup_run::Column::StartedAt, Expr::value(now))
        .col_expr(
            backup_run::Column::FinishedAt,
            Expr::value(busy.then_some(now)),
        )
        .col_expr(backup_run::Column::UpdatedAt, Expr::value(now))
        .filter(backup_run::Column::Id.eq(id))
        .filter(backup_run::Column::Status.eq("queued"))
        .exec(&txn)
        .await?;
    txn.commit().await?;
    if claimed.rows_affected == 0 {
        return Ok(());
    }
    if busy {
        if let Some(done) = backup_run::Entity::find_by_id(id).one(db).await? {
            crate::services::notify_service::backup_finished(state, &done).await;
        }
        return Ok(());
    }
    let mut log = Log {
        db,
        id,
        text: String::new(),
        saved: std::time::Instant::now(),
    };
    // Alive meanwhile (a run whose process died is found: `recover`).
    let result = {
        let work = async {
            match row.kind.as_str() {
                "restore" => restore(state, &row, &mut log).await,
                _ => backup(state, &row, &mut log).await,
            }
        };
        tokio::pin!(work);
        let mut beat = tokio::time::interval(HEARTBEAT);
        beat.tick().await;
        loop {
            tokio::select! {
                result = &mut work => break result,
                _ = beat.tick() => {
                    let beat = backup_run::Entity::update_many()
                        .col_expr(backup_run::Column::UpdatedAt, Expr::value(Utc::now()))
                        .filter(backup_run::Column::Id.eq(id))
                        .filter(backup_run::Column::Status.eq("running"))
                        .exec(db)
                        .await;
                    // Taken for dead meanwhile: stopped (its command killed
                    // as the work is dropped).
                    if beat.is_ok_and(|b| b.rows_affected == 0) {
                        break Err("the run was taken for dead (no heartbeat)".to_owned());
                    }
                }
            }
        }
    };
    let (status, error, size) = match result {
        Ok(done) => ("succeeded", None, done.size),
        Err(error) => {
            log.line(format!("failed: {error}")).await;
            ("failed", Some(error), None)
        }
    };
    let mut active = row.into_active_model();
    active.status = Set(status.to_owned());
    active.error = Set(error);
    active.size = Set(size);
    active.log = Set(Some(crate::models::action::tail(&log.text).to_owned()));
    active.finished_at = Set(Some(Utc::now()));
    active.updated_at = Set(Utc::now());
    let done = active.update(db).await?;
    let notify = state.clone();
    tokio::spawn(async move {
        crate::services::notify_service::backup_finished(&notify, &done).await;
    });
    Ok(())
}

fn stale() -> chrono::Duration {
    chrono::Duration::from_std(STALE).unwrap_or_default()
}

/// The service, its engine, the provider (opened) and the running container.
async fn target(
    state: &AppState,
    run: &backup_run::Model,
    log: &mut Log<'_>,
) -> Result<(Engine, Store, String, String), String> {
    let db = state.db();
    let (service, owner) = owner_of(db, run.service_id)
        .await
        .map_err(|_| "the service is gone")?;
    let engine = Engine::parse(&service.kind)
        .filter(|e| supported(*e))
        .ok_or("this service has no database to back up")?;
    let provider = storage_provider::Entity::find_by_id(run.storage_provider_id.unwrap_or(0))
        .one(db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("the storage provider is gone")?;
    let store = Store::open(&provider, state.keyring())?;
    let docker = state.service::<Docker>().ok_or("Docker is not available")?;
    let tasks = docker
        .engine()
        .list_tasks(&owner)
        .await
        .map_err(|e| format!("the service's tasks: {e}"))?;
    let container = tasks
        .iter()
        .filter(|t| t.status.as_ref().and_then(|s| s.state) == Some(TaskState::RUNNING))
        .find_map(|t| {
            t.status
                .as_ref()?
                .container_status
                .as_ref()?
                .container_id
                .clone()
        })
        .ok_or("the database is not running (deploy it first)")?;
    log.line(format!(
        "{} {owner}, provider {}",
        engine.kind(),
        provider.name
    ))
    .await;
    Ok((engine, store, container, owner))
}

/// A mark for a run's command, recorded with the container before it runs
/// (a run found dead has its command killed: [`recover_interrupted`]).
/// Only while the run is still its own (`running`): one taken for dead
/// meanwhile starts nothing.
async fn marked(db: &DatabaseConnection, run: i64, container: &str) -> Result<String, String> {
    let mark = format!("rdp-backup-{}", random_string(20));
    let owned = backup_run::Entity::update_many()
        .col_expr(backup_run::Column::Container, Expr::value(container))
        .col_expr(backup_run::Column::Mark, Expr::value(mark.as_str()))
        .col_expr(backup_run::Column::UpdatedAt, Expr::value(Utc::now()))
        .filter(backup_run::Column::Id.eq(run))
        .filter(backup_run::Column::Status.eq("running"))
        .exec(db)
        .await
        .map_err(|e| e.to_string())?;
    if owned.rows_affected == 0 {
        return Err("the run was taken for dead (no heartbeat)".to_owned());
    }
    Ok(mark)
}

/// The engines warn about a password on the command line: not worth a line.
fn noise(line: &str) -> bool {
    line.contains("password on the command line")
}

async fn backup(
    state: &AppState,
    run: &backup_run::Model,
    log: &mut Log<'_>,
) -> Result<Done, String> {
    let (engine, store, container, owner) = target(state, run, log).await?;
    let (ext, gzip) = format_of(engine).ok_or("this engine is not backed up")?;
    let cmd = dump_command(engine, &run.database_name).ok_or("this engine is not backed up")?;
    let key = run.file.clone().unwrap_or_default();
    if !valid_key(&key) {
        return Err(format!("{key} is not a valid path"));
    }
    log.line(format!("dumping {} to {key}", run.database_name))
        .await;
    let docker = state.service::<Docker>().ok_or("Docker is not available")?;
    let scratch = Scratch::new()?;
    let path = scratch.file();
    let mut out = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode_private()
        .open(&path)
        .map_err(|e| e.to_string())?;
    let mut encoder =
        gzip.then(|| flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default()));
    let mark = marked(state.db(), run.id, &container).await?;
    let mut chunks = docker
        .engine()
        .run_in_container(&container, cmd, &mark, None)
        .await
        .map_err(|e| format!("the dump could not start: {e}"))?;
    let mut code = None;
    while let Some(chunk) = chunks.next().await {
        match chunk.map_err(|e| format!("the dump: {e}"))? {
            ExecChunk::Out(bytes) => match &mut encoder {
                Some(encoder) => {
                    encoder.write_all(&bytes).map_err(|e| e.to_string())?;
                    let compressed = std::mem::take(encoder.get_mut());
                    out.write_all(&compressed).map_err(|e| e.to_string())?;
                }
                None => out.write_all(&bytes).map_err(|e| e.to_string())?,
            },
            ExecChunk::Err(bytes) => {
                let text = String::from_utf8_lossy(&bytes).into_owned();
                for line in text.lines().filter(|l| !noise(l)) {
                    log.line(line).await;
                }
            }
            ExecChunk::Exit(c) => code = Some(c),
        }
    }
    if let Some(encoder) = encoder {
        let rest = encoder.finish().map_err(|e| e.to_string())?;
        out.write_all(&rest).map_err(|e| e.to_string())?;
    }
    out.sync_all().map_err(|e| e.to_string())?;
    drop(out);
    match code {
        Some(0) => {}
        Some(code) => return Err(format!("the dump exited with {code}")),
        None => return Err("the dump ended without an exit code".to_owned()),
    }
    let size = store.put(&key, &path).await?;
    log.line(format!("uploaded {key} ({} KB)", size / 1024))
        .await;
    // Retention: this service's files of this database in this folder,
    // oldest first. A failure is logged, the backup stands.
    let (folder, _) = key.rsplit_once('/').unwrap_or(("", ""));
    let prefix = file_prefix(&owner, &run.database_name);
    match store.list(folder).await {
        Ok(files) => {
            let mine: Vec<String> = files
                .into_iter()
                .map(|(k, _)| k)
                .filter(|k| {
                    let name = k.rsplit('/').next().unwrap_or_default();
                    name.starts_with(&prefix) && name.ends_with(&format!(".{ext}"))
                })
                .collect();
            let keep = usize::try_from(run.retention.unwrap_or(1).max(1)).unwrap_or(1);
            for old in mine.iter().take(mine.len().saturating_sub(keep)) {
                match store.delete(old).await {
                    Ok(()) => log.line(format!("deleted {old} (retention {keep})")).await,
                    Err(e) => log.line(format!("retention: {e}")).await,
                }
            }
        }
        Err(e) => log.line(format!("retention: {e}")).await,
    }
    Ok(Done {
        size: i64::try_from(size).ok(),
    })
}

/// `0600` for a new file.
trait Private {
    fn mode_private(&mut self) -> &mut Self;
}

impl Private for std::fs::OpenOptions {
    fn mode_private(&mut self) -> &mut Self {
        use std::os::unix::fs::OpenOptionsExt;
        self.mode(0o600)
    }
}

async fn restore(
    state: &AppState,
    run: &backup_run::Model,
    log: &mut Log<'_>,
) -> Result<Done, String> {
    let (engine, store, container, _) = target(state, run, log).await?;
    let key = run.file.clone().unwrap_or_default();
    if !valid_key(&key) {
        return Err(format!("{key} is not a valid path"));
    }
    let gzip = key.ends_with(".sql.gz");
    if !gzip && !(engine == Engine::Mongo && key.ends_with(".archive.gz")) {
        return Err("the file is not a backup of this engine (.sql.gz, .archive.gz)".to_owned());
    }
    let scratch = Scratch::new()?;
    let path = scratch.file();
    let size = store.get(&key, &path).await?;
    log.line(format!("downloaded {key} ({} KB)", size / 1024))
        .await;
    let cmd = restore_command(engine, &run.database_name, database_of(&key))
        .ok_or("this engine is not restored")?;
    // Checked whole before anything reaches the database: a corrupt file
    // changes nothing. SQL is decompressed to a file, Mongo's archive read
    // through once (mongorestore takes it gzipped).
    let source = path.clone();
    let plain = scratch.0.join("restore.sql");
    let target = plain.clone();
    tokio::task::spawn_blocking(move || unpack(&source, gzip.then_some(target.as_path())))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("the file is not a valid gzip archive: {e}"))?;
    let feed_from = if gzip { plain } else { path.clone() };
    // The file as the client's input.
    let (tx, rx) = tokio::sync::mpsc::channel::<Vec<u8>>(8);
    let reader = tokio::task::spawn_blocking(move || feed(&feed_from, &tx));
    let input = futures_util::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|chunk| (chunk, rx))
    })
    .boxed();
    log.line(format!("restoring into {}", run.database_name))
        .await;
    let docker = state.service::<Docker>().ok_or("Docker is not available")?;
    let mark = marked(state.db(), run.id, &container).await?;
    let mut chunks = docker
        .engine()
        .run_in_container(&container, cmd, &mark, Some(input))
        .await
        .map_err(|e| format!("the restore could not start: {e}"))?;
    let mut code = None;
    while let Some(chunk) = chunks.next().await {
        match chunk.map_err(|e| format!("the restore: {e}"))? {
            ExecChunk::Out(bytes) | ExecChunk::Err(bytes) => {
                let text = String::from_utf8_lossy(&bytes).into_owned();
                for line in text.lines().filter(|l| !noise(l)) {
                    log.line(line).await;
                }
            }
            ExecChunk::Exit(c) => code = Some(c),
        }
    }
    reader
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("reading the file: {e}"))?;
    match code {
        Some(0) => {
            log.line("restored").await;
            Ok(Done {
                size: i64::try_from(size).ok(),
            })
        }
        Some(code) => Err(format!("the restore exited with {code}")),
        None => Err("the restore ended without an exit code".to_owned()),
    }
}

/// Reads the gzip file `path` whole (a corrupt one fails here), written
/// decompressed to `to` (`0600`) when given.
fn unpack(path: &Path, to: Option<&Path>) -> std::io::Result<()> {
    let mut reader = flate2::read::MultiGzDecoder::new(std::fs::File::open(path)?);
    match to {
        Some(to) => {
            let mut out = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode_private()
                .open(to)?;
            std::io::copy(&mut reader, &mut out)?;
            out.sync_all()
        }
        None => std::io::copy(&mut reader, &mut std::io::sink()).map(drop),
    }
}

/// Sends `path` in chunks; stops when nobody reads.
fn feed(path: &Path, tx: &tokio::sync::mpsc::Sender<Vec<u8>>) -> std::io::Result<()> {
    use std::io::Read;
    let mut reader = std::fs::File::open(path)?;
    let mut buf = vec![0; 256 * 1024];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 || tx.blocking_send(buf[..n].to_vec()).is_err() {
            return Ok(());
        }
    }
}

/// The scheduler's tick: runs whose process died marked failed, then each
/// enabled configuration whose cron fired since its last slot gets one run
/// (missed slots are not caught up). Claimed with the slot, so two
/// instances never run one twice.
pub async fn tick(state: &AppState) -> AppResult<usize> {
    let db = state.db();
    let n = recover_interrupted(db, state.service::<Docker>().as_deref()).await?;
    if n > 0 {
        tracing::warn!(n, "backups interrupted (their run stopped)");
    }
    let now = Utc::now();
    let mut started = 0;
    let configs = database_backup::Entity::find()
        .filter(database_backup::Column::Enabled.eq(true))
        .all(db)
        .await?;
    for config in configs {
        let Some(cron) = parse_schedule(&config.schedule) else {
            continue;
        };
        let since = config.last_slot_at.unwrap_or(config.created_at);
        let Some(due) = latest_slot(&cron, since, now) else {
            continue;
        };
        let txn = db.begin().await?;
        let mut claim = database_backup::Entity::update_many()
            .col_expr(database_backup::Column::LastSlotAt, Expr::value(due))
            .filter(database_backup::Column::Id.eq(config.id));
        claim = match config.last_slot_at {
            Some(last) => claim.filter(database_backup::Column::LastSlotAt.eq(last)),
            None => claim.filter(database_backup::Column::LastSlotAt.is_null()),
        };
        if claim.exec(&txn).await?.rows_affected == 1 {
            request(&txn, &config, "schedule", None).await?;
            started += 1;
        }
        txn.commit().await?;
    }
    Ok(started)
}

/// The last fire time of `cron` in `(since, now]`.
fn latest_slot(
    cron: &croner::Cron,
    since: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    let mut slot = cron.find_next_occurrence(&since, false).ok()?;
    if slot > now {
        return None;
    }
    // Skipped to the latest (a long stop runs one backup, not many).
    while let Ok(next) = cron.find_next_occurrence(&slot, false) {
        if next > now {
            break;
        }
        slot = next;
    }
    Some(slot)
}

/// Runs whose process stopped (no heartbeat for a while): taken first
/// (`killing`: their runner, if alive, can start nothing more and stops at
/// its next heartbeat), then their command killed in its container and
/// checked dead, then failed. A command that cannot be killed keeps its run
/// `killing`, the service blocked, and is tried again. At boot, every
/// minute, and before a run starts.
pub async fn recover_interrupted(
    db: &DatabaseConnection,
    docker: Option<&Docker>,
) -> AppResult<u64> {
    let now = Utc::now();
    backup_run::Entity::update_many()
        .col_expr(backup_run::Column::Status, Expr::value(KILLING))
        .col_expr(backup_run::Column::UpdatedAt, Expr::value(now))
        .filter(backup_run::Column::Status.eq("running"))
        .filter(backup_run::Column::UpdatedAt.lt(now - stale()))
        .exec(db)
        .await?;
    let taken = backup_run::Entity::find()
        .filter(backup_run::Column::Status.eq(KILLING))
        .all(db)
        .await?;
    let mut failed = 0;
    for run in taken {
        let dead = match (&run.container, &run.mark) {
            (Some(container), Some(mark)) => match docker {
                Some(docker) => match docker.engine().kill_marked(container, mark).await {
                    Ok(()) => true,
                    Err(err) => {
                        tracing::warn!(%err, run = run.id, "a dead run's command not killed yet");
                        false
                    }
                },
                None => false,
            },
            // Nothing was started.
            _ => true,
        };
        if dead {
            failed += backup_run::Entity::update_many()
                .col_expr(backup_run::Column::Status, Expr::value("failed"))
                .col_expr(
                    backup_run::Column::Error,
                    Expr::value("interrupted (its run stopped)"),
                )
                .col_expr(backup_run::Column::FinishedAt, Expr::value(Utc::now()))
                .col_expr(backup_run::Column::UpdatedAt, Expr::value(Utc::now()))
                .filter(backup_run::Column::Id.eq(run.id))
                .filter(backup_run::Column::Status.eq(KILLING))
                .exec(db)
                .await?
                .rows_affected;
        }
    }
    Ok(failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_commands() {
        let at = DateTime::parse_from_rfc3339("2026-10-19T02:00:05Z")
            .unwrap()
            .with_timezone(&Utc);
        let name = file_name("shop_db", "shop", at, 7, "sql.gz");
        assert_eq!(name, "shop_db.shop.20261019T020005Z.r0000000007.sql.gz");
        assert_eq!(database_of(&format!("prod/{name}")), Some("shop"));
        assert_eq!(
            database_of("prod/a_b.my-db.20261019T020005Z.r1.archive.gz"),
            Some("my-db")
        );
        assert_eq!(database_of("prod/notes.txt"), None);
        assert!(valid_database("shop_1") && !valid_database("a b") && !valid_database("--all"));
        // The database is an argument, never part of the script.
        let cmd = dump_command(Engine::Postgres, "x'; rm -rf /").unwrap();
        assert_eq!(cmd[..2], ["sh", "-c"]);
        assert_eq!(cmd[4], "x'; rm -rf /");
        assert!(!cmd[2].contains("rm -rf"));
        assert!(dump_command(Engine::Redis, "x").is_none());
        assert_eq!(format_of(Engine::Mongo), Some(("archive.gz", false)));
        let restore = restore_command(Engine::Mongo, "copy", Some("shop")).unwrap();
        assert_eq!(restore[4..], ["copy", "shop"]);
        assert!(
            restore_command(Engine::Postgres, "x", None).unwrap()[2]
                .contains("--single-transaction")
        );
    }

    #[test]
    fn the_latest_slot_only() {
        let cron = parse_schedule("0 2 * * *").unwrap();
        let at = |s: &str| DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc);
        assert_eq!(
            latest_slot(
                &cron,
                at("2026-10-15T00:00:00Z"),
                at("2026-10-19T03:00:00Z")
            ),
            Some(at("2026-10-19T02:00:00Z"))
        );
        assert_eq!(
            latest_slot(
                &cron,
                at("2026-10-19T02:00:00Z"),
                at("2026-10-19T03:00:00Z")
            ),
            None
        );
        assert!(parse_schedule("not cron").is_none());
        assert!(parse_schedule("*/5 * * * * *").is_none());
    }
}
