//! Certificates uploaded to the panel: read and checked ([`parse`]), and
//! given to Traefik ([`write_store`]): each as a chain and a key file under
//! `<traefik_dir>/certs/` (the key `0600`, names the panel's: `<id>-<hash>`),
//! listed in `<traefik_dir>/rdp-certificates.yml` (`tls.certificates`, with
//! Traefik's own paths). Traefik serves the one covering a name it is asked
//! (by SNI); a domain choosing one gets a router without a resolver. Files
//! only change when the list does: no restart.

use std::io::Write;
use std::path::Path;

use chrono::{DateTime, Utc};
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use sha2::{Digest, Sha256};

/// Largest chain and key accepted (PEM text).
const MAX_CHAIN: usize = 64 * 1024;
const MAX_KEY: usize = 16 * 1024;
/// At most this many certificates in a chain (the certificate, then its
/// intermediates).
const MAX_CHAIN_LENGTH: usize = 6;
/// The file listing them for Traefik (not a service's: never pruned).
pub const STORE_FILE: &str = "rdp-certificates.yml";
/// Their directory, beside the routes.
pub const STORE_DIR: &str = "certs";

/// What the panel reads from a chain and its key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    /// The DNS names it covers, lowercase (`*.example.com` for a wildcard).
    pub names: Vec<String>,
    pub not_after: DateTime<Utc>,
    /// SHA-256 of the certificate (DER), lowercase hex.
    pub fingerprint: String,
    /// The chain and the key as Traefik gets them (PEM, normalised).
    pub chain: String,
    pub key: String,
    /// Whether its issuer is the last certificate given (a self-signed one,
    /// or a chain ending at an authority browsers may not know).
    pub self_signed: bool,
}

/// Reads `chain` (PEM: the certificate, then its intermediates) and `key`
/// (its PEM private key, not encrypted), at `now`: the key must be the
/// certificate's, each certificate signed by the next, the certificate valid
/// now, for servers (when it says what for), and for at least one DNS name.
/// The reason it is refused.
pub fn parse(chain: &str, key: &str, now: DateTime<Utc>) -> Result<Parsed, &'static str> {
    if chain.len() > MAX_CHAIN {
        return Err("the certificate is too large (64 KB at most)");
    }
    if key.len() > MAX_KEY {
        return Err("the key is too large (16 KB at most)");
    }
    if key.contains("ENCRYPTED") {
        return Err("the key is encrypted: give it without a passphrase");
    }
    let ders: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(chain.as_bytes())
        .collect::<Result<_, _>>()
        .map_err(|_| "the certificate is not PEM (-----BEGIN CERTIFICATE-----)")?;
    if ders.is_empty() {
        return Err("no certificate found (-----BEGIN CERTIFICATE-----)");
    }
    if ders.len() > MAX_CHAIN_LENGTH {
        return Err("too many certificates in the chain (6 at most)");
    }
    let key_der = PrivateKeyDer::from_pem_slice(key.as_bytes())
        .map_err(|_| "the key is not a PEM private key (-----BEGIN PRIVATE KEY-----)")?;
    let mut certs = Vec::new();
    for der in &ders {
        let (_, cert) = x509_parser::parse_x509_certificate(der.as_ref())
            .map_err(|_| "a certificate of the chain cannot be read")?;
        certs.push(cert);
    }
    let leaf = &certs[0];
    // The key is the certificate's.
    let signer = rustls::crypto::ring::sign::any_supported_type(&key_der)
        .map_err(|_| "the key's type is not supported (RSA, ECDSA or Ed25519)")?;
    let public = signer
        .public_key()
        .ok_or("the key's public part cannot be read")?;
    if public.as_ref() != leaf.public_key().raw {
        return Err("the key is not this certificate's");
    }
    // Each one signed by the next.
    for pair in certs.windows(2) {
        if pair[0].issuer().as_raw() != pair[1].subject().as_raw()
            || pair[0]
                .verify_signature(Some(pair[1].public_key()))
                .is_err()
        {
            return Err(
                "the chain is out of order or broken: the certificate first, then each issuer",
            );
        }
    }
    // Every certificate of the chain valid now (clients check them all).
    for cert in certs.iter().skip(1) {
        let validity = cert.validity();
        if now.timestamp() < validity.not_before.timestamp()
            || now.timestamp() >= validity.not_after.timestamp()
        {
            return Err("an intermediate certificate of the chain is expired or not valid yet");
        }
    }
    let validity = leaf.validity();
    let not_before = DateTime::from_timestamp(validity.not_before.timestamp(), 0)
        .ok_or("the certificate's dates cannot be read")?;
    let not_after = DateTime::from_timestamp(validity.not_after.timestamp(), 0)
        .ok_or("the certificate's dates cannot be read")?;
    if now < not_before {
        return Err("the certificate is not valid yet");
    }
    if now >= not_after {
        return Err("the certificate has expired");
    }
    match leaf.extended_key_usage() {
        Ok(Some(usage)) if !(usage.value.server_auth || usage.value.any) => {
            return Err("the certificate is not for servers (TLS Web Server Authentication)");
        }
        Err(_) => return Err("the certificate's extended key usage cannot be read"),
        _ => {}
    }
    let mut names: Vec<String> = Vec::new();
    if let Ok(Some(alternatives)) = leaf.subject_alternative_name() {
        for name in &alternatives.value.general_names {
            if let x509_parser::extensions::GeneralName::DNSName(dns) = name {
                let dns = dns.to_ascii_lowercase();
                if crate::services::deploy_rules_service::valid_domain_host(&dns)
                    && !names.contains(&dns)
                {
                    names.push(dns);
                }
            }
        }
    }
    if names.is_empty() {
        return Err("the certificate names no DNS name (subject alternative names)");
    }
    let last = certs.last().expect("one at least");
    let self_signed = last.issuer().as_raw() == last.subject().as_raw();
    let fingerprint = hex(&Sha256::digest(ders[0].as_ref()));
    let chain = ders
        .iter()
        .map(|der| pem("CERTIFICATE", der.as_ref()))
        .collect::<String>();
    let key = match &key_der {
        PrivateKeyDer::Pkcs1(k) => pem("RSA PRIVATE KEY", k.secret_pkcs1_der()),
        PrivateKeyDer::Sec1(k) => pem("EC PRIVATE KEY", k.secret_sec1_der()),
        PrivateKeyDer::Pkcs8(k) => pem("PRIVATE KEY", k.secret_pkcs8_der()),
        _ => return Err("the key's type is not supported (RSA, ECDSA or Ed25519)"),
    };
    Ok(Parsed {
        names,
        not_after,
        fingerprint,
        chain,
        key,
        self_signed,
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn pem(label: &str, der: &[u8]) -> String {
    use base64::Engine;
    let body = base64::engine::general_purpose::STANDARD.encode(der);
    let mut out = format!("-----BEGIN {label}-----\n");
    for line in body.as_bytes().chunks(64) {
        out.push_str(std::str::from_utf8(line).expect("base64 is ASCII"));
        out.push('\n');
    }
    out.push_str(&format!("-----END {label}-----\n"));
    out
}

/// Whether a certificate naming `names` covers `host`: the same name, or a
/// wildcard name one level above (`*.example.com` covers `a.example.com`,
/// not `example.com` nor `a.b.example.com`); a wildcard host only by the same
/// wildcard name.
pub fn covers(names: &[&str], host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    names.iter().any(|name| {
        let name = name.to_ascii_lowercase();
        if name == host {
            return true;
        }
        match (name.strip_prefix("*."), host.split_once('.')) {
            (Some(parent), Some((label, rest))) => {
                !host.starts_with("*.") && !label.is_empty() && rest == parent
            }
            _ => false,
        }
    })
}

/// Gives Traefik the uploaded certificates of the database (their keys
/// opened): [`write_store`] in `traefik`'s directory. One at a time.
pub async fn sync(
    db: &sea_orm::DatabaseConnection,
    keyring: &rustcms::crypto::Keyring,
    traefik: &crate::services::traefik_service::Traefik,
) -> Result<(), String> {
    use sea_orm::{EntityTrait, QueryOrder};
    static WRITING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _writing = WRITING.lock().await;
    let rows = crate::models::certificate::Entity::find()
        .order_by_asc(crate::models::certificate::Column::Id)
        .all(db)
        .await
        .map_err(|e| e.to_string())?;
    let mut stored = Vec::new();
    for row in rows {
        let key = row
            .key
            .open(keyring, &crate::models::certificate::key_context())
            .map_err(|_| format!("the key of {} cannot be read (another APP_KEY)", row.name))?;
        stored.push(Stored {
            id: row.id,
            fingerprint: row.fingerprint,
            chain: row.chain,
            key,
        });
    }
    let mount = crate::services::deploy_rules_service::config()
        .traefik_mount
        .clone();
    let dir = traefik.dir.clone();
    tokio::task::spawn_blocking(move || write_store(&dir, &mount, &stored))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("the certificates cannot be written: {e}"))
}

/// The certificate `id`, its row locked until `db`'s transaction ends (a
/// domain choosing it and its deletion or replacement never interleave;
/// SQLite writes one transaction at a time anyway).
pub async fn locked<C: sea_orm::ConnectionTrait>(
    db: &C,
    id: i64,
) -> rustcms::AppResult<Option<crate::models::certificate::Model>> {
    use sea_orm::{EntityTrait, QuerySelect};
    let find = crate::models::certificate::Entity::find_by_id(id);
    let find = if db.get_database_backend() == sea_orm::DatabaseBackend::Sqlite {
        find
    } else {
        find.lock_exclusive()
    };
    Ok(find.one(db).await?)
}

/// The hosts served with the certificate `id` over HTTPS: its domains', and
/// those of deployed routes still using it (until their service deploys).
pub async fn hosts_using<C: sea_orm::ConnectionTrait>(
    db: &C,
    id: i64,
) -> rustcms::AppResult<Vec<String>> {
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
    let mut hosts: Vec<String> = crate::models::service_domain::Entity::find()
        .filter(crate::models::service_domain::Column::CertificateId.eq(id))
        .filter(crate::models::service_domain::Column::Https.eq(true))
        .all(db)
        .await?
        .into_iter()
        .map(|d| d.host)
        .collect();
    let key = id.to_string();
    for row in crate::models::service::Entity::find()
        .filter(crate::models::service::Column::DeployedSpecHash.is_not_null())
        .all(db)
        .await?
    {
        if let Some(snapshot) =
            crate::services::traefik_service::deployed_snapshot(db, &row).await?
        {
            for d in snapshot.domains {
                if d.https
                    && d.certificate.as_deref() == Some(key.as_str())
                    && !hosts.contains(&d.host)
                {
                    hosts.push(d.host);
                }
            }
        }
    }
    Ok(hosts)
}

/// Whether the certificates a deployment asks for are there: each uploaded
/// one (still covering its HTTPS host), each resolver (the panel's, or the
/// installer's). A deployment asked before one was deleted or narrowed (or
/// a rollback to an old configuration) fails rather than serve without it.
pub async fn check_snapshot<C: sea_orm::ConnectionTrait>(
    db: &C,
    spec: &serde_json::Value,
) -> Result<(), String> {
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
    let Ok(snapshot) =
        serde_json::from_value::<crate::services::deploy_service::Snapshot>(spec.clone())
    else {
        return Ok(());
    };
    if !snapshot.running {
        return Ok(());
    }
    for d in snapshot.domains.iter().filter(|d| d.https) {
        if let Some(id) = &d.certificate {
            let certificate = match id.parse::<i64>() {
                Ok(id) => crate::models::certificate::Entity::find_by_id(id)
                    .one(db)
                    .await
                    .map_err(|e| e.to_string())?,
                Err(_) => None,
            };
            match certificate {
                Some(c) if covers(&c.name_list(), &d.host) => {}
                Some(c) => {
                    return Err(format!(
                        "the certificate {} no longer covers {}: choose another on the domain",
                        c.name, d.host
                    ));
                }
                None => {
                    return Err(format!(
                        "the certificate of {} was deleted: choose another on the domain",
                        d.host
                    ));
                }
            }
        }
        if let Some(name) = &d.cert_resolver
            && name != crate::models::cert_resolver::INSTALLER_RESOLVER
            && crate::models::cert_resolver::Entity::find()
                .filter(crate::models::cert_resolver::Column::Name.eq(name.as_str()))
                .one(db)
                .await
                .map_err(|e| e.to_string())?
                .is_none()
        {
            return Err(format!(
                "the certificate resolver {name} of {} was deleted: choose another on the domain",
                d.host
            ));
        }
    }
    Ok(())
}

/// Whether the certificate `id` is in use: by a domain, or by the routes of
/// a deployed service.
pub async fn in_use<C: sea_orm::ConnectionTrait>(db: &C, id: i64) -> rustcms::AppResult<bool> {
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
    if crate::models::service_domain::Entity::find()
        .filter(crate::models::service_domain::Column::CertificateId.eq(id))
        .one(db)
        .await?
        .is_some()
    {
        return Ok(true);
    }
    let id = id.to_string();
    for row in crate::models::service::Entity::find()
        .filter(crate::models::service::Column::DeployedSpecHash.is_not_null())
        .all(db)
        .await?
    {
        if let Some(snapshot) =
            crate::services::traefik_service::deployed_snapshot(db, &row).await?
            && snapshot
                .domains
                .iter()
                .any(|d| d.certificate.as_deref() == Some(id.as_str()))
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Whole days left before `not_after` at `now`, rounded up while it is valid
/// (23 hours left: 1), down once expired (0 or less).
pub fn days_left(not_after: DateTime<Utc>, now: DateTime<Utc>) -> i64 {
    const DAY: i64 = 86_400_000;
    let millis = (not_after - now).num_milliseconds();
    if millis > 0 {
        (millis + DAY - 1) / DAY
    } else {
        millis.div_euclid(DAY)
    }
}

/// Days left before which a warning is sent (each once per certificate).
const WARN_DAYS: [i64; 4] = [14, 7, 1, 0];

/// The warning due for a certificate with `days_left`, last warned at
/// `notified` days: the threshold crossed since, if any.
pub fn warning_due(days_left: i64, notified: Option<i32>) -> Option<i64> {
    let due = WARN_DAYS
        .iter()
        .copied()
        .filter(|t| days_left <= *t)
        .min()?;
    match notified {
        Some(n) if i64::from(n) <= due => None,
        _ => Some(due),
    }
}

/// Says to the channels which certificates expire soon (14, 7, 1 days
/// before, and once expired), each warning once.
pub async fn check_expiry(state: &rustcms::AppState) -> rustcms::AppResult<()> {
    use sea_orm::{ActiveModelTrait, EntityTrait, IntoActiveModel, Set};
    let now = Utc::now();
    for row in crate::models::certificate::Entity::find()
        .all(state.db())
        .await?
    {
        let days_left = days_left(row.not_after, now);
        let Some(due) = warning_due(days_left, row.notified_days) else {
            continue;
        };
        crate::services::notify_service::notify(
            state,
            crate::services::notify_service::Event::Certificate {
                name: row.name.clone(),
                names: row.names.replace(',', ", "),
                days_left,
            },
        );
        let mut active = row.into_active_model();
        active.notified_days = Set(Some(i32::try_from(due).unwrap_or(0)));
        active.update(state.db()).await?;
    }
    Ok(())
}

/// A certificate as Traefik gets it.
pub struct Stored {
    pub id: i64,
    pub fingerprint: String,
    pub chain: String,
    pub key: String,
}

/// Writes `certs` for Traefik in `dir` (the panel's view of Traefik's
/// directory), referring to them under `traefik_dir` (Traefik's view):
/// new files first (keys `0600`), then the list, then old files removed.
pub fn write_store(dir: &Path, traefik_dir: &str, certs: &[Stored]) -> std::io::Result<()> {
    // Held by the thread writing (a request given up never releases it early).
    static WRITING: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _writing = WRITING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let store = dir.join(STORE_DIR);
    create_private_dir(&store)?;
    let mut keep = std::collections::HashSet::new();
    let mut list = Vec::new();
    for cert in certs {
        let stem = format!(
            "{}-{}",
            cert.id,
            &cert.fingerprint[..16.min(cert.fingerprint.len())]
        );
        let (crt, key) = (format!("{stem}.crt"), format!("{stem}.key"));
        write_private(&store, &crt, cert.chain.as_bytes())?;
        write_private(&store, &key, cert.key.as_bytes())?;
        list.push(serde_json::json!({
            "certFile": format!("{}/{STORE_DIR}/{crt}", traefik_dir.trim_end_matches('/')),
            "keyFile": format!("{}/{STORE_DIR}/{key}", traefik_dir.trim_end_matches('/')),
        }));
        keep.insert(crt);
        keep.insert(key);
    }
    let yaml = if list.is_empty() {
        String::new()
    } else {
        serde_yaml_ng::to_string(&serde_json::json!({ "tls": { "certificates": list } }))
            .map_err(std::io::Error::other)?
    };
    write_private(dir, STORE_FILE, yaml.as_bytes())?;
    for entry in std::fs::read_dir(&store)?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !keep.contains(&name) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    Ok(())
}

fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    // A symlink in its place is never followed (its target is not ours).
    if std::fs::symlink_metadata(dir).is_ok_and(|m| !m.is_dir()) {
        return Err(std::io::Error::other(format!(
            "{} is not a directory (a symlink?): remove it",
            dir.display()
        )));
    }
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Writes `name` in `dir` through a temporary file created `0600` (never
/// readable by others, even for a moment), then renamed over it. A symlink
/// in its place is replaced, never followed.
fn write_private(dir: &Path, name: &str, bytes: &[u8]) -> std::io::Result<()> {
    // The same already: left alone (Traefik reloads on any write).
    if std::fs::symlink_metadata(dir.join(name)).is_ok_and(|m| m.is_file())
        && std::fs::read(dir.join(name)).is_ok_and(|old| old == bytes)
    {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir.join(name), std::fs::Permissions::from_mode(0o600))?;
        }
        return Ok(());
    }
    let tmp = dir.join(format!(".{name}.tmp"));
    let _ = std::fs::remove_file(&tmp);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&tmp, dir.join(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_left_until_it_expires() {
        let now = Utc::now();
        assert_eq!(days_left(now + chrono::Duration::hours(23), now), 1);
        assert_eq!(days_left(now + chrono::Duration::days(14), now), 14);
        assert_eq!(
            days_left(now + chrono::Duration::hours(14 * 24 + 1), now),
            15
        );
        assert_eq!(days_left(now, now), 0);
        assert_eq!(days_left(now + chrono::Duration::milliseconds(500), now), 1);
        assert_eq!(
            days_left(now + chrono::Duration::milliseconds(86_400_500), now),
            2
        );
        assert_eq!(days_left(now - chrono::Duration::hours(1), now), -1);
    }

    #[test]
    fn warns_once_per_threshold() {
        assert_eq!(warning_due(30, None), None);
        assert_eq!(warning_due(14, None), Some(14));
        assert_eq!(warning_due(10, Some(14)), None);
        assert_eq!(warning_due(6, Some(14)), Some(7));
        assert_eq!(warning_due(0, Some(1)), Some(0));
        assert_eq!(warning_due(-3, Some(0)), None);
        // Renewed (warnings reset): nothing until the next threshold.
        assert_eq!(warning_due(80, None), None);
    }

    #[test]
    fn coverage() {
        let names = ["example.com", "*.example.com"];
        assert!(covers(&names, "example.com"));
        assert!(covers(&names, "Shop.Example.com"));
        assert!(!covers(&names, "a.b.example.com"));
        assert!(!covers(&names, "example.org"));
        assert!(covers(&names, "*.example.com"));
        assert!(!covers(&["a.example.com"], "*.example.com"));
        assert!(!covers(&["*.example.com"], "*.shop.example.com"));
    }

    /// A certificate for `names` signed by a new authority: (chain, key).
    fn issued(names: &[&str]) -> (String, String) {
        use rcgen::{BasicConstraints, CertificateParams, IsCa, Issuer, KeyPair};
        let ca_key = KeyPair::generate().unwrap();
        let mut ca = CertificateParams::new(Vec::<String>::new()).unwrap();
        ca.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let ca_cert = ca.self_signed(&ca_key).unwrap();
        let issuer = Issuer::new(ca, ca_key);
        let key = KeyPair::generate().unwrap();
        let params =
            CertificateParams::new(names.iter().map(|n| (*n).to_owned()).collect::<Vec<_>>())
                .unwrap();
        let cert = params.signed_by(&key, &issuer).unwrap();
        (
            format!("{}{}", cert.pem(), ca_cert.pem()),
            key.serialize_pem(),
        )
    }

    #[test]
    fn reads_a_chain_and_its_key() {
        let (chain, key) = issued(&["shop.example.com", "*.shop.example.com"]);
        let parsed = parse(&chain, &key, Utc::now()).unwrap();
        assert_eq!(parsed.names, ["shop.example.com", "*.shop.example.com"]);
        assert_eq!(parsed.fingerprint.len(), 64);
        assert!(parsed.self_signed);
        assert_eq!(parsed.chain.matches("BEGIN CERTIFICATE").count(), 2);
        // Out of order: refused.
        let (leaf, ca) = chain.split_at(chain.find("-----END CERTIFICATE-----").unwrap() + 26);
        assert!(parse(&format!("{ca}{leaf}"), &key, Utc::now()).is_err());
        // Another key: refused.
        let (_, other) = issued(&["shop.example.com"]);
        assert_eq!(
            parse(&chain, &other, Utc::now()),
            Err("the key is not this certificate's")
        );
        // Expired (its default validity ends in 4096; its authority's too):
        // refused then, the chain first.
        let later = DateTime::parse_from_rfc3339("4097-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(
            parse(&chain, &key, later),
            Err("an intermediate certificate of the chain is expired or not valid yet")
        );
        // The certificate alone: its own date.
        let leaf = &chain[..chain.find("-----END CERTIFICATE-----").unwrap() + 26];
        assert_eq!(parse(leaf, &key, later), Err("the certificate has expired"));
    }

    #[test]
    fn writes_the_store_with_private_keys() {
        let dir = std::env::temp_dir().join(format!("rdp-certs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let cert = |id| Stored {
            id,
            fingerprint: "ab".repeat(32),
            chain: "CHAIN".into(),
            key: "KEY".into(),
        };
        write_store(&dir, "/etc/traefik/dynamic", &[cert(1), cert(2)]).unwrap();
        let yaml = std::fs::read_to_string(dir.join(STORE_FILE)).unwrap();
        assert!(
            yaml.contains("/etc/traefik/dynamic/certs/1-abababababababab.key"),
            "{yaml}"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join("certs/1-abababababababab.key"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // One removed: its files go.
        write_store(&dir, "/etc/traefik/dynamic", &[cert(2)]).unwrap();
        assert!(!dir.join("certs/1-abababababababab.key").exists());
        assert!(dir.join("certs/2-abababababababab.crt").exists());
        // None: an empty list.
        write_store(&dir, "/etc/traefik/dynamic", &[]).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join(STORE_FILE)).unwrap(), "");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn refuses_what_is_not_a_certificate() {
        let now = Utc::now();
        assert!(parse("", "", now).is_err());
        assert!(parse("nope", "nope", now).is_err());
        assert_eq!(
            parse("x", "-----BEGIN ENCRYPTED PRIVATE KEY-----", now),
            Err("the key is encrypted: give it without a passphrase")
        );
        assert!(parse(&"a".repeat(MAX_CHAIN + 1), "", now).is_err());
    }
}
