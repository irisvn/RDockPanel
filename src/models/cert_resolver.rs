//! `cert_resolvers` table: Traefik certificate resolvers the panel manages.
//! `http` (Let's Encrypt over HTTP) or `dns` (over DNS with a provider's
//! token, sealed with `APP_KEY`, context `cert_resolvers.token`; the only way
//! to a wildcard certificate). Its `name` is Traefik's resolver name.

use rustcms::crypto::{Context, Encrypted};
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "cert_resolvers")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    /// Traefik's name for it (never changes: domains and deployments refer to it).
    pub name: String,
    /// `http` or `dns`.
    pub kind: String,
    /// `dns`: one of [`DNS_PROVIDERS`].
    pub provider: Option<String>,
    /// The ACME account's email.
    pub email: String,
    #[sea_orm(column_type = "Text", nullable)]
    pub token: Option<Encrypted<String>>,
    /// Let's Encrypt's staging CA (untrusted certificates, generous limits).
    pub staging: bool,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

/// Where `token` is sealed.
pub fn token_context() -> Context {
    Context::column("cert_resolvers", "token")
}

/// A DNS provider of Traefik's ACME client (lego): its name there and the
/// variable its token goes in (as `<VAR>_FILE`, a Docker secret).
pub struct DnsProvider {
    pub name: &'static str,
    pub label: &'static str,
    pub token_var: &'static str,
}

/// The DNS providers a resolver may use: one token each.
pub const DNS_PROVIDERS: &[DnsProvider] = &[
    DnsProvider {
        name: "cloudflare",
        label: "Cloudflare (API token, Zone:DNS:Edit)",
        token_var: "CF_DNS_API_TOKEN",
    },
    DnsProvider {
        name: "digitalocean",
        label: "DigitalOcean",
        token_var: "DO_AUTH_TOKEN",
    },
    DnsProvider {
        name: "hetzner",
        label: "Hetzner DNS",
        token_var: "HETZNER_API_KEY",
    },
    DnsProvider {
        name: "vultr",
        label: "Vultr",
        token_var: "VULTR_API_KEY",
    },
    DnsProvider {
        name: "linode",
        label: "Linode (Akamai)",
        token_var: "LINODE_TOKEN",
    },
];

/// The provider called `name`.
pub fn dns_provider(name: &str) -> Option<&'static DnsProvider> {
    DNS_PROVIDERS.iter().find(|p| p.name == name)
}

/// Resolver names: Traefik's (`--certificatesresolvers.<name>.…`), 2 to 32
/// lowercase letters and digits, starting with a letter; `letsencrypt` is the
/// installer's.
pub fn valid_name(name: &str) -> bool {
    (2..=32).contains(&name.len())
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && name != INSTALLER_RESOLVER
}

/// The installer's resolver (Let's Encrypt over HTTP): never the panel's to change.
pub const INSTALLER_RESOLVER: &str = "letsencrypt";
