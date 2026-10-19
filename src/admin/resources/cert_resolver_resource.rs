//! Certificate resolvers (Settings → Certificates): Let's Encrypt over HTTP,
//! or over DNS with a provider's token (sealed, never shown; the only way to
//! a wildcard certificate). The panel's administrators' only
//! (`system.manage`): they change Traefik. Applied to Traefik with "Apply"
//! (`traefik_service::apply_resolvers`); one in use cannot be deleted.

use std::sync::Arc;

use rustcms::admin::*;
use rustcms::{AppResult, ValidationErrors, async_trait};
use sea_orm::{ColumnTrait, DatabaseTransaction, EntityTrait, PaginatorTrait, QueryFilter};

use crate::admin::{int, is_set, text};
use crate::models::cert_resolver::{self, DNS_PROVIDERS, dns_provider, valid_name};

pub struct CertResolverResource;

#[async_trait]
impl Resource for CertResolverResource {
    type Entity = cert_resolver::Entity;
    const SLUG: &'static str = "cert_resolvers";

    fn meta() -> ResourceMeta {
        ResourceMeta::new("Certificate resolver")
            .plural("Certificate resolvers")
            .title("name")
            .icon("lock")
            .group("Settings")
            .navigation(false)
            .unique("cert_resolvers_name_unique", &["name"], "is already taken")
            // Its token is one variable for all of Traefik: one resolver per provider.
            .unique(
                "cert_resolvers_provider_unique",
                &["provider"],
                "already has a resolver (one per provider)",
            )
    }

    fn fields() -> Vec<Field> {
        vec![
            Field::column(cert_resolver::Column::Name)
                .required()
                .searchable()
                .sortable()
                .max_length(32),
            Field::column(cert_resolver::Column::Kind).required(),
            Field::column(cert_resolver::Column::Provider).max_length(32),
            Field::column(cert_resolver::Column::Email)
                .required()
                .max_length(255),
            Field::column(cert_resolver::Column::Token).encrypted(),
            Field::column(cert_resolver::Column::Staging),
            Field::column(cert_resolver::Column::CreatedAt)
                .guarded()
                .sortable(),
        ]
    }

    fn form() -> Form {
        Form::new().columns(2).schema(components![
            TextInput::make("name").help("Traefik's name for it: lowercase letters and digits"),
            SelectInput::make("kind").options([
                ("http", "Let's Encrypt (HTTP challenge)"),
                ("dns", "Let's Encrypt (DNS challenge: wildcards too)"),
            ]),
            SelectInput::make("provider").options(
                DNS_PROVIDERS
                    .iter()
                    .map(|p| (p.name, p.label))
                    .collect::<Vec<_>>()
            ),
            TextInput::make("email"),
            SecretInput::make("token").help("DNS: the provider's API token"),
            Toggle::make("staging")
                .help("Let's Encrypt's staging CA (untrusted certificates, for tests)"),
        ])
    }

    fn table() -> Table {
        Table::new()
            .columns(vec![
                TextColumn::make("name"),
                TextColumn::make("kind"),
                TextColumn::make("provider"),
                TextColumn::make("created_at"),
            ])
            .default_sort("name")
    }

    fn policy() -> Arc<dyn Policy> {
        Arc::new(ResolverPolicy)
    }

    async fn mutate_before_create(
        ctx: &Ctx,
        txn: &DatabaseTransaction,
        data: Data,
    ) -> AppResult<Data> {
        let data = trimmed(data);
        check(ctx, txn, &data, None).await?;
        Ok(data)
    }

    async fn mutate_before_update(
        ctx: &Ctx,
        txn: &DatabaseTransaction,
        record: &cert_resolver::Model,
        data: Data,
    ) -> AppResult<Data> {
        let data = trimmed(data);
        check(ctx, txn, &data, Some(record)).await?;
        Ok(data)
    }
}

/// Its text values as they are checked and stored: trimmed (an empty
/// provider: none).
fn trimmed(mut data: Data) -> Data {
    for field in ["name", "kind", "provider", "email"] {
        if let Some(value) = text(&data, field).map(|v| v.trim().to_owned()) {
            let value = if field == "provider" && value.is_empty() {
                serde_json::Value::Null
            } else {
                serde_json::Value::String(value)
            };
            data.insert(field.to_owned(), value);
        }
    }
    data
}

async fn check(
    ctx: &Ctx,
    txn: &DatabaseTransaction,
    data: &Data,
    record: Option<&cert_resolver::Model>,
) -> AppResult<()> {
    let mut errors = ValidationErrors::new();
    // The values after this write.
    let after = |field: &str, old: Option<&str>| -> Option<String> {
        if data.contains_key(field) {
            text(data, field).map(|t| t.trim().to_owned())
        } else {
            old.map(str::to_owned)
        }
    };
    let name = after("name", record.map(|r| r.name.as_str())).unwrap_or_default();
    match record {
        // Domains and deployments refer to it by name.
        Some(r) if r.name != name => errors.add("name", ctx.t("cannot change")),
        _ if !valid_name(&name) => errors.add(
            "name",
            ctx.t("must be 2-32 lowercase letters and digits, starting with a letter (not letsencrypt)"),
        ),
        _ => {}
    }
    let email = after("email", record.map(|r| r.email.as_str())).unwrap_or_default();
    if !valid_email(&email) {
        errors.add("email", ctx.t("must be an email address"));
    }
    let kind = after("kind", record.map(|r| r.kind.as_str())).unwrap_or_default();
    let provider =
        after("provider", record.and_then(|r| r.provider.as_deref())).filter(|p| !p.is_empty());
    // A token kept (not sent) or given now.
    let has_token = if data.contains_key("token") {
        // `{"clear": true}` (or null) removes it; `{"replace": "…"}` sets one.
        let cleared = data
            .get("token")
            .and_then(|t| t.get("clear"))
            .is_some_and(|c| c.as_bool() == Some(true));
        is_set(data, "token")
            && !cleared
            && text(data, "token").is_none_or(|t| !t.trim().is_empty())
            && data
                .get("token")
                .and_then(|t| t.get("replace"))
                .is_none_or(|r| r.as_str().is_some_and(|r| !r.trim().is_empty()))
    } else {
        record.is_some_and(|r| r.token.is_some())
    };
    match kind.as_str() {
        "http" => {
            if provider.is_some() {
                errors.add("provider", ctx.t("is for DNS resolvers only"));
            }
            // A wildcard's certificate comes over DNS only: not from this one.
            if let Some(r) = record
                && r.kind == "dns"
            {
                use crate::models::service_domain;
                let wildcards = service_domain::Entity::find()
                    .filter(service_domain::Column::CertResolverId.eq(r.id))
                    .filter(service_domain::Column::Https.eq(true))
                    .filter(service_domain::Column::Host.starts_with("*."))
                    .count(txn)
                    .await?;
                if wildcards > 0
                    || crate::services::cert_service::deployed_wildcard(txn, &r.name).await?
                {
                    errors.add(
                        "kind",
                        ctx.t("a wildcard domain uses it: its certificate needs DNS"),
                    );
                }
            }
        }
        "dns" => match provider.as_deref().map(dns_provider) {
            Some(Some(_)) => {
                if !has_token {
                    errors.add("token", ctx.t("is required"));
                }
                // The provider's token is one variable for all of Traefik:
                // one resolver per provider.
                let mut others = cert_resolver::Entity::find()
                    .filter(cert_resolver::Column::Kind.eq("dns"))
                    .filter(cert_resolver::Column::Provider.eq(provider.clone()));
                if let Some(r) = record {
                    others = others.filter(cert_resolver::Column::Id.ne(r.id));
                }
                if others.count(txn).await? > 0 {
                    errors.add(
                        "provider",
                        ctx.t("already has a resolver (one per provider)"),
                    );
                }
            }
            _ => errors.add("provider", ctx.t("must be one of the DNS providers listed")),
        },
        _ => errors.add("kind", ctx.t("must be http or dns")),
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.into())
    }
}

/// A plausible email: one `@`, something on both sides, no spaces or
/// characters that would leave Traefik's argument.
fn valid_email(email: &str) -> bool {
    let Some((user, domain)) = email.split_once('@') else {
        return false;
    };
    !user.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && email.len() <= 254
        && email
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "@.-_+".contains(c))
}

/// `system.manage` for everything (they change Traefik), and no delete while
/// a domain or a deployed route uses the resolver.
struct ResolverPolicy;

#[async_trait]
impl Policy for ResolverPolicy {
    async fn allows(&self, ctx: &Ctx, ability: &str, record: Option<&Data>) -> bool {
        if !ctx.can("system.manage") {
            return false;
        }
        match (ability, record) {
            ("delete", Some(r)) => {
                let (Some(id), Some(name)) = (int(r, "id"), text(r, "name")) else {
                    return false;
                };
                crate::services::cert_service::in_use(&ctx.conn(), id, name)
                    .await
                    .is_ok_and(|used| !used)
            }
            _ => true,
        }
    }

    async fn admits(&self, ctx: &Ctx, _ability: &str) -> bool {
        ctx.can("system.manage")
    }

    fn view_depends_on_record(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::valid_email;

    #[test]
    fn emails() {
        assert!(valid_email("ops@example.com"));
        assert!(valid_email("a.b+c@sub.example.io"));
        for bad in [
            "",
            "a",
            "a@b",
            "@example.com",
            "a@.com",
            "a b@example.com",
            "a@example.com --x",
            "a@example.com,b",
        ] {
            assert!(!valid_email(bad), "{bad}");
        }
    }
}
