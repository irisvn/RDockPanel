//! Database backups (as EasyPanel's): a database service's dump, taken in its
//! running container with the engine's own tool (`pg_dump`, `mysqldump`,
//! `mariadb-dump`, `mongodump`) and the credentials of its environment (never
//! passed by the panel), gzipped, written to a storage provider as
//! `<path>/<database>-<UTC time>.<ext>`; the oldest files of that database in
//! that path deleted beyond the retention. A restore feeds a file back to
//! the engine's client, in place. Each run is a `backup_runs` row with its
//! log; scheduled runs come from each configuration's cron ([`tick`]).

use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use rustcms::queue;
use rustcms::{AppError, AppResult, AppState};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, DatabaseTransaction, EntityTrait,
    IntoActiveModel, QueryFilter, Set, TransactionTrait,
};

use crate::jobs::run_backup::RunBackup;
use crate::models::{backup_run, database_backup, project, service, storage_provider};
use crate::services::database_service::{Engine, random_string};
use crate::services::deploy_service::service_name;
use crate::services::docker_service::{Docker, ExecChunk, TaskState};
use crate::services::storage_service::{Store, valid_key};

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

/// The command dumping `database`, the file's extension, and whether the
/// panel gzips its output (`mongodump` does it).
fn dump_command(engine: Engine, database: &str) -> Option<(Vec<String>, &'static str, bool)> {
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
    let (ext, gzip) = match engine {
        Engine::Mongo => ("archive.gz", false),
        _ => ("sql.gz", true),
    };
    Some((sh(script, &[database]), ext, gzip))
}

/// The command restoring into `database` (a Mongo archive of `source`
/// renamed to it).
fn restore_command(engine: Engine, database: &str, source: Option<&str>) -> Option<Vec<String>> {
    let script = match engine {
        Engine::Postgres => {
            r#"PGPASSWORD="$POSTGRES_PASSWORD" exec psql -q -v ON_ERROR_STOP=1 -h 127.0.0.1 -U "$POSTGRES_USER" -d "$1""#
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

/// `<database>-<YYYYmmddTHHMMSSZ>.<ext>`.
fn file_name(database: &str, ext: &str, at: DateTime<Utc>) -> String {
    format!("{database}-{}.{ext}", at.format("%Y%m%dT%H%M%SZ"))
}

/// The database a file of [`file_name`] holds.
fn database_of(file: &str) -> Option<&str> {
    let name = file.rsplit('/').next()?;
    let (stem, _) = name.split_once('.')?;
    let (database, time) = stem.rsplit_once('-')?;
    (time.len() == 16 && time.ends_with('Z')).then_some(database)
}

/// Asks for a run of `backup` (in `txn`, with its job).
pub async fn request(
    txn: &DatabaseTransaction,
    backup: &database_backup::Model,
    trigger: &str,
    user_id: Option<i64>,
) -> AppResult<backup_run::Model> {
    let now = Utc::now();
    let run = backup_run::ActiveModel {
        service_id: Set(backup.service_id),
        backup_id: Set(Some(backup.id)),
        storage_provider_id: Set(Some(backup.storage_provider_id)),
        kind: Set("backup".to_owned()),
        trigger: Set(trigger.to_owned()),
        status: Set("queued".to_owned()),
        database_name: Set(backup.database_name.clone()),
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
        if self.saved.elapsed() >= std::time::Duration::from_secs(1) {
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

/// A temporary file, removed when dropped.
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn scratch() -> Scratch {
    Scratch(std::env::temp_dir().join(format!("rdp-backup-{}", random_string(16).to_lowercase())))
}

/// What a finished run reports.
struct Done {
    file: Option<String>,
    size: Option<i64>,
}

/// Runs the queued run `id` (a job): once; a run not queued is left alone.
pub async fn run(state: &AppState, id: i64) -> AppResult<()> {
    let db = state.db();
    let now = Utc::now();
    let claimed = backup_run::Entity::update_many()
        .col_expr(backup_run::Column::Status, Expr::value("running"))
        .col_expr(backup_run::Column::StartedAt, Expr::value(now))
        .col_expr(backup_run::Column::UpdatedAt, Expr::value(now))
        .filter(backup_run::Column::Id.eq(id))
        .filter(backup_run::Column::Status.eq("queued"))
        .exec(db)
        .await?;
    if claimed.rows_affected == 0 {
        return Ok(());
    }
    let row = backup_run::Entity::find_by_id(id)
        .one(db)
        .await?
        .ok_or(AppError::NotFound)?;
    let mut log = Log {
        db,
        id,
        text: String::new(),
        saved: std::time::Instant::now(),
    };
    let result = match row.kind.as_str() {
        "restore" => restore(state, &row, &mut log).await,
        _ => backup(state, &row, &mut log).await,
    };
    let (status, error, done) = match result {
        Ok(done) => ("succeeded", None, done),
        Err(error) => {
            log.line(format!("failed: {error}")).await;
            (
                "failed",
                Some(error),
                Done {
                    file: row.file.clone(),
                    size: None,
                },
            )
        }
    };
    let mut active = row.into_active_model();
    active.status = Set(status.to_owned());
    active.error = Set(error);
    active.file = Set(done.file);
    active.size = Set(done.size);
    active.log = Set(Some(crate::models::action::tail(&log.text).to_owned()));
    active.finished_at = Set(Some(Utc::now()));
    active.updated_at = Set(Utc::now());
    active.update(db).await?;
    Ok(())
}

/// The service, its engine, the provider (opened) and the running container.
async fn target(
    state: &AppState,
    run: &backup_run::Model,
    log: &mut Log<'_>,
) -> Result<(Engine, Store, String), String> {
    let db = state.db();
    let service = service::Entity::find_by_id(run.service_id)
        .one(db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("the service is gone")?;
    let engine = Engine::parse(&service.kind)
        .filter(|e| supported(*e))
        .ok_or("this service has no database to back up")?;
    let project = project::Entity::find_by_id(service.project_id)
        .one(db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("the project is gone")?;
    let provider = storage_provider::Entity::find_by_id(run.storage_provider_id.unwrap_or(0))
        .one(db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("the storage provider is gone")?;
    let store = Store::open(&provider, state.keyring())?;
    let docker = state.service::<Docker>().ok_or("Docker is not available")?;
    let name = service_name(&project.name, &service.name);
    let tasks = docker
        .engine()
        .list_tasks(&name)
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
        "{} {name}, provider {}",
        engine.kind(),
        provider.name
    ))
    .await;
    Ok((engine, store, container))
}

async fn backup(
    state: &AppState,
    run: &backup_run::Model,
    log: &mut Log<'_>,
) -> Result<Done, String> {
    let db = state.db();
    let config = database_backup::Entity::find_by_id(run.backup_id.unwrap_or(0))
        .one(db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("the backup configuration is gone")?;
    let (engine, store, container) = target(state, run, log).await?;
    let (cmd, ext, gzip) =
        dump_command(engine, &run.database_name).ok_or("this engine is not backed up")?;
    let key = format!(
        "{}/{}",
        config.path.trim_matches('/'),
        file_name(&run.database_name, ext, Utc::now())
    );
    if !valid_key(&key) {
        return Err(format!("{key} is not a valid path"));
    }
    log.line(format!("dumping {} to {key}", run.database_name))
        .await;
    let docker = state.service::<Docker>().ok_or("Docker is not available")?;
    let file = scratch();
    let mut out = std::fs::File::create(&file.0).map_err(|e| e.to_string())?;
    let mut encoder =
        gzip.then(|| flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default()));
    let mut chunks = docker
        .engine()
        .run_in_container(&container, cmd, None)
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
                let text = String::from_utf8_lossy(&bytes);
                // The engines warn about a password on the command line.
                for line in text
                    .lines()
                    .filter(|l| !l.contains("password on the command line"))
                {
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
    let size = store.put(&key, &file.0).await?;
    log.line(format!("uploaded {key} ({} KB)", size / 1024))
        .await;
    // Retention: this database's files in this path, oldest first. A failure
    // is logged, the backup stands.
    let folder = config.path.trim_matches('/');
    match store.list(folder).await {
        Ok(files) => {
            let mine: Vec<String> = files
                .into_iter()
                .map(|(k, _)| k)
                .filter(|k| {
                    k.ends_with(&format!(".{ext}"))
                        && database_of(k) == Some(run.database_name.as_str())
                })
                .collect();
            let keep = usize::try_from(config.retention.max(1)).unwrap_or(1);
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
        file: Some(key),
        size: i64::try_from(size).ok(),
    })
}

async fn restore(
    state: &AppState,
    run: &backup_run::Model,
    log: &mut Log<'_>,
) -> Result<Done, String> {
    let (engine, store, container) = target(state, run, log).await?;
    let key = run.file.clone().unwrap_or_default();
    if !valid_key(&key) {
        return Err(format!("{key} is not a valid path"));
    }
    let gzip = key.ends_with(".sql.gz");
    if !gzip && !(engine == Engine::Mongo && key.ends_with(".archive.gz")) {
        return Err("the file is not a backup of this engine (.sql.gz, .archive.gz)".to_owned());
    }
    let file = scratch();
    let size = store.get(&key, &file.0).await?;
    log.line(format!("downloaded {key} ({} KB)", size / 1024))
        .await;
    let cmd = restore_command(engine, &run.database_name, database_of(&key))
        .ok_or("this engine is not restored")?;
    // The file, decompressed when the panel compressed it, as the client's input.
    let (tx, rx) = tokio::sync::mpsc::channel::<Vec<u8>>(8);
    let path: PathBuf = file.0.clone();
    let reader = tokio::task::spawn_blocking(move || feed(&path, gzip, &tx));
    let input = futures_util::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|chunk| (chunk, rx))
    })
    .boxed();
    log.line(format!("restoring into {}", run.database_name))
        .await;
    let docker = state.service::<Docker>().ok_or("Docker is not available")?;
    let mut chunks = docker
        .engine()
        .run_in_container(&container, cmd, Some(input))
        .await
        .map_err(|e| format!("the restore could not start: {e}"))?;
    let mut code = None;
    while let Some(chunk) = chunks.next().await {
        match chunk.map_err(|e| format!("the restore: {e}"))? {
            ExecChunk::Out(bytes) | ExecChunk::Err(bytes) => {
                let text = String::from_utf8_lossy(&bytes).into_owned();
                for line in text
                    .lines()
                    .filter(|l| !l.contains("password on the command line"))
                {
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
                file: Some(key),
                size: i64::try_from(size).ok(),
            })
        }
        Some(code) => Err(format!("the restore exited with {code}")),
        None => Err("the restore ended without an exit code".to_owned()),
    }
}

/// Sends `path` (gunzipped when `gzip`) in chunks; stops when nobody reads.
fn feed(path: &Path, gzip: bool, tx: &tokio::sync::mpsc::Sender<Vec<u8>>) -> std::io::Result<()> {
    use std::io::Read;
    let file = std::fs::File::open(path)?;
    let mut reader: Box<dyn Read> = if gzip {
        Box::new(flate2::read::GzDecoder::new(file))
    } else {
        Box::new(file)
    };
    let mut buf = vec![0; 256 * 1024];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 || tx.blocking_send(buf[..n].to_vec()).is_err() {
            return Ok(());
        }
    }
}

/// The scheduler's tick: each enabled configuration whose cron fired since
/// its last slot gets one run (missed slots are not caught up). Claimed with
/// the slot, so two instances never run one twice.
pub async fn tick(state: &AppState) -> AppResult<usize> {
    let db = state.db();
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

/// Runs cut by a stop (still `running` at boot): failed.
pub async fn recover_interrupted(db: &DatabaseConnection) -> AppResult<u64> {
    let now = Utc::now();
    Ok(backup_run::Entity::update_many()
        .col_expr(backup_run::Column::Status, Expr::value("failed"))
        .col_expr(
            backup_run::Column::Error,
            Expr::value("interrupted (the panel stopped)"),
        )
        .col_expr(backup_run::Column::FinishedAt, Expr::value(now))
        .col_expr(backup_run::Column::UpdatedAt, Expr::value(now))
        .filter(backup_run::Column::Status.eq("running"))
        .exec(db)
        .await?
        .rows_affected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_commands() {
        let at = DateTime::parse_from_rfc3339("2026-10-19T02:00:05Z")
            .unwrap()
            .with_timezone(&Utc);
        let name = file_name("shop", "sql.gz", at);
        assert_eq!(name, "shop-20261019T020005Z.sql.gz");
        assert_eq!(database_of(&format!("prod/{name}")), Some("shop"));
        assert_eq!(
            database_of("prod/my-db-20261019T020005Z.archive.gz"),
            Some("my-db")
        );
        assert_eq!(database_of("prod/notes.txt"), None);
        assert!(valid_database("shop_1") && !valid_database("a b") && !valid_database("--all"));
        // The database is an argument, never part of the script.
        let (cmd, ext, gzip) = dump_command(Engine::Postgres, "x'; rm -rf /").unwrap();
        assert_eq!((ext, gzip), ("sql.gz", true));
        assert_eq!(cmd[..2], ["sh", "-c"]);
        assert_eq!(cmd[4], "x'; rm -rf /");
        assert!(!cmd[2].contains("rm -rf"));
        assert!(dump_command(Engine::Redis, "x").is_none());
        let restore = restore_command(Engine::Mongo, "copy", Some("shop")).unwrap();
        assert_eq!(restore[4..], ["copy", "shop"]);
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
