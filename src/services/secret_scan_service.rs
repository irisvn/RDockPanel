//! Finds secrets in what Docker reports about a container and replaces them,
//! so that what the panel shows never carries a password: env values,
//! command lines (`valkey-server --requirepass …`), health checks
//! (`mysqladmin ping -p…`) and labels (Traefik's basic-auth users). Every
//! reader that shows a container's configuration goes through here.

use std::collections::HashMap;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

/// What a secret is replaced with.
pub const REDACTED: &str = "******";

/// Env names containing one of these hold a secret.
const SECRET_MARKERS: [&str; 9] = [
    "PASSWORD",
    "PASSWD",
    "PASS",
    "PWD",
    "SECRET",
    "TOKEN",
    "KEY",
    "CREDENTIAL",
    "AUTH",
];

/// Label names holding a secret: narrower than env names, since labels such
/// as `org.opencontainers.image.authors` are common and harmless.
const LABEL_MARKERS: [&str; 9] = [
    "PASSWORD",
    "PASSWD",
    "SECRET",
    "TOKEN",
    "CREDENTIAL",
    "APIKEY",
    "API_KEY",
    "PRIVATE_KEY",
    "AUTHORIZATION",
];

/// Traefik middlewares whose label value is a list of `user:hash`.
const LABEL_USERS: [&str; 2] = ["basicauth.users", "digestauth.users"];

/// Known secret values shorter than this are not searched for elsewhere:
/// they would match ordinary words.
const MIN_KNOWN: usize = 4;

/// What a flag name holds a secret when it contains (lowercase).
const FLAG_MARKERS: [&str; 7] = [
    "pass",
    "pwd",
    "secret",
    "token",
    "key",
    "auth",
    "credential",
];

static USERINFO: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)([a-z][a-z0-9+.-]*://[^\s/:@]*):([^\s@]+)@").expect("valid regex")
});
static FLAG_VALUE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)(--?[a-z0-9_.-]*?(?:pass|pwd|secret|token|key|auth|credential)[a-z0-9_.-]*)(=|[ \t]+)("[^"]*"|'[^']*'|[^\s"'-][^\s"']*)"#,
    )
    .expect("valid regex")
});
static NAME_VALUE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)\b([a-z0-9_]*(?:pass|pwd|secret|token|key|auth|credential)[a-z0-9_]*)=("[^"]*"|'[^']*'|[^\s"']+)"#,
    )
    .expect("valid regex")
});
static SHORT_P: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(^|\s)-p("[^"]*"|'[^']*'|[^\s"']+)"#).expect("valid regex"));
/// `Authorization: Bearer x`, `X-Api-Key: x`.
static HEADER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)\b((?:proxy-)?authorization|x-[a-z-]*(?:key|token|secret)[a-z-]*)(\s*:\s*)((?:bearer|basic|token)\s+)?("[^"]*"|'[^']*'|[^\s"']+)"#,
    )
    .expect("valid regex")
});
/// `"password":"x"` in a JSON text.
static JSON_PAIR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)("[a-z0-9_.-]*(?:pass|pwd|secret|token|key|auth|credential)[a-z0-9_.-]*"\s*:\s*)"[^"]*""#,
    )
    .expect("valid regex")
});
/// `redis-cli -a x` (and valkey-cli): `-a` is the password only there.
static CLI_A: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"((?:redis|valkey)-cli\b[^;&|]*?\s-a[ \t]+)("[^"]*"|'[^']*'|[^\s"']+)"#)
        .expect("valid regex")
});
static USER_PASS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(^|\s)(-u|--user)(=|\s+)([^\s:"']+):([^\s"']+)"#).expect("valid regex")
});

/// Whether an env name holds a secret (for what is shown: broad, a harmless
/// name may be hidden too).
pub fn secret_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    SECRET_MARKERS.iter().any(|marker| upper.contains(marker))
}

/// Whether an env name holds a secret, by its words (`DB_PASSWORD`,
/// `GITHUB_TOKEN`, `APIKEY`): for an import, which moves secrets out of a
/// command and blocks on them, so a name that only contains a marker
/// (`PASSENGER_APP_ENV`, `KEYCLOAK_ADMIN`, `OAUTH_ENABLED`, the shell's
/// `PWD`) is not one.
pub fn secret_env_name(name: &str) -> bool {
    const WORDS: [&str; 13] = [
        "PASSWORD",
        "PASSWD",
        "PASS",
        "PWD",
        "SECRET",
        "TOKEN",
        "KEY",
        "APIKEY",
        "CREDENTIAL",
        "CREDENTIALS",
        "AUTH",
        "PRIVATE",
        "PASSPHRASE",
    ];
    const ENDINGS: [&str; 5] = ["PASSWORD", "PASSWD", "SECRET", "TOKEN", "APIKEY"];
    let upper = name.to_ascii_uppercase();
    if matches!(upper.as_str(), "PWD" | "OLDPWD") {
        return false;
    }
    upper
        .split(['_', '-', '.'])
        .any(|word| WORDS.contains(&word) || ENDINGS.iter().any(|e| word.ends_with(e)))
}

/// Whether an option or label name holds a secret, by its parts (`.`, `_`,
/// `-` apart): `splunk-token`, `app.db.pass`, `X-Api-Key`.
fn secret_key(name: &str) -> bool {
    const WORDS: [&str; 11] = [
        "PASSWORD",
        "PASSWD",
        "PASS",
        "SECRET",
        "TOKEN",
        "KEY",
        "APIKEY",
        "CREDENTIAL",
        "CREDENTIALS",
        "AUTHORIZATION",
        "PASSPHRASE",
    ];
    name.to_ascii_uppercase()
        .split(['_', '-', '.'])
        .any(|word| WORDS.contains(&word))
}

/// Replaces the secrets of a container's inspect document: `Config.Env`,
/// `Path`, `Args`, `Config.Cmd`, `Config.Entrypoint`,
/// `Config.Healthcheck.Test` and `Config.Labels`. A value of a secret env
/// is also replaced wherever else it appears (`--requirepass` given the
/// value of `REDIS_PASSWORD`).
pub fn redact_container(doc: &mut Value) {
    let known = known_secrets(doc);
    if let Some(Value::Array(env)) = doc.pointer_mut("/Config/Env") {
        for entry in env {
            if let Some(text) = entry.as_str()
                && let Some(redacted) = redact_env_entry(text)
            {
                *entry = Value::String(redacted);
            }
        }
    }
    if let Some(Value::String(path)) = doc.get_mut("Path") {
        *path = redact_text(path, &known);
    }
    for pointer in [
        "/Args",
        "/Config/Cmd",
        "/Config/Entrypoint",
        "/Config/Healthcheck/Test",
    ] {
        match doc.pointer_mut(pointer) {
            Some(Value::Array(items)) => redact_value_args(items, &known),
            Some(Value::String(text)) => *text = redact_text(text, &known),
            _ => {}
        }
    }
    if let Some(Value::Object(labels)) = doc.pointer_mut("/Config/Labels") {
        for (name, value) in labels.iter_mut() {
            if let Some(text) = value.as_str() {
                *value = Value::String(redact_label(name, text));
            }
        }
    }
    // Everything else (log driver options, volume driver options, …).
    redact_leaves(doc, &known);
}

/// Replaces, in every string of `doc`, the known secrets, URL passwords,
/// `NAME=value` with a secret name and JSON secret fields.
fn redact_leaves(doc: &mut Value, known: &[String]) {
    match doc {
        Value::String(text) => {
            let mut out = text.clone();
            for secret in known.iter().filter(|secret| secret.len() >= MIN_KNOWN) {
                out = out.replace(secret.as_str(), REDACTED);
            }
            let out = USERINFO.replace_all(&out, format!("${{1}}:{REDACTED}@"));
            let out = NAME_VALUE.replace_all(&out, format!("${{1}}={REDACTED}"));
            *text = JSON_PAIR
                .replace_all(&out, format!("${{1}}\"{REDACTED}\""))
                .into_owned();
        }
        Value::Array(items) => items.iter_mut().for_each(|item| redact_leaves(item, known)),
        // An option named as a secret (`LogConfig.Config.splunk-token`): its
        // value, whatever it looks like.
        Value::Object(map) => map.iter_mut().for_each(|(key, item)| {
            if item.is_string() && secret_key(key) {
                *item = Value::String(REDACTED.to_owned());
            } else {
                redact_leaves(item, known);
            }
        }),
        _ => {}
    }
}

/// Replaces the secret values of labels (a container list's labels).
pub fn redact_labels(labels: &mut HashMap<String, String>) {
    for (name, value) in labels.iter_mut() {
        *value = redact_label(name, value);
    }
}

/// A command line with its secrets replaced: the value after a secret flag
/// (`--requirepass x`, `--password=x`), `NAME=value` with a secret name,
/// `-p<password>`, `-u user:password`, URL passwords and the `known`
/// values.
pub fn redact_args(args: &[String], known: &[String]) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len());
    let mut hide_next = false;
    // After a secret flag, a value may start with `-` (`--password -x!9`,
    // `--password -xyz123`): hidden unless it is a long flag (`--port`).
    let flag_like = |a: &str| {
        a.starts_with("--")
            && a.trim_start_matches('-')
                .split('=')
                .next()
                .is_some_and(|n| {
                    !n.is_empty()
                        && n.bytes().all(|b| {
                            b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.'
                        })
                })
    };
    for arg in args {
        if hide_next && !flag_like(arg) {
            out.push(REDACTED.to_owned());
            hide_next = false;
            continue;
        }
        hide_next = false;
        // `redis-cli -a x`: the value of `-a` after redis-cli/valkey-cli.
        if arg == "-a"
            && out
                .iter()
                .any(|prev: &String| prev.ends_with("redis-cli") || prev.ends_with("valkey-cli"))
        {
            out.push(arg.clone());
            hide_next = true;
            continue;
        }
        // A Go-style long flag (`-password x`): a name, not `-p` + a value.
        if let Some(name) = arg.strip_prefix('-')
            && name.len() > 2
            && name.bytes().all(|b| b.is_ascii_alphabetic())
            && FLAG_MARKERS
                .iter()
                .any(|marker| name.to_ascii_lowercase().contains(marker))
        {
            out.push(arg.clone());
            hide_next = true;
            continue;
        }
        if arg == "-u" || arg == "--user" {
            // `-u user:password`: only the password part, below.
            out.push(arg.clone());
            continue;
        }
        // A long flag (`--requirepass`) takes its value from the next item;
        // a short one carries it (`-pSecret`, handled as text).
        if let Some(name) = arg.strip_prefix("--")
            && !arg.contains('=')
        {
            let name = name.to_ascii_lowercase();
            hide_next = FLAG_MARKERS.iter().any(|marker| name.contains(marker));
        }
        out.push(redact_text(arg, known));
    }
    // `-u user:password` as two items.
    for i in 1..out.len() {
        let flag = out[i - 1].as_str();
        if (flag == "-u" || flag == "--user")
            && let Some((user, _)) = out[i].split_once(':')
        {
            out[i] = format!("{user}:{REDACTED}");
        }
    }
    out
}

/// One argument of a command, as it can be written again without its secrets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArgPart {
    /// Kept as it is.
    Plain(String),
    /// `prefix` + the value of a secret (`--password=`, `-p`, or nothing).
    Secret { prefix: String, value: String },
}

/// The arguments of a command with each secret set apart, so that the secret
/// can move to an environment variable and the command read it from there.
/// `None` when a secret sits inside a longer argument in a way that cannot be
/// cut out (a URL's password, a known value in the middle of a word).
pub fn split_secret_args(args: &[String], known: &[String]) -> Option<Vec<ArgPart>> {
    let hidden = redact_args(args, known);
    args.iter()
        .zip(&hidden)
        .map(|(arg, redacted)| {
            if redacted == arg {
                Some(ArgPart::Plain(arg.clone()))
            } else if redacted == REDACTED {
                Some(ArgPart::Secret {
                    prefix: String::new(),
                    value: arg.clone(),
                })
            } else {
                // `--password=x` or `-px`: what comes before the mask is kept.
                let prefix = redacted.strip_suffix(REDACTED)?;
                let value = arg.strip_prefix(prefix)?;
                (!value.is_empty()).then(|| ArgPart::Secret {
                    prefix: prefix.to_owned(),
                    value: value.to_owned(),
                })
            }
        })
        .collect()
}

/// A text (a shell command, one argument) with its secrets replaced.
pub fn redact_text(text: &str, known: &[String]) -> String {
    let mut out = text.to_owned();
    for secret in known {
        if secret.len() >= MIN_KNOWN {
            out = out.replace(secret.as_str(), REDACTED);
        }
    }
    let out = USERINFO.replace_all(&out, format!("${{1}}:{REDACTED}@"));
    // Before the flags: `-pSecret` is a password, not a flag named `pSecret`.
    let out = CLI_A.replace_all(&out, format!("${{1}}{REDACTED}"));
    let out = SHORT_P.replace_all(&out, format!("${{1}}-p{REDACTED}"));
    let out = HEADER.replace_all(&out, format!("${{1}}${{2}}${{3}}{REDACTED}"));
    let out = JSON_PAIR.replace_all(&out, format!("${{1}}\"{REDACTED}\""));
    let out = FLAG_VALUE.replace_all(&out, format!("${{1}}${{2}}{REDACTED}"));
    let out = NAME_VALUE.replace_all(&out, format!("${{1}}={REDACTED}"));
    let out = USER_PASS.replace_all(&out, format!("${{1}}${{2}}${{3}}${{4}}:{REDACTED}"));
    out.into_owned()
}

/// `NAME=value` with the value replaced when the name looks secret, else the
/// password of a URL's userinfo (`DATABASE_URL=mysql://u:******@db/x`);
/// `None` when nothing changes.
fn redact_env_entry(text: &str) -> Option<String> {
    let (name, value) = text.split_once('=')?;
    if secret_name(name) {
        return Some(format!("{name}={REDACTED}"));
    }
    let redacted = USERINFO.replace_all(value, format!("${{1}}:{REDACTED}@"));
    (redacted != value).then(|| format!("{name}={redacted}"))
}

fn redact_label(name: &str, value: &str) -> String {
    // `X-Api-Key`, `api_key` and `apikey` are the same marker.
    let upper = name.to_ascii_uppercase().replace(['-', '_', '.'], "");
    let lower = name.to_ascii_lowercase();
    if LABEL_MARKERS
        .iter()
        .any(|marker| upper.contains(&marker.replace('_', "")))
        || LABEL_USERS.iter().any(|users| lower.ends_with(users))
        || secret_key(name)
    {
        return REDACTED.to_owned();
    }
    USERINFO
        .replace_all(value, format!("${{1}}:{REDACTED}@"))
        .into_owned()
}

fn redact_value_args(items: &mut [Value], known: &[String]) {
    let args: Vec<String> = items
        .iter()
        .map(|item| item.as_str().unwrap_or_default().to_owned())
        .collect();
    for (item, redacted) in items.iter_mut().zip(redact_args(&args, known)) {
        if item.is_string() {
            *item = Value::String(redacted);
        }
    }
}

/// The values of the container's secret env, and the passwords of URLs in
/// its env: searched for in its command and health check.
fn known_secrets(doc: &Value) -> Vec<String> {
    let Some(Value::Array(env)) = doc.pointer("/Config/Env") else {
        return Vec::new();
    };
    let mut known = Vec::new();
    for entry in env.iter().filter_map(Value::as_str) {
        let Some((name, value)) = entry.split_once('=') else {
            continue;
        };
        if secret_name(name) && !value.is_empty() {
            known.push(value.to_owned());
        }
        for caps in USERINFO.captures_iter(value) {
            if let Some(password) = caps.get(2) {
                known.push(password.as_str().to_owned());
            }
        }
    }
    // Longest first, so that a secret containing another is replaced whole.
    known.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
    known.dedup();
    known
}

// ---------------------------------------------------------------- Imports
//
// What an import moves out of a command or a health check is decided here,
// precisely (what is shown in Inspect is hidden more broadly above): by the
// program each word belongs to, so that `pg_isready -p 5432` (a port) or
// `mongosh --authenticationDatabase admin` (a name) are not secrets, and a
// reference to a variable (`-p$MYSQL_ROOT_PASSWORD`) is not one either.

/// What a command holds, for an import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cut {
    /// No secret.
    Clean,
    /// Its words, each secret set apart (it can move to the environment).
    Parts(Vec<ArgPart>),
    /// A secret the panel cannot set apart (inside a URL, a nested script, a
    /// word that holds more than it).
    Stuck,
}

/// Programs whose `-p<value>` (attached) is a password; elsewhere `-p` is a
/// port (pg_isready, psql, redis-cli, postgres) or a vhost (rabbitmqctl).
const P_IS_PASSWORD: [&str; 10] = [
    "mysql",
    "mysqladmin",
    "mysqldump",
    "mysqlcheck",
    "mariadb",
    "mariadb-admin",
    "mariadb-dump",
    "mariadb-check",
    "mongo",
    "mongosh",
];

/// Long flags whose value is a secret (their name, without dashes).
const SECRET_FLAGS: [&str; 14] = [
    "password",
    "pass",
    "passwd",
    "requirepass",
    "masterauth",
    "token",
    "secret",
    "api-key",
    "apikey",
    "auth-token",
    "access-key",
    "secret-key",
    "secret-access-key",
    "pwd",
];

/// Whether a long flag (its name, without dashes) takes a secret: a part of
/// its name is a secret word (`--db-password`, `--access-token`,
/// `--api-key`), and it does not name a file, a path, a database or a switch
/// (`--password-file`, `--authenticationDatabase`, `--no-auth-warning`).
fn secret_flag(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if SECRET_FLAGS.contains(&lower.as_str()) {
        return true;
    }
    let parts: Vec<&str> = lower.split(['-', '_']).filter(|p| !p.is_empty()).collect();
    let secret = parts.iter().any(|p| {
        matches!(
            *p,
            "password"
                | "passwd"
                | "pass"
                | "pwd"
                | "secret"
                | "token"
                | "apikey"
                | "credential"
                | "credentials"
        )
    }) || lower.contains("apikey")
        || lower.ends_with("password")
        || (parts.contains(&"api") && parts.contains(&"key"))
        || (parts.contains(&"access") && parts.contains(&"key"))
        || (parts.contains(&"secret") && parts.contains(&"key"));
    let not = parts.first() == Some(&"no")
        || parts.last().is_some_and(|p| {
            matches!(
                *p,
                "file"
                    | "path"
                    | "dir"
                    | "warning"
                    | "database"
                    | "db"
                    | "source"
                    | "mechanism"
                    | "type"
                    | "length"
                    | "env"
                    | "name"
                    | "id"
            )
        });
    secret && !not
}

/// A word that only names a variable (`$NAME`, `${NAME}`): not a secret.
fn reference(word: &str) -> bool {
    let inner = word
        .strip_prefix("${")
        .and_then(|w| w.strip_suffix('}'))
        .or_else(|| word.strip_prefix('$'));
    inner.is_some_and(|name| {
        let mut chars = name.chars();
        chars
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

fn program_name(word: &str) -> String {
    word.rsplit('/').next().unwrap_or(word).to_ascii_lowercase()
}

/// The words of one command line (argv), each secret set apart. `program` is
/// the program the first words go to (the image's entrypoint for a command
/// that starts with options).
fn cut_words(words: &[String], known: &[String], program: Option<&str>) -> Cut {
    let mut parts: Vec<ArgPart> = Vec::with_capacity(words.len());
    let mut current: Option<String> = program.map(program_name);
    let mut next_is_secret = false;
    let mut found = false;
    let secret = |prefix: &str, value: &str| ArgPart::Secret {
        prefix: prefix.to_owned(),
        value: value.to_owned(),
    };
    for word in words {
        if next_is_secret {
            next_is_secret = false;
            // The next flag (`--auth --bind_ip_all`): no value was given.
            if !reference(word) && !word.is_empty() && !word.starts_with("--") {
                parts.push(secret("", word));
                found = true;
                continue;
            }
        }
        // `NAME=value` with a secret name (`PGPASSWORD=…`, `-Ex.password=…`).
        if let Some((key, value)) = word.split_once('=') {
            let name = key.trim_start_matches('-');
            let name = name
                .strip_prefix('E')
                .or_else(|| name.strip_prefix('D'))
                .filter(|_| key.starts_with('-') && !key.starts_with("--"))
                .unwrap_or(name);
            let long = key.strip_prefix("--").map(str::to_ascii_lowercase);
            // `--auth=<value>` (a bare `--auth` is a switch, as mongod's).
            let flag_secret = long
                .as_deref()
                .is_some_and(|f| secret_flag(f) || f == "auth");
            // `-E…password=` (Elasticsearch), `-D…password=` (Java).
            let env_secret = key.starts_with('-')
                && !key.starts_with("--")
                && name.split(['.', '_', '-']).any(|w| {
                    matches!(
                        w.to_ascii_uppercase().as_str(),
                        "PASSWORD" | "PASSWD" | "PASS" | "SECRET" | "TOKEN" | "APIKEY"
                    )
                })
                || (!key.starts_with('-')
                    && key
                        .bytes()
                        .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
                    && secret_env_name(name));
            if (flag_secret || env_secret) && !value.is_empty() && !reference(value) {
                parts.push(secret(&format!("{key}="), value));
                found = true;
                continue;
            }
        }
        if let Some(flag) = word.strip_prefix("--")
            && !flag.contains('=')
            && secret_flag(flag)
        {
            parts.push(ArgPart::Plain(word.clone()));
            next_is_secret = true;
            continue;
        }
        let program = current.clone().unwrap_or_default();
        // `mysql -p<password>`; a bare `-p` asks for it (no value follows).
        if let Some(value) = word.strip_prefix("-p")
            && P_IS_PASSWORD.contains(&program.as_str())
            && !value.is_empty()
            && !word.starts_with("--")
        {
            if reference(value) {
                parts.push(ArgPart::Plain(word.clone()));
            } else {
                parts.push(secret("-p", value));
                found = true;
            }
            continue;
        }
        // `mongosh -p <password>`, `redis-cli -a <password>`.
        if (word == "-p" && matches!(program.as_str(), "mongo" | "mongosh"))
            || (word == "-a" && matches!(program.as_str(), "redis-cli" | "valkey-cli"))
        {
            parts.push(ArgPart::Plain(word.clone()));
            next_is_secret = true;
            continue;
        }
        // `-u user:password`, `--user user:password` (curl, wget).
        if let Some(last) = parts.last()
            && matches!(last, ArgPart::Plain(w) if w == "-u" || w == "--user")
            && let Some((user, pass)) = word.split_once(':')
            && !pass.is_empty()
            && !reference(pass)
        {
            parts.push(secret(&format!("{user}:"), pass));
            found = true;
            continue;
        }
        // A known secret, whole.
        if known.iter().any(|k| k.len() >= MIN_KNOWN && k == word) {
            parts.push(secret("", word));
            found = true;
            continue;
        }
        // Inside a word (a URL's password, a known secret in the middle, a
        // nested script): it cannot be set apart.
        if known
            .iter()
            .any(|k| k.len() >= MIN_KNOWN && word.contains(k.as_str()))
            || USERINFO.is_match(word)
            || (word.contains(char::is_whitespace)
                && !matches!(cut_shell(word, known, None), Cut::Clean))
        {
            return Cut::Stuck;
        }
        if current.is_none() && !word.starts_with('-') && !word.contains('=') {
            current = Some(program_name(word));
        }
        parts.push(ArgPart::Plain(word.clone()));
    }
    if found { Cut::Parts(parts) } else { Cut::Clean }
}

/// An exec-form command (argv), for an import.
pub fn cut_argv(args: &[String], known: &[String], entrypoint: &[String]) -> Cut {
    // A command that starts with an option goes to the image's entrypoint.
    let program = entrypoint
        .iter()
        .find(|w| {
            !w.ends_with(".sh") && !matches!(program_name(w).as_str(), "tini" | "dumb-init" | "--")
        })
        .map(String::as_str);
    cut_words(args, known, program)
}

/// A word of a shell line: its text as written (`raw`) and as the shell
/// reads it (`word`), or an operator (`&&`, `||`, `;`, `|`, `&`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellWord {
    pub raw: String,
    pub word: String,
    pub operator: bool,
}

/// Splits a shell line into words and operators, keeping each word as
/// written. `None` for what it does not follow (a substitution `$(…)` or
/// backticks holding a quote, an unclosed quote).
pub fn shell_words_of(line: &str) -> Option<Vec<ShellWord>> {
    let mut out = Vec::new();
    let mut chars = line.chars().peekable();
    let mut raw = String::new();
    let mut word = String::new();
    let flush = |raw: &mut String, word: &mut String, out: &mut Vec<ShellWord>| {
        if !raw.is_empty() {
            out.push(ShellWord {
                raw: std::mem::take(raw),
                word: std::mem::take(word),
                operator: false,
            });
        }
    };
    while let Some(c) = chars.next() {
        match c {
            ' ' | '\t' => flush(&mut raw, &mut word, &mut out),
            // Lines are not followed (continuations, an operator ending one).
            '\n' => return None,
            // A redirection (`>/dev/null`, `2>&1`): a word of its own, never
            // part of the word before it (`secret>/dev/null`).
            '>' | '<' => {
                // Inside one redirection already (`>>`, `&>`, `2>&`): goes on.
                let redirecting = raw
                    .chars()
                    .all(|c| c.is_ascii_digit() || matches!(c, '>' | '<' | '&'));
                if !raw.is_empty() && !redirecting {
                    flush(&mut raw, &mut word, &mut out);
                }
                raw.push(c);
                word.push(c);
            }
            // `2>&1`, `>&2`: part of the redirection.
            '&' if raw.ends_with(['>', '<']) => {
                raw.push(c);
                word.push(c);
            }
            // `&>` starts one (the word before ends).
            '&' if chars.peek() == Some(&'>') => {
                flush(&mut raw, &mut word, &mut out);
                raw.push(c);
                word.push(c);
            }
            ';' | '|' | '&' => {
                flush(&mut raw, &mut word, &mut out);
                let mut op = c.to_string();
                if (c == '|' || c == '&') && chars.peek() == Some(&c) {
                    op.push(chars.next()?);
                }
                out.push(ShellWord {
                    raw: op.clone(),
                    word: op,
                    operator: true,
                });
            }
            '\'' => {
                raw.push(c);
                loop {
                    let q = chars.next()?;
                    raw.push(q);
                    if q == '\'' {
                        break;
                    }
                    word.push(q);
                }
            }
            '"' => {
                raw.push(c);
                loop {
                    let q = chars.next()?;
                    raw.push(q);
                    if q == '"' {
                        break;
                    }
                    if q == '\\' {
                        // Inside "…" a backslash escapes only $ ` " \ and a
                        // newline; before anything else it stays.
                        let e = chars.next()?;
                        if e == '\n' {
                            return None;
                        }
                        raw.push(e);
                        if !matches!(e, '$' | '`' | '"' | '\\') {
                            word.push('\\');
                        }
                        word.push(e);
                        continue;
                    }
                    if q == '`' || (q == '$' && chars.peek() == Some(&'(')) {
                        return None;
                    }
                    word.push(q);
                }
            }
            '\\' => {
                raw.push(c);
                let e = chars.next()?;
                if e == '\n' {
                    return None;
                }
                raw.push(e);
                word.push(e);
            }
            '`' | '(' | ')' => return None,
            '$' if chars.peek() == Some(&'(') => return None,
            _ => {
                raw.push(c);
                word.push(c);
            }
        }
    }
    flush(&mut raw, &mut word, &mut out);
    Some(out)
}

/// A shell line (`CMD-SHELL`), for an import: each simple command cut by its
/// own program. The parts are the words as written (`Plain` keeps `raw`).
pub fn cut_shell(line: &str, known: &[String], program: Option<&str>) -> Cut {
    let Some(words) = shell_words_of(line) else {
        // Not followed: a secret anywhere in it cannot be set apart.
        let hidden = redact_text(line, known);
        return if hidden == line {
            Cut::Clean
        } else {
            Cut::Stuck
        };
    };
    let mut parts = Vec::new();
    let mut found = false;
    // Compound commands (`if …; then`, loops, groups): not followed.
    const KEYWORDS: [&str; 14] = [
        "if", "then", "else", "elif", "fi", "while", "until", "do", "done", "for", "case", "esac",
        "{", "}",
    ];
    if words
        .iter()
        .any(|w| !w.operator && KEYWORDS.contains(&w.raw.as_str()))
    {
        let hidden = redact_text(line, known);
        return if hidden == line {
            Cut::Clean
        } else {
            Cut::Stuck
        };
    }
    let mut segment: Vec<&ShellWord> = Vec::new();
    let mut run = |segment: &mut Vec<&ShellWord>, parts: &mut Vec<ArgPart>| -> bool {
        let words: Vec<String> = segment.iter().map(|w| w.word.clone()).collect();
        // Leading `VAR=x`, `exec`, `env`: the program is the word after.
        let first = words
            .iter()
            .position(|w| !(w.contains('=') && !w.starts_with('-')) && w != "exec" && w != "env");
        let program = first.map(|i| words[i].as_str()).or(program);
        match cut_words(&words, known, program) {
            Cut::Stuck => return false,
            Cut::Clean => parts.extend(segment.iter().map(|w| ArgPart::Plain(w.raw.clone()))),
            Cut::Parts(cut) => {
                found = true;
                for (w, part) in segment.iter().zip(cut) {
                    parts.push(match part {
                        ArgPart::Plain(_) => ArgPart::Plain(w.raw.clone()),
                        secret => secret,
                    });
                }
            }
        }
        segment.clear();
        true
    };
    for w in &words {
        if w.operator {
            if !run(&mut segment, &mut parts) {
                return Cut::Stuck;
            }
            parts.push(ArgPart::Plain(w.raw.clone()));
        } else {
            segment.push(w);
        }
    }
    if !run(&mut segment, &mut parts) {
        return Cut::Stuck;
    }
    if found { Cut::Parts(parts) } else { Cut::Clean }
}

/// A shell line with each secret replaced by a variable the shell reads
/// (`-p"$RDP_SECRET_HC_1"`); the names from `name(n)`, the values given back.
pub fn rewrite_shell(
    parts: &[ArgPart],
    mut name: impl FnMut() -> String,
) -> (String, Vec<(String, String)>) {
    let mut out = Vec::new();
    let mut moved = Vec::new();
    for part in parts {
        match part {
            ArgPart::Plain(raw) => out.push(raw.clone()),
            ArgPart::Secret { prefix, value } => {
                let var = name();
                // Kept as written when plain (`PGPASSWORD=`, `-p`, `--password=`):
                // quoted, an assignment would become a command's name.
                let plain = prefix.chars().all(|c| {
                    c.is_ascii_alphanumeric()
                        || matches!(c, '_' | '-' | '.' | '/' | ':' | '=' | '+' | '@' | ',')
                });
                let prefix = if plain {
                    prefix.clone()
                } else {
                    shell_words::quote(prefix).into_owned()
                };
                out.push(format!("{prefix}\"${var}\""));
                moved.push((var, value.clone()));
            }
        }
    }
    (out.join(" "), moved)
}

#[cfg(test)]
mod tests {

    fn words(line: &str) -> Vec<String> {
        shell_words::split(line).unwrap()
    }

    #[test]
    fn an_import_knows_the_secrets_of_common_health_checks() {
        let none: [String; 0] = [];
        // Not secrets: ports, names, references, server options.
        for clean in [
            "pg_isready -U postgres -p 5432",
            "pg_isready -p5432",
            "redis-cli -p 6379 ping",
            "mysqladmin ping -h 127.0.0.1 -p$MYSQL_ROOT_PASSWORD",
            "mysqladmin ping -h localhost -p\"${MYSQL_ROOT_PASSWORD}\"",
            "healthcheck.sh --connect --innodb_initialized",
            "mongosh --quiet --authenticationDatabase admin --eval 'db.runCommand(\"ping\").ok'",
            "rabbitmq-diagnostics -q ping",
            "redis-cli --no-auth-warning ping",
            "curl -f http://localhost:9000/minio/health/live",
            "mysql -uroot -p -e 'select 1'",
        ] {
            assert_eq!(cut_shell(clean, &none, None), Cut::Clean, "{clean}");
        }
        // Secrets the panel moves.
        for (line, value) in [
            (
                "mysqladmin ping -h 127.0.0.1 -uroot -pRootPw9 --silent",
                "RootPw9",
            ),
            ("redis-cli -a s3cr3t ping", "s3cr3t"),
            ("redis-cli --pass s3cr3t ping", "s3cr3t"),
            ("PGPASSWORD=pw1234 pg_isready -U app", "pw1234"),
            ("mongosh -u root -p 'hunter22' --eval 1", "hunter22"),
            ("curl -u admin:letmein http://localhost/health", "letmein"),
            ("valkey-cli --requirepass=abc123 ping", "abc123"),
        ] {
            let Cut::Parts(parts) = cut_shell(line, &none, None) else {
                panic!("{line}");
            };
            let (rewritten, moved) = rewrite_shell(&parts, || "RDP_SECRET_HC_1".to_owned());
            assert_eq!(
                moved,
                vec![("RDP_SECRET_HC_1".to_owned(), value.to_owned())],
                "{line}"
            );
            assert!(!rewritten.contains(value), "{rewritten}");
            assert!(rewritten.contains("\"$RDP_SECRET_HC_1\""), "{rewritten}");
        }
        // Flags named after a secret, whatever their prefix; not a file's.
        for (line, value) in [
            ("server --db-password=S3cr3tA", "S3cr3tA"),
            ("server --access-token S3cr3tB", "S3cr3tB"),
        ] {
            let Cut::Parts(parts) = cut_shell(line, &none, None) else {
                panic!("{line}");
            };
            let (rewritten, moved) = rewrite_shell(&parts, || "V".to_owned());
            assert_eq!(moved[0].1, value, "{line}");
            assert!(!rewritten.contains(value), "{rewritten}");
        }
        for clean in [
            "server --password-file /run/secrets/db",
            "mongod --auth --bind_ip_all",
        ] {
            assert_eq!(cut_shell(clean, &none, None), Cut::Clean, "{clean}");
        }
        // Lines and continuations are not followed: a secret in one cannot be cut.
        for stuck in [
            "true\nmysqladmin ping -pS3cr3tC",
            "redis-cli -a sec\\\nret ping",
        ] {
            assert_eq!(cut_shell(stuck, &none, None), Cut::Stuck, "{stuck:?}");
        }
        // A redirection attached to a word is not part of it.
        let Cut::Parts(parts) = cut_shell("redis-cli -a S3cr3tF>/dev/null", &none, None) else {
            panic!();
        };
        let (rewritten, moved) = rewrite_shell(&parts, || "V".to_owned());
        assert_eq!(moved[0].1, "S3cr3tF");
        assert_eq!(rewritten, "redis-cli -a \"$V\" >/dev/null");
        // Compound redirections stay whole.
        for (line, tail) in [
            ("redis-cli -a S3cr3tG ping >>/dev/null", ">>/dev/null"),
            ("redis-cli -a S3cr3tG ping &>/dev/null", "&>/dev/null"),
            ("redis-cli -a S3cr3tG ping 2>&1", "2>&1"),
        ] {
            let Cut::Parts(parts) = cut_shell(line, &none, None) else {
                panic!("{line}");
            };
            let (rewritten, _) = rewrite_shell(&parts, || "V".to_owned());
            assert_eq!(
                rewritten,
                format!("redis-cli -a \"$V\" ping {tail}"),
                "{line}"
            );
        }
        // Compound commands are not followed: a secret in one cannot be cut.
        assert_eq!(
            cut_shell("if mysqladmin ping -pS3cr3tD; then exit 0; fi", &none, None),
            Cut::Stuck
        );
        // Redirections and escapes stay as the shell reads them.
        let Cut::Parts(parts) = cut_shell("redis-cli -a S3cr3tE ping >/dev/null 2>&1", &none, None)
        else {
            panic!();
        };
        let (rewritten, _) = rewrite_shell(&parts, || "V".to_owned());
        assert_eq!(rewritten, "redis-cli -a \"$V\" ping >/dev/null 2>&1");
        let Cut::Parts(parts) = cut_shell("redis-cli -a \"pa\\ss\" ping", &none, None) else {
            panic!();
        };
        let (_, moved) = rewrite_shell(&parts, || "V".to_owned());
        assert_eq!(moved[0].1, "pa\\ss");
        // An assignment stays one (not quoted into a command's name).
        let Cut::Parts(parts) = cut_shell("PGPASSWORD=pw1234 pg_isready -U app", &none, None)
        else {
            panic!();
        };
        let (rewritten, _) = rewrite_shell(&parts, || "RDP_SECRET_HC_1".to_owned());
        assert_eq!(
            rewritten,
            "PGPASSWORD=\"$RDP_SECRET_HC_1\" pg_isready -U app"
        );
        // A URL's password or a nested script: cannot be set apart.
        for stuck in [
            "curl -f http://user:pa55word@localhost/health",
            "sh -c 'mysqladmin ping -psecretpw'",
        ] {
            assert_eq!(cut_shell(stuck, &none, None), Cut::Stuck, "{stuck}");
        }
        // Server options of an argv command are not secrets either.
        let args = words("postgres -c password_encryption=scram-sha-256 -p 5433");
        assert_eq!(cut_argv(&args, &none, &[]), Cut::Clean);
    }

    #[test]
    fn an_import_knows_a_secret_name_by_its_words() {
        for name in [
            "DB_PASSWORD",
            "GITHUB_TOKEN",
            "APIKEY",
            "AWS_SECRET_ACCESS_KEY",
            "SMTP_PASS",
            "MYSQLPASSWORD",
        ] {
            assert!(secret_env_name(name), "{name}");
        }
        for name in [
            "PASSENGER_APP_ENV",
            "KEYCLOAK_ADMIN",
            "OAUTH_ENABLED",
            "PWD",
            "OLDPWD",
            "BYPASS_CACHE",
        ] {
            assert!(!secret_env_name(name), "{name}");
        }
    }

    #[test]
    fn secrets_named_by_their_option_or_label_are_hidden() {
        let mut doc = serde_json::json!({
            "HostConfig": {"LogConfig": {"Config": {"splunk-token": "opaqueSensitiveValue", "splunk-url": "https://s.io"}}},
            "Config": {"Labels": {"app.db.pass": "x1", "com.example.api.key": "abc", "org.opencontainers.image.authors": "me"}},
        });
        redact_container(&mut doc);
        let out = doc.to_string();
        for secret in ["opaqueSensitiveValue", "\"x1\"", "\"abc\""] {
            assert!(!out.contains(secret), "{out}");
        }
        assert!(
            out.contains("https://s.io") && out.contains("\"me\""),
            "{out}"
        );
    }

    #[test]
    fn a_secret_value_that_starts_with_a_dash_or_holds_a_slash_is_hidden() {
        let args: Vec<String> = ["app", "--password", "-xyz123", "--port", "80"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        let out = redact_args(&args, &[]);
        assert_eq!(out[2], REDACTED, "{out:?}");
        assert_eq!(out[4], "80");
        let text = redact_text("https://u:pa/ss@db.io/x", &[]);
        assert!(!text.contains("pa/ss"), "{text}");
    }

    use serde_json::json;

    use super::*;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn replaces_secret_env_and_url_passwords() {
        let mut doc = json!({"Config": {"Env": [
            "PATH=/bin", "DB_PASSWORD=hunter2", "api_key=x", "NOEQUALS",
            "DATABASE_URL=mysql://panel:hunter2@db:3306/panel", "REDIS_URL=redis://:pw@cache",
            "HOME_URL=https://x.io/a@b"
        ]}});
        redact_container(&mut doc);
        assert_eq!(
            doc["Config"]["Env"],
            json!([
                "PATH=/bin",
                "DB_PASSWORD=******",
                "api_key=******",
                "NOEQUALS",
                "DATABASE_URL=mysql://panel:******@db:3306/panel",
                "REDIS_URL=redis://:******@cache",
                "HOME_URL=https://x.io/a@b"
            ])
        );
    }

    #[test]
    fn replaces_the_value_after_a_secret_flag() {
        assert_eq!(
            redact_args(
                &strings(&["valkey-server", "--requirepass", "s3cr3t-v4lue"]),
                &[]
            ),
            strings(&["valkey-server", "--requirepass", REDACTED])
        );
        assert_eq!(
            redact_args(
                &strings(&["app", "--password=abc", "--api-token", "t0k", "--verbose"]),
                &[]
            ),
            strings(&[
                "app",
                "--password=******",
                "--api-token",
                REDACTED,
                "--verbose"
            ])
        );
        // A secret flag followed by another flag hides nothing.
        assert_eq!(
            redact_args(&strings(&["app", "--auth", "--debug"]), &[]),
            strings(&["app", "--auth", "--debug"])
        );
    }

    #[test]
    fn replaces_secrets_inside_shell_commands() {
        let shell = "PGPASSWORD=pg1234 psql -h db && mysqladmin ping -uroot -pTopSecret1 \
                     && curl -u admin:letmein https://user:urlpass@x.io/ && valkey-cli --requirepass 'q w'";
        let out = redact_text(shell, &[]);
        for secret in ["pg1234", "TopSecret1", "letmein", "urlpass", "q w"] {
            assert!(!out.contains(secret), "{secret} in {out}");
        }
        assert!(out.contains("PGPASSWORD=******"), "{out}");
        assert!(out.contains("-p******"), "{out}");
        assert!(out.contains("-u admin:******"), "{out}");
        assert!(out.contains("https://user:******@x.io/"), "{out}");
    }

    #[test]
    fn replaces_known_secret_values_anywhere() {
        let mut doc = json!({
            "Path": "redis-server",
            "Args": ["--masterauth-file", "/run/x", "--port", "6379", "plain-ValueOfSecret"],
            "Config": {
                "Env": ["REDIS_PASSWORD=ValueOfSecret"],
                "Cmd": ["redis-server", "-a", "ValueOfSecret"],
                "Healthcheck": {"Test": ["CMD", "redis-cli", "-a", "ValueOfSecret", "ping"]},
            }
        });
        redact_container(&mut doc);
        let text = doc.to_string();
        assert!(!text.contains("ValueOfSecret"), "{text}");
        assert_eq!(doc["Args"][3], json!("6379"));
    }

    #[test]
    fn replaces_flags_whose_name_starts_with_the_marker() {
        // Review 0.11.1 #1, #2, #5: the text form of a health check or `sh -c`.
        let shell = "redis-cli --pass s3cretA ping; mysql --password s3cretB; \
                     cloudflared run --token eyJs3cretC; app --secret s3cretD --auth s3cretE; \
                     redis-cli -a s3cretF ping; mysqladmin -p'Top s3cretG'";
        let out = redact_text(shell, &[]);
        for secret in [
            "s3cretA", "s3cretB", "s3cretC", "s3cretD", "s3cretE", "s3cretF", "s3cretG",
        ] {
            assert!(!out.contains(secret), "{secret} in {out}");
        }
        // Single-dash long flags (Go style) and `-a` of redis-cli, as items.
        assert_eq!(
            redact_args(
                &strings(&["app", "-password", "hunter2", "-token", "t0k3n"]),
                &[]
            ),
            strings(&["app", "-password", REDACTED, "-token", REDACTED])
        );
        assert_eq!(
            redact_args(
                &strings(&["CMD", "redis-cli", "-a", "s3cretF", "ping"]),
                &[]
            ),
            strings(&["CMD", "redis-cli", "-a", REDACTED, "ping"])
        );
        assert_eq!(
            redact_args(&strings(&["ls", "-a", "/data"]), &[]),
            strings(&["ls", "-a", "/data"])
        );
    }

    #[test]
    fn replaces_headers_and_json_fields() {
        let text = r#"curl -H 'Authorization: Bearer abc123' -H "X-Api-Key: key456" -d '{"password":"hunter22","name":"x"}'"#;
        let out = redact_text(text, &[]);
        for secret in ["abc123", "key456", "hunter22"] {
            assert!(!out.contains(secret), "{secret} in {out}");
        }
        assert!(out.contains(r#""name":"x""#), "{out}");
    }

    #[test]
    fn replaces_secrets_anywhere_in_the_document() {
        // Review 0.11.1 #3, #4: outside the command fields, and header labels.
        let mut doc = json!({
            "Config": {
                "Env": ["API_TOKEN=Tok3nValue"],
                "Labels": {
                    "x.customrequestheaders.X-Api-Key": "LabelKey1",
                    "x.headers.Authorization": "Basic LabelAuth2",
                    "x.note": "uses Tok3nValue"
                }
            },
            "HostConfig": {
                "LogConfig": {"Config": {"loki-url": "https://u:LogPass3@loki/push"}},
                "Mounts": [{"VolumeOptions": {"DriverConfig": {"Options": {"o": "username=u,password=CifsPass4"}}}}]
            }
        });
        redact_container(&mut doc);
        let text = doc.to_string();
        for secret in [
            "Tok3nValue",
            "LabelKey1",
            "LabelAuth2",
            "LogPass3",
            "CifsPass4",
        ] {
            assert!(!text.contains(secret), "{secret} in {text}");
        }
    }

    #[test]
    fn secrets_are_set_apart_when_they_can_be_cut_out() {
        let parts = split_secret_args(
            &strings(&[
                "valkey-server",
                "--requirepass",
                "s3cr3t-value",
                "--port=6379",
                "--auth=tok3n",
            ]),
            &[],
        )
        .unwrap();
        assert_eq!(
            parts,
            [
                ArgPart::Plain("valkey-server".to_owned()),
                ArgPart::Plain("--requirepass".to_owned()),
                ArgPart::Secret {
                    prefix: String::new(),
                    value: "s3cr3t-value".to_owned()
                },
                ArgPart::Plain("--port=6379".to_owned()),
                ArgPart::Secret {
                    prefix: "--auth=".to_owned(),
                    value: "tok3n".to_owned()
                },
            ]
        );
        // A secret inside a URL cannot be cut out of its argument.
        assert_eq!(
            split_secret_args(&strings(&["app", "redis://:hunter22@cache"]), &[]),
            None
        );
        // Nothing to hide: every part plain.
        let plain = split_secret_args(&strings(&["nginx", "-g", "daemon off;"]), &[]).unwrap();
        assert!(plain.iter().all(|p| matches!(p, ArgPart::Plain(_))));
    }

    #[test]
    fn leaves_ordinary_commands_alone() {
        for args in [
            strings(&["postgres", "-c", "max_connections=100"]),
            strings(&["nginx", "-g", "daemon off;"]),
            strings(&["mkdir", "-p", "/data"]),
            strings(&["node", "dist/server.js", "--port", "3000"]),
        ] {
            assert_eq!(redact_args(&args, &[]), args);
        }
    }

    #[test]
    fn replaces_secret_labels_only() {
        let mut labels = HashMap::from([
            (
                "traefik.http.middlewares.a.basicauth.users".to_owned(),
                "admin:$apr1$xyz".to_owned(),
            ),
            ("app.api_key".to_owned(), "k".to_owned()),
            (
                "org.opencontainers.image.authors".to_owned(),
                "Jane".to_owned(),
            ),
            (
                "com.docker.compose.project".to_owned(),
                "openarchiver".to_owned(),
            ),
            ("backup.target".to_owned(), "s3://id:pw@bucket".to_owned()),
        ]);
        redact_labels(&mut labels);
        assert_eq!(
            labels["traefik.http.middlewares.a.basicauth.users"],
            REDACTED
        );
        assert_eq!(labels["app.api_key"], REDACTED);
        assert_eq!(labels["org.opencontainers.image.authors"], "Jane");
        assert_eq!(labels["com.docker.compose.project"], "openarchiver");
        assert_eq!(labels["backup.target"], "s3://id:******@bucket");
    }
}
