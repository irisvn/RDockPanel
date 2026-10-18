//! A service's redirects (its Redirects tab): a regular expression over the
//! request's URL (`^https?://shop.example.com/(.*)`) and its replacement
//! (`https://www.shop.example.com/${1}`), as Traefik's `redirectRegex`.
//! Applied on the next deployment.

use rustcms::admin::*;
use rustcms::{AppResult, ValidationErrors, async_trait};
use sea_orm::DatabaseTransaction;

use crate::admin::text;
use crate::models::service_redirect;
use crate::services::deploy_service::check_redirect;

pub struct ServiceRedirectResource;

#[async_trait]
impl Resource for ServiceRedirectResource {
    type Entity = service_redirect::Entity;
    const SLUG: &'static str = "service_redirects";

    fn meta() -> ResourceMeta {
        ResourceMeta::new("Redirect")
            .plural("Redirects")
            .title("regex")
            .icon("tag")
            .group("Deploy")
            .lock_parent("service_id", "services")
    }

    fn fields() -> Vec<Field> {
        vec![
            Field::column(service_redirect::Column::ServiceId)
                .belongs_to("services")
                .required()
                .filterable(),
            Field::column(service_redirect::Column::Regex)
                .required()
                .sortable()
                .max_length(500),
            Field::column(service_redirect::Column::Replacement)
                .required()
                .max_length(500),
            Field::column(service_redirect::Column::Permanent),
            Field::column(service_redirect::Column::Enabled),
        ]
    }

    fn form() -> Form {
        Form::new().columns(2).schema(components![
            SelectInput::make("service_id"),
            TextInput::make("regex").help("e.g. ^https?://example.com/(.*)"),
            TextInput::make("replacement").help("e.g. https://www.example.com/${1}"),
            Toggle::make("permanent"),
            Toggle::make("enabled"),
        ])
    }

    fn table() -> Table {
        Table::new()
            .columns(vec![
                TextColumn::make("regex"),
                TextColumn::make("replacement"),
                TextColumn::make("service_id"),
            ])
            .default_sort("regex")
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
        _record: &service_redirect::Model,
        data: Data,
    ) -> AppResult<Data> {
        check(ctx, &data)?;
        Ok(data)
    }
}

fn check(ctx: &Ctx, data: &Data) -> AppResult<()> {
    let mut errors = ValidationErrors::new();
    if let Some(regex) = text(data, "regex")
        && let Err(e) = check_redirect(regex, "x")
    {
        errors.add("regex", ctx.t(e));
    }
    if let Some(replacement) = text(data, "replacement")
        && let Err(e) = check_redirect("x", replacement)
    {
        errors.add("replacement", ctx.t(e));
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.into())
    }
}
