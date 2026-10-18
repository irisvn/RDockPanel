//! A service's domains: Traefik routes `host` + `path` to `port` of the
//! service (HTTPS with a certificate when `https`). One service per host and
//! path; applied on the next deployment.

use rustcms::admin::*;
use rustcms::{AppResult, ValidationErrors, async_trait};
use sea_orm::DatabaseTransaction;
use serde_json::Value;

use crate::admin::{int, is_set, text};
use crate::models::service_domain;
use crate::services::deploy_rules_service::{valid_hostname, valid_port, valid_route_path};

pub struct ServiceDomainResource;

#[async_trait]
impl Resource for ServiceDomainResource {
    type Entity = service_domain::Entity;
    const SLUG: &'static str = "service_domains";

    fn meta() -> ResourceMeta {
        ResourceMeta::new("Domain")
            .plural("Domains")
            .title("host")
            .icon("tag")
            .group("Deploy")
            .lock_parent("service_id", "services")
            .unique(
                "service_domains_host_path_unique",
                &["host", "path"],
                "is already routed to a service",
            )
    }

    fn fields() -> Vec<Field> {
        vec![
            Field::column(service_domain::Column::ServiceId)
                .belongs_to("services")
                .required()
                .filterable(),
            Field::column(service_domain::Column::Host)
                .required()
                .searchable()
                .sortable()
                .max_length(253),
            Field::column(service_domain::Column::Path)
                .required()
                .max_length(255),
            Field::column(service_domain::Column::Port).required(),
            Field::column(service_domain::Column::Https),
            Field::column(service_domain::Column::ComposeService).max_length(64),
        ]
    }

    fn form() -> Form {
        Form::new().columns(2).schema(components![
            SelectInput::make("service_id"),
            TextInput::make("host")
                .help("e.g. app.example.com (its DNS must point to this server)"),
            TextInput::make("path").help("/ for the whole site, or a prefix like /api"),
            NumberInput::make("port").help("The port the app listens on inside its container"),
            Toggle::make("https").help("Redirect HTTP to HTTPS and get a certificate"),
        ])
    }

    fn table() -> Table {
        Table::new()
            .columns(vec![
                TextColumn::make("host"),
                TextColumn::make("path"),
                TextColumn::make("port"),
                BooleanColumn::make("https"),
                TextColumn::make("service_id"),
            ])
            .default_sort("host")
    }

    async fn mutate_before_create(
        ctx: &Ctx,
        txn: &DatabaseTransaction,
        data: Data,
    ) -> AppResult<Data> {
        let data = check(ctx, data)?;
        check_container(ctx, txn, &data, None).await?;
        Ok(data)
    }

    async fn mutate_before_update(
        ctx: &Ctx,
        txn: &DatabaseTransaction,
        record: &service_domain::Model,
        data: Data,
    ) -> AppResult<Data> {
        let data = check(ctx, data)?;
        check_container(ctx, txn, &data, Some(record)).await?;
        Ok(data)
    }
}

/// A Compose service's domain leads to one of its containers (named); any
/// other service's to the service itself (none named).
async fn check_container(
    ctx: &Ctx,
    txn: &DatabaseTransaction,
    data: &Data,
    record: Option<&service_domain::Model>,
) -> AppResult<()> {
    use sea_orm::EntityTrait;
    let service_id = int(data, "service_id").or(record.map(|r| r.service_id));
    let Some(service_id) = service_id else {
        return Ok(());
    };
    let compose = crate::models::service::Entity::find_by_id(service_id)
        .one(txn)
        .await?
        .is_some_and(|s| s.kind == "compose");
    let container = if data.contains_key("compose_service") {
        text(data, "compose_service")
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .map(str::to_owned)
    } else {
        record.and_then(|r| r.compose_service.clone())
    };
    let error = match (compose, container) {
        (true, None) => Some("names the Compose file's service it leads to"),
        (true, Some(c)) if !crate::services::deploy_service::valid_compose_service(&c) => {
            Some("is not a service name of a Compose file")
        }
        (false, Some(_)) => Some("is only for a Compose service"),
        _ => None,
    };
    match error {
        Some(error) => Err(ValidationErrors::field("compose_service", ctx.t(error)).into()),
        None => Ok(()),
    }
}

/// Hosts are stored lowercase; then each value is checked.
fn check(ctx: &Ctx, mut data: Data) -> AppResult<Data> {
    if let Some(host) = text(&data, "host").map(|h| h.trim().to_ascii_lowercase()) {
        data.insert("host".to_owned(), Value::String(host));
    }
    let mut errors = ValidationErrors::new();
    if let Some(host) = text(&data, "host")
        && !valid_hostname(host)
    {
        errors.add("host", ctx.t("is not a host name like app.example.com"));
    }
    if let Some(path) = text(&data, "path")
        && !valid_route_path(path)
    {
        errors.add(
            "path",
            ctx.t("must start with / and use letters, digits, - _ . ~"),
        );
    }
    if is_set(&data, "port") && !int(&data, "port").is_some_and(valid_port) {
        errors.add("port", ctx.t("must be between 1 and 65535"));
    }
    if errors.is_empty() {
        Ok(data)
    } else {
        Err(errors.into())
    }
}
