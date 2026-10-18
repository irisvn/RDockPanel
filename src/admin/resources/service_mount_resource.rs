//! A service's storage: a named volume (`rdp_<project>_<service>_<name>`), or
//! a host directory (bind mount). Bind mounts need `system.manage` and a
//! source under one of the configured roots (`[panel] bind_roots`), resolved
//! with its symlinks: never `/`, the Docker socket, `/proc`, the panel's own
//! files.

use rustcms::admin::*;
use rustcms::{AppResult, ValidationErrors, async_trait};
use sea_orm::DatabaseTransaction;
use serde_json::Value;

use crate::admin::text;
use crate::http::controllers::docker_controller::SYSTEM_PERMISSION;
use crate::models::service_mount;
use crate::services::deploy_rules_service::{
    bind_source, config, valid_container_path, valid_volume_name,
};

pub struct ServiceMountResource;

#[async_trait]
impl Resource for ServiceMountResource {
    type Entity = service_mount::Entity;
    const SLUG: &'static str = "service_mounts";

    fn meta() -> ResourceMeta {
        ResourceMeta::new("Mount")
            .plural("Mounts")
            .title("target")
            .icon("folder")
            .group("Deploy")
            .lock_parent("service_id", "services")
            .unique(
                "service_mounts_service_target_unique",
                &["service_id", "target"],
                "is already mounted",
            )
    }

    fn fields() -> Vec<Field> {
        vec![
            Field::column(service_mount::Column::ServiceId)
                .belongs_to("services")
                .required()
                .filterable(),
            Field::column(service_mount::Column::Kind).required(),
            Field::column(service_mount::Column::Name).max_length(40),
            Field::column(service_mount::Column::Source).max_length(512),
            Field::column(service_mount::Column::Target)
                .required()
                .sortable()
                .max_length(512),
            Field::column(service_mount::Column::ReadOnly),
            // Sealed with APP_KEY: templates put passwords in files.
            Field::column(service_mount::Column::SealedContent).encrypted(),
        ]
    }

    fn form() -> Form {
        Form::new().columns(2).schema(components![
            SelectInput::make("service_id"),
            SelectInput::make("kind").options([
                ("volume", "Volume"),
                ("bind", "Host directory (bind)"),
                ("file", "File"),
            ]),
            TextInput::make("name").help("Volume: its name, e.g. data"),
            TextInput::make("source").help("Bind: a host directory under an allowed root"),
            TextInput::make("target").help("Path inside the container, e.g. /var/lib/app"),
            Toggle::make("read_only"),
            SecretInput::make("sealed_content")
                .column_span(2)
                .help("File: its content (sealed; replaced, never shown)"),
        ])
    }

    fn policy() -> std::sync::Arc<dyn Policy> {
        std::sync::Arc::new(MountPolicy)
    }

    fn table() -> Table {
        Table::new()
            .columns(vec![
                TextColumn::make("kind"),
                TextColumn::make("name"),
                TextColumn::make("source"),
                TextColumn::make("target"),
                BooleanColumn::make("read_only"),
            ])
            .default_sort("target")
    }

    async fn mutate_before_create(
        ctx: &Ctx,
        txn: &DatabaseTransaction,
        data: Data,
    ) -> AppResult<Data> {
        privileged_parent(ctx, txn, crate::admin::int(&data, "service_id")).await?;
        check(ctx, data, None)
    }

    async fn mutate_before_update(
        ctx: &Ctx,
        txn: &DatabaseTransaction,
        record: &service_mount::Model,
        data: Data,
    ) -> AppResult<Data> {
        privileged_parent(ctx, txn, Some(record.service_id)).await?;
        // Never moved to another service (whose own rules it would skip).
        if crate::admin::int(&data, "service_id").is_some_and(|id| id != record.service_id) {
            return Err(
                rustcms::ValidationErrors::field("service_id", ctx.t("cannot change")).into(),
            );
        }
        check(ctx, data, Some(record))
    }
}

/// The largest file content, in bytes: what a MySQL `TEXT` holds (a Swarm
/// config takes more; such files are small).
const MAX_FILE: usize = 65_535;

/// Checks the mount as it will be after this write; a bind's source is
/// stored resolved.
fn check(ctx: &Ctx, mut data: Data, record: Option<&service_mount::Model>) -> AppResult<Data> {
    let after = |field: &str, old: Option<&str>| -> Option<String> {
        if data.contains_key(field) {
            text(&data, field).map(str::to_owned)
        } else {
            old.map(str::to_owned)
        }
    };
    let kind = after("kind", record.map(|r| r.kind.as_str())).unwrap_or_default();
    let name = after("name", record.and_then(|r| r.name.as_deref())).filter(|s| !s.is_empty());
    let source =
        after("source", record.and_then(|r| r.source.as_deref())).filter(|s| !s.is_empty());
    let target = after("target", record.map(|r| r.target.as_str()));
    // A file's content: replaced now (`{"replace": …}`), or kept.
    let replaced = data
        .get("sealed_content")
        .and_then(|v| v.get("replace"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let cleared = data
        .get("sealed_content")
        .and_then(|v| v.get("clear"))
        .is_some_and(|v| v == &Value::Bool(true));
    let has_content =
        replaced.is_some() || (!cleared && record.is_some_and(|r| r.sealed_content.is_some()));
    let mut errors = ValidationErrors::new();
    match kind.as_str() {
        "volume" => {
            if !name.as_deref().is_some_and(valid_volume_name) {
                errors.add(
                    "name",
                    ctx.t("must be 1-40 lowercase letters, digits, - and _"),
                );
            }
            if source.is_some() {
                errors.add("source", ctx.t("is only for host directories"));
            }
        }
        "bind" => {
            if !ctx.can(SYSTEM_PERMISSION) {
                errors.add(
                    "kind",
                    ctx.t("host directories need the system.manage permission"),
                );
            } else if name.is_some() {
                errors.add("name", ctx.t("is only for volumes"));
            } else {
                match source
                    .as_deref()
                    .map(|s| bind_source(s, &config().bind_roots))
                {
                    None => errors.add("source", ctx.t("is required")),
                    Some(Err(refusal)) => errors.add("source", ctx.t(refusal.message())),
                    Some(Ok(resolved)) => {
                        data.insert(
                            "source".to_owned(),
                            Value::String(resolved.to_string_lossy().into_owned()),
                        );
                    }
                }
            }
        }
        "file" => {
            if name.is_some() {
                errors.add("name", ctx.t("is only for volumes"));
            }
            if source.is_some() {
                errors.add("source", ctx.t("is only for host directories"));
            }
            if !has_content {
                errors.add("sealed_content", ctx.t("is required"));
            } else if replaced.as_deref().is_some_and(|c| c.len() > MAX_FILE) {
                errors.add("sealed_content", ctx.t("is at most 64 KB"));
            }
        }
        _ => errors.add("kind", ctx.t("must be volume, bind or file")),
    }
    if !target.as_deref().is_some_and(valid_container_path) {
        errors.add("target", ctx.t("must be an absolute path other than /"));
    }
    if errors.is_empty() {
        Ok(data)
    } else {
        Err(errors.into())
    }
}

/// The mounts of a service with added capabilities or sysctls: as the
/// service itself, its administrators' only.
async fn privileged_parent(
    ctx: &Ctx,
    txn: &DatabaseTransaction,
    service_id: Option<i64>,
) -> AppResult<()> {
    use sea_orm::EntityTrait;
    let Some(id) = service_id else {
        return Ok(());
    };
    let privileged = crate::models::service::Entity::find_by_id(id)
        .one(txn)
        .await?
        .is_some_and(|s| crate::admin::resources::service_resource::privileged(&s));
    if privileged && !ctx.can(crate::http::controllers::docker_controller::SYSTEM_PERMISSION) {
        return Err(rustcms::AppError::Forbidden);
    }
    Ok(())
}

/// The permissions, and a privileged service's mounts deleted by its
/// administrators only.
struct MountPolicy;

#[async_trait]
impl Policy for MountPolicy {
    async fn allows(&self, ctx: &Ctx, ability: &str, record: Option<&Data>) -> bool {
        use sea_orm::EntityTrait;
        if !ctx.can(&format!("service_mounts.{ability}")) {
            return false;
        }
        if matches!(ability, "view" | "view_any")
            || ctx.can(crate::http::controllers::docker_controller::SYSTEM_PERMISSION)
        {
            return true;
        }
        let Some(id) = record.and_then(|r| crate::admin::int(r, "service_id")) else {
            return true;
        };
        crate::models::service::Entity::find_by_id(id)
            .one(&ctx.conn())
            .await
            .is_ok_and(|s| {
                !s.is_some_and(|s| crate::admin::resources::service_resource::privileged(&s))
            })
    }

    async fn admits(&self, ctx: &Ctx, ability: &str) -> bool {
        ctx.can(&format!("service_mounts.{ability}"))
    }

    fn view_depends_on_record(&self) -> bool {
        false
    }
}
