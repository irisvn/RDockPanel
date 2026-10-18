//! A service's published ports (raw TCP/UDP, for what is not HTTP: HTTP goes
//! through domains). One service per port and protocol; the proxy's and the
//! panel's ports are reserved.

use rustcms::admin::*;
use rustcms::{AppResult, ValidationErrors, async_trait};
use sea_orm::DatabaseTransaction;

use crate::admin::{int, is_set, text};
use crate::models::service_port;
use crate::services::deploy_rules_service::{config, valid_port};

pub struct ServicePortResource;

#[async_trait]
impl Resource for ServicePortResource {
    type Entity = service_port::Entity;
    const SLUG: &'static str = "service_ports";

    fn meta() -> ResourceMeta {
        ResourceMeta::new("Port")
            .plural("Ports")
            .title("published")
            .icon("layers")
            .group("Deploy")
            .lock_parent("service_id", "services")
            .unique(
                "service_ports_published_protocol_unique",
                &["published", "protocol"],
                "is already published",
            )
    }

    fn fields() -> Vec<Field> {
        vec![
            Field::column(service_port::Column::ServiceId)
                .belongs_to("services")
                .required()
                .filterable(),
            Field::column(service_port::Column::Published)
                .required()
                .sortable(),
            Field::column(service_port::Column::Target).required(),
            Field::column(service_port::Column::Protocol).required(),
            Field::column(service_port::Column::Mode).required(),
        ]
    }

    fn form() -> Form {
        Form::new().columns(2).schema(components![
            SelectInput::make("service_id"),
            NumberInput::make("published").help("The port on the server"),
            NumberInput::make("target").help("The port inside the container"),
            SelectInput::make("protocol").options([("tcp", "TCP"), ("udp", "UDP")]),
            SelectInput::make("mode").options([
                ("ingress", "Every node (Swarm routing mesh)"),
                ("host", "Only the node running the task"),
            ]),
        ])
    }

    fn table() -> Table {
        Table::new()
            .columns(vec![
                TextColumn::make("published"),
                TextColumn::make("target"),
                TextColumn::make("protocol"),
                TextColumn::make("mode"),
                TextColumn::make("service_id"),
            ])
            .default_sort("published")
    }

    async fn mutate_before_create(
        ctx: &Ctx,
        _txn: &DatabaseTransaction,
        data: Data,
    ) -> AppResult<Data> {
        check(ctx, &data)?;
        Ok(data)
    }

    async fn mutate_before_update(
        ctx: &Ctx,
        _txn: &DatabaseTransaction,
        _record: &service_port::Model,
        data: Data,
    ) -> AppResult<Data> {
        check(ctx, &data)?;
        Ok(data)
    }
}

fn check(ctx: &Ctx, data: &Data) -> AppResult<()> {
    let mut errors = ValidationErrors::new();
    if is_set(data, "published") {
        match int(data, "published") {
            Some(port) if !valid_port(port) => {
                errors.add("published", ctx.t("must be between 1 and 65535"))
            }
            Some(port)
                if config()
                    .reserved_ports
                    .iter()
                    .any(|r| i64::from(*r) == port) =>
            {
                errors.add(
                    "published",
                    ctx.t("is reserved for the panel and its proxy"),
                )
            }
            Some(_) => {}
            None => errors.add("published", ctx.t("must be between 1 and 65535")),
        }
    }
    if is_set(data, "target") && !int(data, "target").is_some_and(valid_port) {
        errors.add("target", ctx.t("must be between 1 and 65535"));
    }
    if text(data, "protocol").is_some_and(|p| !matches!(p, "tcp" | "udp")) {
        errors.add("protocol", ctx.t("must be tcp or udp"));
    }
    if text(data, "mode").is_some_and(|m| !matches!(m, "ingress" | "host")) {
        errors.add("mode", ctx.t("must be ingress or host"));
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.into())
    }
}
