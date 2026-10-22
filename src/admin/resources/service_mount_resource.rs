//! A service's storage: a named volume (`rdp_<project>_<service>_<name>`), or
//! a host directory (bind mount). Bind mounts need `system.manage` and a
//! source under one of the configured roots (`[panel] bind_roots`), resolved
//! with its symlinks: never `/`, the Docker socket, `/proc`, the panel's own
//! files.

use rustcms::admin::*;
use rustcms::{AppResult, ValidationErrors, async_trait};
use sea_orm::{ColumnTrait, DatabaseTransaction, EntityTrait, QueryFilter};
use serde_json::Value;

use crate::admin::{int, text};
use crate::models::service_mount;
use crate::policies::permissions;
use crate::services::deploy_rules_service::{
    bind_source, config, sealable, valid_container_path, valid_volume_name,
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
            .navigation(false)
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
        std::sync::Arc::new(crate::policies::child_policy::ChildPolicy("service_mounts"))
    }

    /// A member sees their projects' records only (a role seeing every
    /// record: all of them).
    fn scope(
        query: sea_orm::Select<<Self as Resource>::Entity>,
        ctx: &Ctx,
    ) -> sea_orm::Select<<Self as Resource>::Entity> {
        use sea_orm::{ColumnTrait, QueryFilter};
        match crate::policies::members_only(ctx, "service_mounts") {
            Some(user) => query.filter(
                crate::models::service_mount::Column::ServiceId
                    .in_subquery(crate::services::access_service::member_services(user)),
            ),
            None => query,
        }
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
        crate::policies::write(ctx, txn, &data, None, "service_mounts").await?;
        let data = check(ctx, data, None)?;
        files_fit(ctx, txn, &data, None).await?;
        Ok(data)
    }

    async fn mutate_before_update(
        ctx: &Ctx,
        txn: &DatabaseTransaction,
        record: &service_mount::Model,
        data: Data,
    ) -> AppResult<Data> {
        crate::policies::write(ctx, txn, &data, Some(record.service_id), "service_mounts").await?;
        let data = check(ctx, data, Some(record))?;
        files_fit(ctx, txn, &data, Some(record)).await?;
        Ok(data)
    }
}

/// The file as it will be after this write, with the service's other
/// files: within what a deployment seals together. Checked when its
/// content, path or kind changes; files that cannot be read are left to
/// the deployment to tell (a file can still be fixed).
async fn files_fit(
    ctx: &Ctx,
    txn: &DatabaseTransaction,
    data: &Data,
    record: Option<&service_mount::Model>,
) -> AppResult<()> {
    let kind = text(data, "kind").or(record.map(|r| r.kind.as_str()));
    if kind != Some("file") {
        return Ok(());
    }
    let replaced = data
        .get("sealed_content")
        .and_then(|v| v.get("replace"))
        .and_then(Value::as_str);
    let target = text(data, "target").or(record.map(|r| r.target.as_str()));
    let unchanged = replaced.is_none()
        && record.is_some_and(|r| r.kind == "file" && Some(r.target.as_str()) == target);
    if unchanged {
        return Ok(());
    }
    let keyring = ctx.keyring()?;
    let content = match (replaced, record.and_then(|r| r.sealed_content.as_ref())) {
        (Some(content), _) => content.to_owned(),
        (None, Some(sealed)) => match sealed.open(keyring, &service_mount::content_context()) {
            Ok(content) => content,
            Err(_) => return Ok(()),
        },
        (None, None) => return Ok(()),
    };
    let Some(service) = int(data, "service_id").or(record.map(|r| r.service_id)) else {
        return Ok(());
    };
    let mut others = service_mount::Entity::find()
        .filter(service_mount::Column::ServiceId.eq(service))
        .filter(service_mount::Column::Kind.eq("file"));
    if let Some(record) = record {
        others = others.filter(service_mount::Column::Id.ne(record.id));
    }
    let mut files = std::collections::BTreeMap::new();
    for other in others.all(txn).await? {
        let Some(sealed) = &other.sealed_content else {
            continue;
        };
        match sealed.open(keyring, &service_mount::content_context()) {
            Ok(content) => files.insert(other.target, content),
            Err(_) => return Ok(()),
        };
    }
    files.insert(target.unwrap_or_default().to_owned(), content);
    if sealable(&files) {
        Ok(())
    } else {
        Err(ValidationErrors::field(
            "sealed_content",
            ctx.t("the service's files are at most 32 KiB together"),
        )
        .into())
    }
}

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
            if !ctx.can(permissions::SYSTEM_MANAGE) {
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
            } else if replaced.as_deref().is_some_and(|c| !sealable(&c)) {
                errors.add("sealed_content", ctx.t("is at most 32 KiB"));
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
