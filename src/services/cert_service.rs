//! Certificate resolvers on Traefik. The installer starts Traefik with its
//! static configuration as arguments (one source only: Traefik reads no
//! other then), its `letsencrypt` resolver among them. The panel owns the
//! arguments of its own resolvers (`--certificatesresolvers.<name>.…`, any
//! name but `letsencrypt`), the DNS tokens as Swarm secrets (`rdp-dns-…`,
//! read by Traefik's ACME client through `<VAR>_FILE`), and the label saying
//! which configuration is applied. "Apply" ([`apply`]) rewrites those and
//! leaves everything else (networks, mounts, ports, the installer's
//! arguments); at boot the panel applies again when they differ (an
//! installer run replaces the arguments).

use std::collections::{BTreeMap, HashMap};

use rustcms::crypto::Keyring;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder};
use sha2::Sha256;

use crate::models::cert_resolver::{self, INSTALLER_RESOLVER, dns_provider};
use crate::models::{service, service_domain};
use crate::services::docker_service::{Docker, DockerError, DockerResult};
use crate::services::traefik_service::Traefik;

/// Traefik's ACME storage directory (the installer's mount, writable).
const ACME_DIR: &str = "/etc/traefik/acme";
/// Let's Encrypt's staging CA.
const STAGING_CA: &str = "https://acme-staging-v02.api.letsencrypt.org/directory";
/// Label of the panel's DNS token secrets.
const SECRET_LABEL: &str = "rdockpanel.dns-token";
/// Label on Traefik's service: the digest of the resolvers applied.
pub const APPLIED_LABEL: &str = "rdockpanel.resolvers";

/// A resolver as Traefik gets it (its token opened).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolver {
    pub name: String,
    pub kind: String,
    pub provider: Option<String>,
    pub email: String,
    pub token: Option<String>,
    pub staging: bool,
}

/// What Traefik gets: arguments, secrets (name → (variable, token)), and the
/// digest of it all (keyed: it never reveals a token).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub args: Vec<String>,
    pub secrets: BTreeMap<String, (String, String)>,
    pub digest: String,
}

/// The plan of `resolvers` (`key`: derived from `APP_KEY`).
pub fn plan(resolvers: &[Resolver], key: &[u8]) -> Plan {
    use hmac::{KeyInit, Mac};
    let mut resolvers: Vec<&Resolver> = resolvers.iter().collect();
    resolvers.sort_by(|a, b| a.name.cmp(&b.name));
    let mut args = Vec::new();
    let mut secrets = BTreeMap::new();
    let mut mac = hmac::Hmac::<Sha256>::new_from_slice(key).expect("HMAC takes keys of any length");
    for r in resolvers {
        let base = format!("--certificatesresolvers.{}.acme", r.name);
        args.push(format!("{base}.email={}", r.email));
        args.push(format!("{base}.storage={ACME_DIR}/{}.json", r.name));
        if r.staging {
            args.push(format!("{base}.caserver={STAGING_CA}"));
        }
        match (
            r.kind.as_str(),
            r.provider.as_deref().and_then(dns_provider),
        ) {
            ("dns", Some(provider)) => {
                args.push(format!("{base}.dnschallenge.provider={}", provider.name));
                args.push(format!(
                    "{base}.dnschallenge.resolvers=1.1.1.1:53,8.8.8.8:53"
                ));
                let token = r.token.clone().unwrap_or_default();
                // A new token, a new secret (Swarm's are immutable).
                let mut name_mac = mac.clone();
                name_mac.update(format!("{}\0{token}", r.name).as_bytes());
                let suffix: String = name_mac.finalize().into_bytes()[..6]
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect();
                secrets.insert(
                    format!("rdp-dns-{}-{suffix}", r.name),
                    (provider.token_var.to_owned(), token),
                );
            }
            _ => args.push(format!("{base}.httpchallenge.entrypoint=web")),
        }
    }
    for arg in &args {
        mac.update(arg.as_bytes());
        mac.update(b"\n");
    }
    for name in secrets.keys() {
        mac.update(name.as_bytes());
        mac.update(b"\n");
    }
    let digest = mac.finalize().into_bytes()[..12]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Plan {
        args,
        secrets,
        digest,
    }
}

/// The resolver an argument configures (`--certificatesresolvers.<name>.…`).
fn arg_resolver(arg: &str) -> Option<&str> {
    arg.strip_prefix("--certificatesresolvers.")
        .and_then(|rest| rest.split_once('.'))
        .map(|(name, _)| name)
}

/// Whether `arg` is one of the panel's resolver arguments: of a resolver it
/// gave Traefik (`owned`, from its label) or gives now; never the
/// installer's, nor one an administrator added by hand.
fn is_owned_arg(arg: &str, owned: &[String]) -> bool {
    arg_resolver(arg)
        .is_some_and(|name| name != INSTALLER_RESOLVER && owned.iter().any(|o| o == name))
}

impl Plan {
    /// The resolvers it configures, by name.
    fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .args
            .iter()
            .filter_map(|a| arg_resolver(a))
            .map(str::to_owned)
            .collect();
        names.dedup();
        names
    }

    /// Whether `spec` runs exactly this plan: labelled with it, its
    /// arguments, its variables and its secrets all there, none of the
    /// panel's others.
    fn runs_in(&self, spec: &crate::services::docker_service::ServiceSpec) -> bool {
        let labels = spec.labels.clone().unwrap_or_default();
        if labels.get(APPLIED_LABEL) != Some(&self.digest) {
            return false;
        }
        let owned = owned_names(&labels, self);
        let container = spec
            .task_template
            .as_ref()
            .and_then(|t| t.container_spec.clone())
            .unwrap_or_default();
        let args = container.args.unwrap_or_default();
        let env = container.env.unwrap_or_default();
        let secrets: Vec<String> = container
            .secrets
            .unwrap_or_default()
            .into_iter()
            .filter_map(|s| s.secret_name)
            .collect();
        self.args.iter().all(|a| args.contains(a))
            && args.iter().filter(|a| is_owned_arg(a, &owned)).count() == self.args.len()
            && self.secrets.iter().all(|(name, (var, _))| {
                secrets.contains(name) && env.contains(&format!("{var}_FILE=/run/secrets/{name}"))
            })
            && env.iter().filter(|e| is_token_var(e)).count() == self.secrets.len()
            && secrets.iter().filter(|n| n.starts_with("rdp-dns-")).count() == self.secrets.len()
    }
}

/// Whether the variable `entry` (`NAME=value`) is a DNS provider's token
/// file (`<VAR>_FILE`, the panel's to give: whatever its value).
fn is_token_var(entry: &str) -> bool {
    let name = entry.split_once('=').map_or(entry, |(name, _)| name);
    crate::models::cert_resolver::DNS_PROVIDERS
        .iter()
        .any(|p| name.strip_suffix("_FILE") == Some(p.token_var))
}

/// Label on Traefik's service: the names of the resolvers the panel gave it.
const OWNED_LABEL: &str = "rdockpanel.resolver-names";

/// The resolvers the panel gave Traefik (its label) and gives now.
fn owned_names(labels: &HashMap<String, String>, plan: &Plan) -> Vec<String> {
    let mut owned: Vec<String> = labels
        .get(OWNED_LABEL)
        .map(|l| {
            l.split(',')
                .filter(|n| !n.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    owned.extend(plan.names());
    owned
}

/// The key of the plan's digests, from `APP_KEY`.
pub fn plan_key(keyring: &Keyring) -> [u8; 32] {
    keyring.current().derive("rdockpanel.cert-resolvers")
}

/// The resolvers of the database, their tokens opened.
pub async fn resolvers(
    db: &DatabaseConnection,
    keyring: &Keyring,
) -> Result<Vec<Resolver>, String> {
    let rows = cert_resolver::Entity::find()
        .order_by_asc(cert_resolver::Column::Name)
        .all(db)
        .await
        .map_err(|e| e.to_string())?;
    rows.into_iter()
        .map(|r| {
            let token = match &r.token {
                Some(sealed) => Some(
                    sealed
                        .open(keyring, &cert_resolver::token_context())
                        .map_err(|_| {
                            format!("the token of {} cannot be read (another APP_KEY)", r.name)
                        })?,
                ),
                None => None,
            };
            Ok(Resolver {
                name: r.name,
                kind: r.kind,
                provider: r.provider,
                email: r.email,
                token,
                staging: r.staging,
            })
        })
        .collect()
}

/// Whether Traefik runs `plan` (its label says so); `None`: no Traefik service.
pub async fn applied(
    docker: &Docker,
    traefik: &Traefik,
    plan: &Plan,
) -> DockerResult<Option<bool>> {
    if traefik.service.is_empty() {
        return Ok(None);
    }
    let Some(current) = docker.engine().inspect_service(&traefik.service).await? else {
        return Ok(None);
    };
    let spec = current.spec.unwrap_or_default();
    let labelled = spec
        .labels
        .as_ref()
        .is_some_and(|l| l.contains_key(APPLIED_LABEL));
    // Nothing of the panel's on a Traefik that never had any: applied.
    Ok(Some(
        plan.runs_in(&spec) || (!labelled && plan.args.is_empty()),
    ))
}

/// Gives Traefik `plan` again when its label says it has it but its
/// configuration lost some of it (an installer run replaces the arguments):
/// what Apply gave, back. A plan not applied yet (resolvers changed, Apply
/// not pressed) waits. Whether it gave it again.
pub async fn reconcile(docker: &Docker, traefik: &Traefik, plan: &Plan) -> DockerResult<bool> {
    apply_plan(docker, traefik, plan, Mode::Reconcile).await
}

/// Gives Traefik `plan` (Swarm restarts it): its secrets created, the panel's
/// arguments, variables and secrets replaced, the rest kept. Old secrets are
/// removed after. Whether anything changed (nothing when Traefik runs it).
pub async fn apply(docker: &Docker, traefik: &Traefik, plan: &Plan) -> DockerResult<bool> {
    apply_plan(docker, traefik, plan, Mode::Apply).await
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Whatever Traefik has.
    Apply,
    /// Only when Traefik is labelled with this plan (decided under the
    /// lock: an Apply in between is never undone).
    Reconcile,
}

async fn apply_plan(
    docker: &Docker,
    traefik: &Traefik,
    plan: &Plan,
    mode: Mode,
) -> DockerResult<bool> {
    if traefik.service.is_empty() {
        return Ok(false);
    }
    let api = docker.engine();
    // Secrets, the service, then old secrets: one Apply at a time (another
    // never deletes the secret this one is about to give).
    let _exclusive = traefik.exclusive().await;
    if mode == Mode::Reconcile {
        // Most often nothing to do: no secret made for it.
        match api.inspect_service(&traefik.service).await? {
            Some(current) => {
                let spec = current.spec.unwrap_or_default();
                let labelled =
                    spec.labels.as_ref().and_then(|l| l.get(APPLIED_LABEL)) == Some(&plan.digest);
                if !labelled || plan.runs_in(&spec) {
                    return Ok(false);
                }
            }
            None => return Ok(false),
        }
    }
    // The secrets first (a name taken: it is that very content).
    let mut ids = HashMap::new();
    let existing: HashMap<String, String> = api
        .list_secrets(SECRET_LABEL, "1")
        .await?
        .into_iter()
        .map(|(id, name)| (name, id))
        .collect();
    for (name, (_, token)) in &plan.secrets {
        let id = match existing.get(name) {
            Some(id) => id.clone(),
            None => {
                let labels = HashMap::from([(SECRET_LABEL.to_owned(), "1".to_owned())]);
                match api.create_secret(name, token.as_bytes(), labels).await {
                    Ok(id) => id,
                    Err(DockerError::Conflict(_)) => api
                        .list_secrets(SECRET_LABEL, "1")
                        .await?
                        .into_iter()
                        .find(|(_, n)| n == name)
                        .map(|(id, _)| id)
                        .ok_or(DockerError::NotFound)?,
                    Err(err) => return Err(err),
                }
            }
        };
        ids.insert(name.clone(), id);
    }
    let changed = traefik
        .update_service_locked(api, |spec| {
            if plan.runs_in(spec) {
                return false;
            }
            let labels = spec.labels.get_or_insert_with(HashMap::new);
            if mode == Mode::Reconcile && labels.get(APPLIED_LABEL) != Some(&plan.digest) {
                return false;
            }
            let owned = owned_names(labels, plan);
            labels.insert(APPLIED_LABEL.to_owned(), plan.digest.clone());
            labels.insert(OWNED_LABEL.to_owned(), plan.names().join(","));
            let container = spec
                .task_template
                .get_or_insert_with(Default::default)
                .container_spec
                .get_or_insert_with(Default::default);
            let args = container.args.get_or_insert_with(Vec::new);
            args.retain(|a| !is_owned_arg(a, &owned));
            args.extend(plan.args.iter().cloned());
            let env = container.env.get_or_insert_with(Vec::new);
            env.retain(|e| !is_token_var(e));
            let secrets = container.secrets.get_or_insert_with(Vec::new);
            secrets.retain(|s| {
                !s.secret_name
                    .as_deref()
                    .is_some_and(|n| n.starts_with("rdp-dns-"))
            });
            for (name, (var, _)) in &plan.secrets {
                env.push(format!("{var}_FILE=/run/secrets/{name}"));
                secrets.push(bollard::models::TaskSpecContainerSpecSecrets {
                    file: Some(bollard::models::TaskSpecContainerSpecFile {
                        name: Some(name.clone()),
                        uid: Some("0".to_owned()),
                        gid: Some("0".to_owned()),
                        mode: Some(0o400),
                    }),
                    secret_id: ids.get(name).cloned(),
                    secret_name: Some(name.clone()),
                });
            }
            // Restarted with the new configuration.
            let template = spec.task_template.get_or_insert_with(Default::default);
            template.force_update = Some(template.force_update.unwrap_or(0) + 1);
            true
        })
        .await?;
    // Secrets no longer given (Swarm refuses those still in use).
    for (name, id) in existing {
        if !plan.secrets.contains_key(&name) {
            let _ = api.delete_secret(&id).await;
        }
    }
    Ok(changed)
}

/// What a domain's HTTPS may use: the resolver `resolver_id` exists, a
/// wildcard's certificate comes over DNS (its resolver's, or the default's
/// when there is none: only development's self-signed one), and one host
/// has one certificate (Traefik picks it by name, before any path): every
/// HTTPS domain of the host chooses the same resolver. `own`: the domain
/// itself, left out. The field and the message of what is wrong.
pub async fn check_domain_tls<C: sea_orm::ConnectionTrait>(
    db: &C,
    own: Option<i64>,
    host: &str,
    https: bool,
    resolver_id: Option<i64>,
) -> rustcms::AppResult<Option<(&'static str, &'static str)>> {
    let resolver = match resolver_id {
        Some(id) => match cert_resolver::Entity::find_by_id(id).one(db).await? {
            Some(r) => Some(r),
            None => return Ok(Some(("cert_resolver_id", "is not a certificate resolver"))),
        },
        None => None,
    };
    if !https {
        return Ok(None);
    }
    if host.starts_with("*.") {
        let dns = match &resolver {
            Some(r) => r.kind == "dns",
            // The default: the installer's (HTTP) one, or none (development).
            None => crate::services::deploy_rules_service::config()
                .cert_resolver
                .is_empty(),
        };
        if !dns {
            return Ok(Some((
                "cert_resolver_id",
                "a wildcard's certificate needs a DNS resolver: add one in Settings and choose it",
            )));
        }
    }
    let mut others = service_domain::Entity::find()
        .filter(service_domain::Column::Host.eq(host))
        .filter(service_domain::Column::Https.eq(true));
    if let Some(id) = own {
        others = others.filter(service_domain::Column::Id.ne(id));
    }
    if others
        .all(db)
        .await?
        .iter()
        .any(|d| d.cert_resolver_id != resolver_id)
    {
        return Ok(Some((
            "cert_resolver_id",
            "this host's other domains use another certificate resolver (one per host)",
        )));
    }
    Ok(None)
}

/// Whether a deployed route asks the resolver `name` for a wildcard's
/// certificate (it must stay a DNS one until that service deploys again).
pub async fn deployed_wildcard<C: sea_orm::ConnectionTrait>(
    db: &C,
    name: &str,
) -> rustcms::AppResult<bool> {
    let options = crate::services::deployment_service::deploy_options(0);
    for row in service::Entity::find()
        .filter(service::Column::DeployedSpecHash.is_not_null())
        .all(db)
        .await?
    {
        if let Some(routes) =
            crate::services::traefik_service::deployed_routes(db, &row, &options).await?
            && routes.uses_resolver_for_wildcard(name)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Whether the resolver `id` (named `name`) is in use: by a domain, or by
/// the routes of a deployed service (its deployment refers to it by name).
pub async fn in_use<C: sea_orm::ConnectionTrait>(
    db: &C,
    id: i64,
    name: &str,
) -> rustcms::AppResult<bool> {
    let domains = service_domain::Entity::find()
        .filter(service_domain::Column::CertResolverId.eq(id))
        .one(db)
        .await?;
    if domains.is_some() {
        return Ok(true);
    }
    let options = crate::services::deployment_service::deploy_options(0);
    for row in service::Entity::find()
        .filter(service::Column::DeployedSpecHash.is_not_null())
        .all(db)
        .await?
    {
        if let Some(routes) =
            crate::services::traefik_service::deployed_routes(db, &row, &options).await?
            && routes.uses_resolver(name)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolver(name: &str, kind: &str, provider: Option<&str>, token: Option<&str>) -> Resolver {
        Resolver {
            name: name.to_owned(),
            kind: kind.to_owned(),
            provider: provider.map(str::to_owned),
            email: "ops@example.com".to_owned(),
            token: token.map(str::to_owned),
            staging: false,
        }
    }

    #[test]
    fn plans_arguments_and_secrets() {
        let plan = plan(
            &[
                resolver("cf", "dns", Some("cloudflare"), Some("tok")),
                resolver("http", "http", None, None),
            ],
            b"key",
        );
        assert_eq!(
            plan.args,
            [
                "--certificatesresolvers.cf.acme.email=ops@example.com",
                "--certificatesresolvers.cf.acme.storage=/etc/traefik/acme/cf.json",
                "--certificatesresolvers.cf.acme.dnschallenge.provider=cloudflare",
                "--certificatesresolvers.cf.acme.dnschallenge.resolvers=1.1.1.1:53,8.8.8.8:53",
                "--certificatesresolvers.http.acme.email=ops@example.com",
                "--certificatesresolvers.http.acme.storage=/etc/traefik/acme/http.json",
                "--certificatesresolvers.http.acme.httpchallenge.entrypoint=web",
            ]
        );
        let (name, (var, token)) = plan.secrets.iter().next().unwrap();
        assert!(name.starts_with("rdp-dns-cf-"), "{name}");
        assert_eq!((var.as_str(), token.as_str()), ("CF_DNS_API_TOKEN", "tok"));
    }

    #[test]
    fn digests_change_with_the_token_and_never_show_it() {
        let a = plan(
            &[resolver("cf", "dns", Some("cloudflare"), Some("one"))],
            b"key",
        );
        let b = plan(
            &[resolver("cf", "dns", Some("cloudflare"), Some("two"))],
            b"key",
        );
        assert_ne!(a.digest, b.digest);
        assert_ne!(a.secrets.keys().next(), b.secrets.keys().next());
        assert!(!a.digest.contains("one") && !format!("{:?}", a.args).contains("one"));
        // The same input, the same plan.
        assert_eq!(
            a,
            plan(
                &[resolver("cf", "dns", Some("cloudflare"), Some("one"))],
                b"key"
            )
        );
    }

    #[test]
    fn owns_its_resolvers_arguments_only() {
        let owned = vec!["cf".to_owned(), "letsencrypt".to_owned()];
        assert!(is_owned_arg(
            "--certificatesresolvers.cf.acme.email=a@b.c",
            &owned
        ));
        assert!(!is_owned_arg(
            "--certificatesresolvers.letsencrypt.acme.email=a@b.c",
            &owned
        ));
        // An administrator's own resolver, added by hand: never touched.
        assert!(!is_owned_arg(
            "--certificatesresolvers.mine.acme.email=a@b.c",
            &owned
        ));
        assert!(!is_owned_arg("--entrypoints.web.address=:80", &owned));
        assert!(!is_owned_arg(
            "--providers.file.directory=/etc/traefik/dynamic",
            &owned
        ));
    }
}
