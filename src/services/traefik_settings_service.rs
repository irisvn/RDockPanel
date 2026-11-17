//! Traefik in the panel (Settings → Traefik): its state, a restart, and the
//! administrators' own middlewares. Those are a YAML map `custom-<name>: {<type>: …}`
//! of the middleware types that only change a request or an answer (headers,
//! limits, redirects, prefixes, compression, …): none that sends requests
//! elsewhere (`forwardAuth`, `errors`, plugins) or reads files of Traefik's
//! (`usersFile`). They are written to `rdp-custom-middlewares.yml` beside
//! the routes (Traefik reloads it: no restart), and a domain uses one as
//! `custom-<name>@file`.

use std::path::Path;

use serde_json::{Value, json};

use crate::services::docker_service::{Docker, DockerResult};
use crate::services::traefik_service::Traefik;

/// The setting holding them (`panel_settings`).
pub const MIDDLEWARES_KEY: &str = "traefik.middlewares";
/// The middlewares imports made (an imported domain's IP allow list), as
/// JSON `{name: definition}`: apart from the administrators' YAML, which they
/// never rewrite; both are written in one file.
pub const IMPORTED_KEY: &str = "traefik.middlewares.imported";
/// Their file, beside the routes (not a service's: never pruned).
pub const MIDDLEWARES_FILE: &str = "rdp-custom-middlewares.yml";
/// Largest YAML accepted.
const MAX_YAML: usize = 32 * 1024;

/// Whether `name` is a custom middleware's: `custom-` then 1 to 48 lowercase
/// letters, digits and `-`.
pub fn valid_name(name: &str) -> bool {
    name.strip_prefix("custom-").is_some_and(|rest| {
        (1..=48).contains(&rest.len())
            && rest.starts_with(|c: char| c.is_ascii_alphanumeric())
            && rest
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    })
}

/// What an option of a middleware type holds.
#[derive(Clone, Copy)]
enum Kind {
    /// A number.
    Num,
    /// A string.
    Str,
    /// A string or a number (a duration, a port).
    StrOrNum,
    Bool,
    /// A list of strings.
    List,
    /// A map of strings (headers).
    Strings,
    /// A small map of the options listed (an IP strategy, a source criterion).
    Map(&'static [(&'static str, Kind)]),
}

const IP_STRATEGY: &[(&str, Kind)] = &[
    ("depth", Kind::Num),
    ("excludedIPs", Kind::List),
    ("ipv6Subnet", Kind::Num),
];
const SOURCE_CRITERION: &[(&str, Kind)] = &[
    ("requestHeaderName", Kind::Str),
    ("requestHost", Kind::Bool),
    ("ipStrategy", Kind::Map(IP_STRATEGY)),
];

/// The middleware types allowed and their options (Traefik v3): each only
/// changes the request or the answer it is given.
const TYPES: &[(&str, &[(&str, Kind)])] = &[
    ("addPrefix", &[("prefix", Kind::Str)]),
    (
        "basicAuth",
        &[
            ("users", Kind::List),
            ("realm", Kind::Str),
            ("removeHeader", Kind::Bool),
            ("headerField", Kind::Str),
        ],
    ),
    (
        "buffering",
        &[
            ("maxRequestBodyBytes", Kind::Num),
            ("memRequestBodyBytes", Kind::Num),
            ("maxResponseBodyBytes", Kind::Num),
            ("memResponseBodyBytes", Kind::Num),
            ("retryExpression", Kind::Str),
        ],
    ),
    ("chain", &[("middlewares", Kind::List)]),
    (
        "circuitBreaker",
        &[
            ("expression", Kind::Str),
            ("checkPeriod", Kind::StrOrNum),
            ("fallbackDuration", Kind::StrOrNum),
            ("recoveryDuration", Kind::StrOrNum),
            ("responseCode", Kind::Num),
        ],
    ),
    (
        "compress",
        &[
            ("excludedContentTypes", Kind::List),
            ("includedContentTypes", Kind::List),
            ("minResponseBodyBytes", Kind::Num),
            ("defaultEncoding", Kind::Str),
            ("encodings", Kind::List),
        ],
    ),
    ("contentType", &[("autoDetect", Kind::Bool)]),
    (
        "digestAuth",
        &[
            ("users", Kind::List),
            ("realm", Kind::Str),
            ("removeHeader", Kind::Bool),
            ("headerField", Kind::Str),
        ],
    ),
    ("grpcWeb", &[("allowOrigins", Kind::List)]),
    (
        "headers",
        &[
            ("customRequestHeaders", Kind::Strings),
            ("customResponseHeaders", Kind::Strings),
            ("accessControlAllowCredentials", Kind::Bool),
            ("accessControlAllowHeaders", Kind::List),
            ("accessControlAllowMethods", Kind::List),
            ("accessControlAllowOriginList", Kind::List),
            ("accessControlAllowOriginListRegex", Kind::List),
            ("accessControlExposeHeaders", Kind::List),
            ("accessControlMaxAge", Kind::Num),
            ("addVaryHeader", Kind::Bool),
            ("allowedHosts", Kind::List),
            ("hostsProxyHeaders", Kind::List),
            ("sslProxyHeaders", Kind::Strings),
            ("stsSeconds", Kind::Num),
            ("stsIncludeSubdomains", Kind::Bool),
            ("stsPreload", Kind::Bool),
            ("forceSTSHeader", Kind::Bool),
            ("frameDeny", Kind::Bool),
            ("customFrameOptionsValue", Kind::Str),
            ("contentTypeNosniff", Kind::Bool),
            ("browserXssFilter", Kind::Bool),
            ("customBrowserXSSValue", Kind::Str),
            ("contentSecurityPolicy", Kind::Str),
            ("contentSecurityPolicyReportOnly", Kind::Str),
            ("publicKey", Kind::Str),
            ("referrerPolicy", Kind::Str),
            ("permissionsPolicy", Kind::Str),
            ("isDevelopment", Kind::Bool),
        ],
    ),
    (
        "inFlightReq",
        &[
            ("amount", Kind::Num),
            ("sourceCriterion", Kind::Map(SOURCE_CRITERION)),
        ],
    ),
    (
        "ipAllowList",
        &[
            ("sourceRange", Kind::List),
            ("ipStrategy", Kind::Map(IP_STRATEGY)),
            ("rejectStatusCode", Kind::Num),
        ],
    ),
    (
        "rateLimit",
        &[
            ("average", Kind::Num),
            ("period", Kind::StrOrNum),
            ("burst", Kind::Num),
            ("sourceCriterion", Kind::Map(SOURCE_CRITERION)),
        ],
    ),
    (
        "redirectRegex",
        &[
            ("regex", Kind::Str),
            ("replacement", Kind::Str),
            ("permanent", Kind::Bool),
        ],
    ),
    (
        "redirectScheme",
        &[
            ("scheme", Kind::Str),
            ("port", Kind::StrOrNum),
            ("permanent", Kind::Bool),
        ],
    ),
    ("replacePath", &[("path", Kind::Str)]),
    (
        "replacePathRegex",
        &[("regex", Kind::Str), ("replacement", Kind::Str)],
    ),
    (
        "retry",
        &[("attempts", Kind::Num), ("initialInterval", Kind::StrOrNum)],
    ),
    (
        "stripPrefix",
        &[("prefixes", Kind::List), ("forceSlash", Kind::Bool)],
    ),
    ("stripPrefixRegex", &[("regex", Kind::List)]),
];

/// Largest configuration once read (aliases expanded).
const MAX_EXPANDED: usize = 64 * 1024;

/// Whether `value` is what `kind` says.
fn fits(value: &Value, kind: Kind) -> bool {
    match kind {
        Kind::Num => value.is_number(),
        Kind::Str => value.is_string(),
        Kind::StrOrNum => value.is_string() || value.is_number(),
        Kind::Bool => value.is_boolean(),
        Kind::List => value
            .as_array()
            .is_some_and(|a| a.iter().all(Value::is_string)),
        Kind::Strings => value
            .as_object()
            .is_some_and(|m| m.values().all(Value::is_string)),
        Kind::Map(options) => value.as_object().is_some_and(|m| {
            m.iter().all(|(k, v)| {
                options
                    .iter()
                    .any(|(name, kind)| name == k && fits(v, *kind))
            })
        }),
    }
}

/// Whether a string of `value` (or a key) holds a Go template's delimiters.
fn has_template(value: &Value) -> bool {
    match value {
        Value::String(s) => s.contains("{{") || s.contains("}}"),
        Value::Array(items) => items.iter().any(has_template),
        Value::Object(map) => map
            .iter()
            .any(|(k, v)| k.contains("{{") || k.contains("}}") || has_template(v)),
        _ => false,
    }
}

/// Reads the administrators' YAML: their middlewares (`None` for none), or
/// why they are refused: a map of `custom-<name>` to one allowed type and
/// its options (of the right kinds), no key twice, no template (once its
/// escapes are read), chains of their own without cycles.
pub fn parse_middlewares(yaml: &str) -> Result<Option<serde_json::Map<String, Value>>, String> {
    parse(yaml, false)
}

/// [`parse_middlewares`] of what was saved: an authentication without users
/// (accepted before Traefik v3.6.19 refused it) is left out, not the others.
fn parse_saved(yaml: &str) -> Result<Option<serde_json::Map<String, Value>>, String> {
    parse(yaml, true)
}

fn parse(yaml: &str, saved: bool) -> Result<Option<serde_json::Map<String, Value>>, String> {
    if yaml.len() > MAX_YAML {
        return Err("too long (32 KB at most)".to_owned());
    }
    if yaml.trim().is_empty() {
        return Ok(None);
    }
    // YAML's own value first: a key given twice is an error there.
    let parsed: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(yaml).map_err(|e| format!("not YAML: {e}"))?;
    let value: Value = serde_json::to_value(&parsed)
        .map_err(|_| "keys are names (text), values YAML's".to_owned())?;
    // Aliases expanded: the size of what Traefik would get.
    if serde_json::to_vec(&value).map_or(true, |v| v.len() > MAX_EXPANDED) {
        return Err("too large once its aliases are expanded".to_owned());
    }
    // Traefik's files are Go templates: none here, escaped or not.
    if has_template(&value) {
        return Err("templates ({{ … }}) are not allowed".to_owned());
    }
    let Value::Object(mut map) = value else {
        return Err("a map of middlewares: `custom-name: {headers: …}`".to_owned());
    };
    let mut without_users = Vec::new();
    for (name, definition) in &map {
        if !valid_name(name) {
            return Err(format!(
                "{name}: names are `custom-` then lowercase letters, digits and -"
            ));
        }
        let Value::Object(definition) = definition else {
            return Err(format!("{name}: one middleware type and its options"));
        };
        if definition.len() != 1 {
            return Err(format!("{name}: exactly one middleware type"));
        }
        let (kind, options) = definition.iter().next().expect("one");
        let Some((_, allowed)) = TYPES.iter().find(|(t, _)| t == kind) else {
            return Err(format!(
                "{name}: {kind} is not allowed here (allowed: {})",
                TYPES.iter().map(|(t, _)| *t).collect::<Vec<_>>().join(", ")
            ));
        };
        let Value::Object(options) = options else {
            return Err(format!(
                "{name}: {kind} takes a map of options ({{}} for none)"
            ));
        };
        for (option, value) in options {
            if option == "usersFile" {
                return Err(format!(
                    "{name}: usersFile is not allowed (give users inline)"
                ));
            }
            match allowed.iter().find(|(o, _)| o == option) {
                Some((_, expected)) if fits(value, *expected) => {}
                Some(_) => {
                    return Err(format!(
                        "{name}: {kind}.{option} has the wrong kind of value"
                    ));
                }
                None => return Err(format!("{name}: {kind} has no option {option}")),
            }
        }
        // Without users Traefik refuses it (v3.6.19+), and every router using it.
        if (kind == "basicAuth" || kind == "digestAuth")
            && options
                .get("users")
                .and_then(Value::as_array)
                .is_none_or(|users| users.is_empty())
        {
            if !saved {
                return Err(format!("{name}: {kind} needs at least one user"));
            }
            without_users.push(name.clone());
        }
        // A chain of the administrators' own only.
        if kind == "chain" {
            let refs = options
                .get("middlewares")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if refs.is_empty()
                || !refs
                    .iter()
                    .all(|r| r.as_str().is_some_and(|r| map.contains_key(r) && r != name))
            {
                return Err(format!(
                    "{name}: a chain lists other custom middlewares of this list"
                ));
            }
        }
    }
    if let Some(name) = chain_cycle(&map) {
        return Err(format!("{name}: its chain comes back to itself"));
    }
    for name in without_users {
        tracing::warn!(
            middleware = name,
            "an authentication without users: not written"
        );
        map.remove(&name);
    }
    Ok(Some(map))
}

/// A middleware whose chain comes back to it, through others.
fn chain_cycle(map: &serde_json::Map<String, Value>) -> Option<String> {
    let next = |name: &str| -> Vec<String> {
        map.get(name)
            .and_then(|d| d.get("chain"))
            .and_then(|c| c.get("middlewares"))
            .and_then(Value::as_array)
            .map(|refs| {
                refs.iter()
                    .filter_map(|r| r.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    };
    for start in map.keys() {
        let mut stack = next(start);
        let mut seen = std::collections::HashSet::new();
        while let Some(name) = stack.pop() {
            if name == *start {
                return Some(start.clone());
            }
            if seen.insert(name.clone()) {
                stack.extend(next(&name));
            }
        }
    }
    None
}

/// Writes `middlewares` for Traefik in `dir` (an empty file for none).
pub fn write_middlewares(
    dir: &Path,
    middlewares: Option<&serde_json::Map<String, Value>>,
) -> std::io::Result<()> {
    let yaml = match middlewares {
        Some(map) if !map.is_empty() => {
            serde_yaml_ng::to_string(&json!({ "http": { "middlewares": map } }))
                .map_err(std::io::Error::other)?
        }
        _ => String::new(),
    };
    std::fs::create_dir_all(dir)?;
    crate::services::certificate_service::write_private(dir, MIDDLEWARES_FILE, yaml.as_bytes())
}

/// The middlewares imports made, each checked as the administrators' are.
pub async fn imported<C: sea_orm::ConnectionTrait>(
    db: &C,
) -> Result<serde_json::Map<String, Value>, String> {
    let saved = crate::models::panel_setting::get(db, IMPORTED_KEY)
        .await
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    if saved.trim().is_empty() {
        return Ok(serde_json::Map::new());
    }
    Ok(parse_saved(&saved).unwrap_or(None).unwrap_or_default())
}

/// A name for an import's middleware (`custom-imp-<12 hex>`): from what it is
/// for, so that the same import made again gets the same name.
pub fn imported_name(seed: &str) -> String {
    use sha2::{Digest, Sha256};
    let hash: String = Sha256::digest(seed.as_bytes())
        .iter()
        .take(6)
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("custom-imp-{hash}")
}

/// The name of an imported domain's IP allow list (`definition`): the same
/// list, one middleware.
pub fn allow_name(definition: &Value) -> String {
    imported_name(&serde_json::to_string(definition).unwrap_or_default())
}

/// Keeps the imports' middlewares in `txn`: adds `add`, drops those no
/// domain uses any more (an import undone, a domain changed). Locks the
/// setting first: two writers never lose one's change. Returns the names
/// added and dropped (empty: nothing changed).
pub async fn keep_imported<C: sea_orm::ConnectionTrait>(
    txn: &C,
    add: &serde_json::Map<String, Value>,
) -> Result<(Vec<String>, Vec<String>), sea_orm::DbErr> {
    use crate::models::{panel_setting, service_domain};
    use sea_orm::sea_query::OnConflict;
    use sea_orm::{EntityTrait, QueryFilter, QuerySelect, Set};
    // The row first (a first writer waits for another's insert; one there
    // keeps its value: only its time is touched), then locked.
    panel_setting::Entity::insert(panel_setting::ActiveModel {
        key: Set(IMPORTED_KEY.to_owned()),
        value: Set(String::new()),
        updated_at: Set(chrono::Utc::now()),
    })
    .on_conflict(
        OnConflict::column(panel_setting::Column::Key)
            .update_column(panel_setting::Column::UpdatedAt)
            .to_owned(),
    )
    .exec_without_returning(txn)
    .await?;
    panel_setting::Entity::find_by_id(IMPORTED_KEY.to_owned())
        .lock_exclusive()
        .one(txn)
        .await?;
    let mut map = imported(txn).await.map_err(sea_orm::DbErr::Custom)?;
    let used: std::collections::BTreeSet<String> = service_domain::Entity::find()
        .filter(sea_orm::ColumnTrait::is_not_null(
            &service_domain::Column::Middlewares,
        ))
        .all(txn)
        .await?
        .into_iter()
        .flat_map(|d| {
            d.middlewares
                .unwrap_or_default()
                .split(',')
                // `name@file`, or the bare name (a file router's own provider).
                .filter_map(|m| match m.trim().split_once('@') {
                    Some((name, "file")) => Some(name.to_owned()),
                    Some(_) => None,
                    None => Some(m.trim().to_owned()),
                })
                .collect::<Vec<_>>()
        })
        .collect();
    let dropped: Vec<String> = map
        .keys()
        .filter(|name| !used.contains(*name) && !add.contains_key(*name))
        .cloned()
        .collect();
    for name in &dropped {
        map.remove(name);
    }
    let mut added = Vec::new();
    for (name, definition) in add {
        if map.insert(name.clone(), definition.clone()).is_none() {
            added.push(name.clone());
        }
    }
    if !added.is_empty() || !dropped.is_empty() {
        let text = serde_json::to_string(&map).unwrap_or_default();
        if text.len() > MAX_YAML {
            return Err(sea_orm::DbErr::Custom(
                "the imported middlewares would pass 32 KB".to_owned(),
            ));
        }
        panel_setting::set(txn, IMPORTED_KEY, &text).await?;
    }
    Ok((added, dropped))
}

/// Whether `definition` (`{"ipAllowList": {...}}`) is one the panel takes.
pub fn valid_definition(name: &str, definition: &Value) -> bool {
    let text = serde_json::to_string(&json!({ name: definition })).unwrap_or_default();
    matches!(parse_middlewares(&text), Ok(Some(_)))
}

/// Writes the middlewares kept in the database (at boot, after a save).
pub async fn sync_middlewares(
    db: &sea_orm::DatabaseConnection,
    traefik: &Traefik,
) -> Result<(), String> {
    // Read and written one at a time: an older read never lands last.
    static FILE: crate::services::certificate_service::Generations =
        crate::services::certificate_service::Generations::new();
    let generation = FILE.read();
    let yaml = crate::models::panel_setting::get(db, MIDDLEWARES_KEY)
        .await
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    // Kept only once checked: a check failing now writes none.
    let mut map = parse_saved(&yaml).unwrap_or(None).unwrap_or_default();
    // Then the imports' (an administrator's of the same name wins).
    for (name, definition) in imported(db).await? {
        map.entry(name).or_insert(definition);
    }
    let map = (!map.is_empty()).then_some(map);
    let dir = traefik.dir.clone();
    tokio::task::spawn_blocking(move || {
        FILE.write(generation, || write_middlewares(&dir, map.as_ref()))
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("the middlewares cannot be written: {e}"))
}

/// Traefik's state, as the page shows it: its image, published ports, its
/// tasks (state, error) and its last update's state. Nothing else of its
/// specification (arguments, variables).
pub async fn status(docker: &Docker, traefik: &Traefik) -> DockerResult<Value> {
    let api = docker.engine();
    let Some(service) = api.inspect_service(&traefik.service).await? else {
        return Ok(json!({ "service": traefik.service, "found": false }));
    };
    let spec = service.spec.clone().unwrap_or_default();
    let image = spec
        .task_template
        .as_ref()
        .and_then(|t| t.container_spec.as_ref())
        .and_then(|c| c.image.clone())
        .unwrap_or_default();
    // `traefik:v3.7@sha256:…`: shown without its digest.
    let shown = image.split('@').next().unwrap_or_default().to_owned();
    let ports: Vec<Value> = spec
        .endpoint_spec
        .as_ref()
        .and_then(|e| e.ports.clone())
        .unwrap_or_default()
        .into_iter()
        .map(|p| json!({ "published": p.published_port, "target": p.target_port }))
        .collect();
    let mut tasks: Vec<Value> = api
        .list_tasks(&traefik.service)
        .await?
        .into_iter()
        .filter(|t| {
            t.desired_state
                .as_ref()
                .is_some_and(|d| d.to_string() == "running")
        })
        .map(|t| {
            let status = t.status.unwrap_or_default();
            json!({
                "state": status.state.map(|s| s.to_string()),
                "since": status.timestamp,
                "error": status.err,
            })
        })
        .collect();
    tasks.truncate(10);
    let update = service
        .update_status
        .and_then(|u| u.state)
        .map(|s| s.to_string());
    Ok(json!({
        "service": traefik.service,
        "found": true,
        "image": shown,
        "ports": ports,
        "tasks": tasks,
        "update": update,
    }))
}

/// Restarts Traefik: its tasks replaced (Swarm, one at a time). Whether it did.
pub async fn restart(docker: &Docker, traefik: &Traefik) -> DockerResult<bool> {
    traefik
        .update_service(docker.engine(), |spec| {
            let template = spec.task_template.get_or_insert_with(Default::default);
            template.force_update = Some(template.force_update.unwrap_or(0) + 1);
            true
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert!(valid_name("custom-ratelimit"));
        assert!(valid_name("custom-a1-b"));
        for bad in [
            "ratelimit",
            "custom-",
            "custom-A",
            "custom--x",
            "custom-x_y",
            "custom-x@file",
        ] {
            assert!(!valid_name(bad), "{bad}");
        }
    }

    #[test]
    fn middlewares_checked() {
        let ok = "custom-limit:\n  rateLimit:\n    average: 50\ncustom-sec:\n  headers:\n    frameDeny: true\ncustom-both:\n  chain:\n    middlewares: [custom-limit, custom-sec]\n";
        let map = parse_middlewares(ok).unwrap().unwrap();
        assert_eq!(map.len(), 3);
        // Saved before Traefik refused it: that one left out, the others kept.
        let older = "custom-limit:\n  rateLimit:\n    average: 50\ncustom-auth:\n  basicAuth:\n    users: []\n";
        let kept = parse_saved(older).unwrap().unwrap();
        assert_eq!(kept.keys().collect::<Vec<_>>(), ["custom-limit"]);
        assert_eq!(parse_middlewares("  ").unwrap(), None);
        for (bad, why) in [
            (
                "custom-x:\n  forwardAuth:\n    address: http://evil\n",
                "not allowed",
            ),
            ("custom-x:\n  errors:\n    service: x\n", "not allowed"),
            ("limit:\n  rateLimit: {average: 1}\n", "names"),
            (
                "custom-x:\n  basicAuth:\n    usersFile: /etc/passwd\n",
                "usersFile",
            ),
            (
                "custom-x:\n  basicAuth:\n    users: []\n",
                "at least one user",
            ),
            ("custom-x:\n  digestAuth: {}\n", "at least one user"),
            ("custom-x:\n  headers: {}\n  compress: {}\n", "exactly one"),
            (
                "custom-x:\n  chain:\n    middlewares: [panel-auth@file]\n",
                "chain",
            ),
            (
                "custom-x:\n  headers:\n    customRequestHeaders: {X: '{{ env \"A\" }}'}\n",
                "templates",
            ),
            ("- a\n- b\n", "map"),
            ("custom-x: [", "YAML"),
            (
                "custom-x:\n  headers:\n    customRequestHeaders: {X: \"\\x7b\\x7b env \\x7d\\x7d\"}\n",
                "templates",
            ),
            (
                "custom-x:\n  rateLimit:\n    average: wrong\n",
                "wrong kind",
            ),
            ("custom-x:\n  headers: 42\n", "map of options"),
            ("custom-x:\n  headers:\n    nope: true\n", "no option"),
            (
                "custom-x:\n  compress: {}\ncustom-x:\n  headers: {}\n",
                "YAML",
            ),
            (
                "custom-a:\n  chain:\n    middlewares: [custom-b]\ncustom-b:\n  chain:\n    middlewares: [custom-a]\n",
                "back to itself",
            ),
        ] {
            let err = parse_middlewares(bad).unwrap_err();
            assert!(err.contains(why), "{bad}: {err}");
        }
    }

    #[test]
    fn aliases_count_once_expanded() {
        let big = "x".repeat(2_000);
        let mut yaml =
            format!("custom-a:\n  headers:\n    customRequestHeaders: &h {{X: \"{big}\"}}\n");
        for i in 0..40 {
            yaml.push_str(&format!(
                "custom-b{i}:\n  headers:\n    customRequestHeaders: *h\n"
            ));
        }
        assert!(yaml.len() < MAX_YAML);
        assert!(parse_middlewares(&yaml).unwrap_err().contains("expanded"));
    }

    #[test]
    fn writes_their_file() {
        let dir = std::env::temp_dir().join(format!("rdp-mw-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let map = parse_middlewares("custom-gz:\n  compress: {}\n").unwrap();
        write_middlewares(&dir, map.as_ref()).unwrap();
        let yaml = std::fs::read_to_string(dir.join(MIDDLEWARES_FILE)).unwrap();
        assert!(
            yaml.contains("custom-gz") && yaml.contains("middlewares"),
            "{yaml}"
        );
        write_middlewares(&dir, None).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join(MIDDLEWARES_FILE)).unwrap(),
            ""
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
