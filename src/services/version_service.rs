//! Settings → Version: what the panel runs, whether a newer release is out,
//! and what changed. Releases only (tags `X.Y.Z` of
//! `ghcr.io/irisvn/rdockpanel`): a `main` image is a development build,
//! never compared. The panel only checks and says so; an update is the
//! user's (the installer's command, with that version).
//!
//! The newest release is the highest `X.Y.Z` tag the registry lists (asked
//! anonymously, as `docker pull` does); its notes are GitHub's release of
//! that tag. The last successful check is kept (`panel_settings`): a failed
//! one keeps it, marked stale, and never counts as "up to date". Checks
//! are scheduled every 6 hours; one asked by hand waits [`COOLDOWN`] after
//! the last, and checks never run two at once.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use rustcms::{AppResult, AppState};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::models::panel_setting;

/// The panel's image repository (the registry's and GitHub's).
pub const REPOSITORY: &str = "irisvn/rdockpanel";
/// What a check found, as last done (JSON of [`Check`]).
const CHECK_KEY: &str = "version.check";
/// The version the panel last started as (a change is the history's).
const SEEN_KEY: &str = "version.seen";
/// A user's dismissed notice: `version.dismissed.<user id>` = the version.
const DISMISSED_PREFIX: &str = "version.dismissed.";
/// Between two checks asked by hand.
pub const COOLDOWN: Duration = Duration::from_secs(60);
const TIMEOUT: Duration = Duration::from_secs(15);
/// Release notes kept (the newest first), each cut at [`NOTES_MAX`].
const RELEASES_KEPT: usize = 10;
const NOTES_MAX: usize = 8_000;
/// The kept check's JSON at most (a TEXT column holds 65 535 bytes).
const KEPT_MAX: usize = 60_000;

/// When the last check ended.
static LAST: Mutex<Option<Instant>> = Mutex::new(None);
static CHECKING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// A release: its version and GitHub's notes (Markdown, shown as text).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Release {
    pub version: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub published_at: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
}

/// What the last check found.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Check {
    /// When it last succeeded.
    #[serde(default)]
    pub checked_at: Option<DateTime<Utc>>,
    /// The newest release then.
    #[serde(default)]
    pub latest: Option<String>,
    #[serde(default)]
    pub releases: Vec<Release>,
    /// GitHub's ETag of the releases (asked again only if changed).
    #[serde(default)]
    pub etag: Option<String>,
    /// Why the last attempt failed (the result above is then stale).
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub error_at: Option<DateTime<Utc>>,
    /// GitHub not asked before this (its rate limit's reset).
    #[serde(default)]
    pub github_after: Option<DateTime<Utc>>,
}

/// `X.Y.Z` (a leading `v` allowed), nothing else (no pre-release).
pub fn parse(version: &str) -> Option<(u64, u64, u64)> {
    let mut parts = version.strip_prefix('v').unwrap_or(version).split('.');
    let (a, b, c) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    let number = |s: &str| {
        (!s.is_empty() && s.len() <= 9 && s.bytes().all(|b| b.is_ascii_digit()))
            .then(|| s.parse().ok())
            .flatten()
    };
    Some((number(a)?, number(b)?, number(c)?))
}

/// The highest release among `tags`.
pub fn newest<'a>(tags: impl IntoIterator<Item = &'a str>) -> Option<String> {
    tags.into_iter()
        .filter(|t| !t.starts_with('v'))
        .filter_map(|t| parse(t).map(|v| (v, t)))
        .max_by_key(|(v, _)| *v)
        .map(|(_, t)| t.to_owned())
}

/// What the panel runs as: `release` (a version's image), `development`
/// (`main`, `main-<sha>`, any other tag) or `source` (not installed).
pub fn channel(image: Option<&str>) -> &'static str {
    let Some(image) = image else {
        return "source";
    };
    let reference = image.split('@').next().unwrap_or(image);
    let (name, tag) = reference
        .rsplit_once(':')
        .filter(|(_, tag)| !tag.contains('/'))
        .unwrap_or((reference, "latest"));
    let ours = name == format!("ghcr.io/{REPOSITORY}");
    let release = tag == "latest"
        || parse(tag).is_some()
        || (tag.split('.').count() == 2 && tag.split('.').all(|p| p.parse::<u64>().is_ok()));
    if ours && release {
        "release"
    } else {
        "development"
    }
}

/// Whether `latest` is newer than `running` (both `X.Y.Z`).
pub fn newer(latest: &str, running: &str) -> bool {
    matches!((parse(latest), parse(running)), (Some(l), Some(r)) if l > r)
}

/// The releases after `running` up to `latest` (the newest first).
pub fn between<'a>(releases: &'a [Release], running: &str, latest: &str) -> Vec<&'a Release> {
    let (Some(from), Some(to)) = (parse(running), parse(latest)) else {
        return vec![];
    };
    let mut found: Vec<_> = releases
        .iter()
        .filter(|r| parse(&r.version).is_some_and(|v| v > from && v <= to))
        .collect();
    found.sort_by_key(|r| std::cmp::Reverse(parse(&r.version)));
    found
}

/// The last check (empty if none).
pub async fn last<C: sea_orm::ConnectionTrait>(db: &C) -> Check {
    match panel_setting::get(db, CHECK_KEY).await {
        Ok(Some(json)) => serde_json::from_str(&json).unwrap_or_default(),
        _ => Check::default(),
    }
}

fn client() -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(TIMEOUT)
        .user_agent(concat!("rdockpanel/", env!("RDP_VERSION")))
        .build()
}

/// The repository's tags, asked as `docker pull` does (an anonymous pull
/// token), every page.
async fn tags(client: &reqwest::Client) -> Result<Vec<String>, String> {
    #[derive(Deserialize)]
    struct Token {
        token: String,
    }
    #[derive(Deserialize)]
    struct Tags {
        #[serde(default)]
        tags: Option<Vec<String>>,
    }
    let token: Token = client
        .get(format!(
            "https://ghcr.io/token?service=ghcr.io&scope=repository:{REPOSITORY}:pull"
        ))
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| format!("the registry's token: {e}"))?
        .json()
        .await
        .map_err(|e| format!("the registry's token: {e}"))?;
    let mut all = Vec::new();
    let mut url = format!("https://ghcr.io/v2/{REPOSITORY}/tags/list?n=1000");
    for _ in 0..20 {
        let response = client
            .get(&url)
            .bearer_auth(&token.token)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|e| format!("the registry's tags: {e}"))?;
        // The next page: `Link: </v2/…/tags/list?last=…&n=…>; rel="next"`.
        let next = response
            .headers()
            .get(reqwest::header::LINK)
            .and_then(|v| v.to_str().ok())
            .and_then(next_page);
        let page: Tags = response
            .json()
            .await
            .map_err(|e| format!("the registry's tags: {e}"))?;
        all.extend(page.tags.unwrap_or_default());
        match next {
            Some(next) => url = next,
            None => break,
        }
    }
    Ok(all)
}

/// The `rel="next"` page of a registry's `Link` header, on ghcr.io only.
fn next_page(link: &str) -> Option<String> {
    link.split(',').find_map(|part| {
        let (target, params) = part.trim().split_once(';')?;
        // `rel=next`, `rel="next"`, `rel="next prev"`: the token `next`.
        let next = params.split(';').any(|p| {
            p.trim()
                .strip_prefix("rel")
                .map(str::trim_start)
                .and_then(|p| p.strip_prefix('='))
                .is_some_and(|v| {
                    v.trim()
                        .trim_matches('"')
                        .split_whitespace()
                        .any(|t| t.eq_ignore_ascii_case("next"))
                })
        });
        if !next {
            return None;
        }
        let target = target.trim().strip_prefix('<')?.strip_suffix('>')?;
        if target.starts_with("/v2/") {
            Some(format!("https://ghcr.io{target}"))
        } else if target.starts_with("https://ghcr.io/v2/") {
            Some(target.to_owned())
        } else {
            None
        }
    })
}

/// Why GitHub's releases were not read.
enum NotRead {
    /// Its rate limit: again after this.
    Limited(Option<DateTime<Utc>>),
    Failed(String),
}

/// GitHub's releases (their notes), unless unchanged since `etag`: then
/// `None`.
async fn releases(
    client: &reqwest::Client,
    etag: Option<&str>,
) -> Result<Option<(Vec<Release>, Option<String>)>, NotRead> {
    #[derive(Deserialize)]
    struct GitHubRelease {
        tag_name: String,
        name: Option<String>,
        body: Option<String>,
        published_at: Option<String>,
        html_url: Option<String>,
        #[serde(default)]
        draft: bool,
        #[serde(default)]
        prerelease: bool,
    }
    let mut request = client
        .get(format!(
            "https://api.github.com/repos/{REPOSITORY}/releases?per_page=100"
        ))
        .header(reqwest::header::ACCEPT, "application/vnd.github+json");
    if let Some(etag) = etag {
        request = request.header(reqwest::header::IF_NONE_MATCH, etag);
    }
    let response = request
        .send()
        .await
        .map_err(|e| NotRead::Failed(format!("GitHub's releases: {e}")))?;
    if response.status() == reqwest::StatusCode::NOT_MODIFIED {
        return Ok(None);
    }
    if response.status() == reqwest::StatusCode::FORBIDDEN
        || response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS
    {
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<i64>().ok())
        };
        let reset = header("retry-after")
            .map(|secs| Utc::now() + chrono::Duration::seconds(secs))
            .or_else(|| header("x-ratelimit-reset").and_then(|t| DateTime::from_timestamp(t, 0)));
        return Err(NotRead::Limited(reset));
    }
    let response = response
        .error_for_status()
        .map_err(|e| NotRead::Failed(format!("GitHub's releases: {e}")))?;
    let etag = response
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let list: Vec<GitHubRelease> = response
        .json()
        .await
        .map_err(|e| NotRead::Failed(format!("GitHub's releases: {e}")))?;
    let mut found: Vec<Release> = list
        .into_iter()
        .filter(|r| !r.draft && !r.prerelease)
        .filter_map(|r| {
            let version = r
                .tag_name
                .strip_prefix('v')
                .unwrap_or(&r.tag_name)
                .to_owned();
            parse(&version)?;
            Some(Release {
                version,
                name: r.name,
                notes: r.body.map(|b| b.chars().take(NOTES_MAX).collect()),
                published_at: r.published_at,
                url: r.html_url,
            })
        })
        .collect();
    found.sort_by_key(|r| std::cmp::Reverse(parse(&r.version)));
    found.truncate(RELEASES_KEPT);
    Ok(Some((found, etag)))
}

/// Why a check was not made now.
#[derive(Debug, PartialEq)]
pub enum Refused {
    /// Asked by hand within [`COOLDOWN`] of the last: again after this.
    TooSoon(Duration),
}

/// Checks now (`by_hand`: within the cooldown, refused) and keeps what it
/// found; the check as kept.
pub async fn check(app: &AppState, by_hand: bool) -> AppResult<Result<Check, Refused>> {
    // One at a time: a second asked meanwhile gets the first's result.
    let started = Instant::now();
    let _checking = CHECKING.lock().await;
    // Done while this one waited: that result; asked by hand too soon:
    // refused; else this one runs.
    let done_meanwhile = {
        let at = LAST.lock().expect("not poisoned");
        match *at {
            Some(when) if when >= started => true,
            Some(when) if by_hand && when.elapsed() < COOLDOWN => {
                return Ok(Err(Refused::TooSoon(COOLDOWN - when.elapsed())));
            }
            _ => false,
        }
    };
    if done_meanwhile {
        return Ok(Ok(last(app.db()).await));
    }
    let mut kept = last(app.db()).await;
    let found = async {
        let client = client().map_err(|e| e.to_string())?;
        let tags = tags(&client).await?;
        let latest = newest(tags.iter().map(String::as_str))
            .ok_or_else(|| "the registry lists no release".to_owned())?;
        // GitHub's rate limit kept to: its notes as they were until its reset.
        let releases = match kept.github_after {
            Some(after) if after > Utc::now() => Err(NotRead::Limited(Some(after))),
            _ => releases(&client, kept.etag.as_deref()).await,
        };
        Ok::<_, String>((latest, releases))
    }
    .await;
    match found {
        Ok((latest, releases)) => {
            kept.latest = Some(latest);
            kept.checked_at = Some(Utc::now());
            match releases {
                Ok(Some((releases, etag))) => {
                    kept.github_after = None;
                    kept.releases = releases;
                    kept.etag = etag;
                    kept.error = None;
                    kept.error_at = None;
                }
                Ok(None) => {
                    kept.error = None;
                    kept.error_at = None;
                }
                // The version is known; its notes are not (said so).
                Err(NotRead::Limited(after)) => {
                    kept.github_after = after;
                    kept.error = Some(match after {
                        Some(at) => format!(
                            "GitHub's rate limit: the release notes again after {}",
                            at.format("%H:%M UTC")
                        ),
                        None => "GitHub's rate limit: the release notes later".to_owned(),
                    });
                    kept.error_at = Some(Utc::now());
                }
                Err(NotRead::Failed(why)) => {
                    kept.error = Some(why);
                    kept.error_at = Some(Utc::now());
                }
            }
        }
        Err(why) => {
            tracing::warn!(error = why, "the version check failed");
            kept.error = Some(why);
            kept.error_at = Some(Utc::now());
        }
    }
    // Within what `panel_settings` keeps (a TEXT column): older notes go.
    let mut json = serde_json::to_string(&kept).map_err(rustcms::AppError::internal)?;
    while json.len() > KEPT_MAX && !kept.releases.is_empty() {
        kept.releases.pop();
        json = serde_json::to_string(&kept).map_err(rustcms::AppError::internal)?;
    }
    // Ended once kept: a waiting caller then reads this one (not kept: it
    // checks again).
    panel_setting::set(app.db(), CHECK_KEY, &json).await?;
    *LAST.lock().expect("not poisoned") = Some(Instant::now());
    Ok(Ok(kept))
}

/// Settings → Version: this panel, the last check, the update (its notes and
/// the installer's command for it), what it runs with, and the history.
pub async fn status(app: &AppState, installed: bool) -> AppResult<Value> {
    use crate::services::docker_service::Docker;
    let docker = app.service::<Docker>();
    let own = match docker.as_deref() {
        Some(docker) if installed => crate::services::system_service::own_image(docker)
            .await
            .ok()
            .flatten(),
        _ => None,
    };
    let image = own.as_ref().map(|(image, _)| image.clone());
    let channel = channel(if installed { image.as_deref() } else { None });
    let check = last(app.db()).await;
    let running = env!("RDP_VERSION");
    let update = update_of(&check, channel);
    let notes: Vec<&Release> = match &update {
        Some(latest) => between(&check.releases, running, latest),
        None => check
            .releases
            .iter()
            .filter(|r| r.version == running)
            .collect(),
    };
    // The command for that version (or the newest, for a development build).
    let target = update.clone().or_else(|| {
        (channel == "development")
            .then(|| check.latest.clone())
            .flatten()
    });
    let command = target.as_ref().map(|v| {
        format!(
            "curl -fsSL {} | sudo sh -s -- --version {v}",
            crate::services::panel_domain_service::INSTALLER
        )
    });
    let mut components = json!({ "rustcms": env!("RDP_CORE_VERSION") });
    // The panel's database: SQLite in its process, or MySQL's service.
    let sqlite = app.db().get_database_backend() == sea_orm::DatabaseBackend::Sqlite;
    if sqlite {
        use sea_orm::ConnectionTrait;
        let version = app
            .db()
            .query_one_raw(sea_orm::Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "SELECT sqlite_version()",
            ))
            .await
            .ok()
            .flatten()
            .and_then(|row| row.try_get_by_index::<String>(0).ok());
        if let Some(version) = version {
            components["sqlite"] = json!(version);
        }
    }
    if let Some(docker) = docker.as_deref() {
        if let Ok(info) = docker.info().await {
            components["docker"] = json!(info.server_version);
            components["os"] = json!(info.operating_system);
            components["kernel"] = json!(info.kernel_version);
            components["architecture"] = json!(info.architecture);
        }
        let traefik = app
            .service::<crate::services::traefik_service::Traefik>()
            .map(|t| t.service.clone())
            .unwrap_or_default();
        for (key, service) in [
            ("traefik", traefik.as_str()),
            ("mysql", if sqlite { "" } else { "rdp-mysql" }),
            ("cloudflared", crate::services::tunnel_service::SERVICE),
        ] {
            if service.is_empty() {
                continue;
            }
            let image = docker
                .engine()
                .inspect_service(service)
                .await
                .ok()
                .flatten()
                .and_then(|s| s.spec)
                .and_then(|s| s.task_template)
                .and_then(|t| t.container_spec)
                .and_then(|c| c.image);
            if let Some(image) = image {
                components[key] = json!(image.split('@').next().unwrap_or(&image));
            }
        }
    }
    let commit = env!("RDP_COMMIT");
    Ok(json!({
        "rdockpanel": {
            "version": running,
            "commit": if commit.is_empty() { Value::Null } else { json!(commit) },
            "image": image.as_ref().map(|i| i.split('@').next().unwrap_or(i).to_owned()),
            "digest": image.as_ref().and_then(|i| i.split_once('@').map(|(_, d)| d.to_owned())),
            "channel": channel,
            "installed": installed,
            "source": format!("https://github.com/{REPOSITORY}"),
        },
        "check": {
            "checked_at": check.checked_at,
            "latest": check.latest,
            "error": check.error,
            "error_at": check.error_at,
            "cooldown": COOLDOWN.as_secs(),
        },
        "update": update,
        "notes": notes,
        "command": command,
        "components": components,
        "history": history(app.db()).await?,
        "updating": crate::services::panel_update_service::progress(app.db()).await?,
        // Migrations a newer version applied (this one rolled back to).
        "newer_migrations": rustcms::db::unknown_migrations::<crate::database::migrations::Migrator>(app.db())
            .await
            .unwrap_or_default(),
        // The button's: a release's panel, installed, its service found.
        "can_update": update.is_some() && installed && image.is_some(),
    }))
}

/// The update a release channel's panel may take: the newest release when
/// newer than this binary's version (from the last check).
pub fn update_of(check: &Check, channel: &str) -> Option<String> {
    let latest = check.latest.as_deref()?;
    (channel == "release" && newer(latest, env!("RDP_VERSION"))).then(|| latest.to_owned())
}

/// The notice the Dashboard shows `user` (a panel administrator): the
/// update, unless they dismissed that version.
pub async fn notice(app: &AppState, user: i64) -> Option<Value> {
    let image = match app.service::<crate::services::docker_service::Docker>() {
        Some(docker) if crate::services::deploy_rules_service::config().installed => {
            crate::services::system_service::own_image(&docker)
                .await
                .ok()
                .flatten()
                .map(|(image, _)| image)
        }
        _ => None,
    };
    let check = last(app.db()).await;
    let version = update_of(&check, channel(image.as_deref()))?;
    let dismissed = panel_setting::get(app.db(), &format!("{DISMISSED_PREFIX}{user}"))
        .await
        .ok()
        .flatten();
    (dismissed.as_deref() != Some(version.as_str())).then(|| json!({ "version": version }))
}

/// `user` hides the notice of `version` (shown again for a newer one).
pub async fn dismiss<C: sea_orm::ConnectionTrait>(
    db: &C,
    user: i64,
    version: &str,
) -> AppResult<()> {
    if parse(version).is_none() {
        return Err(rustcms::ValidationErrors::field("version", "is not a version").into());
    }
    panel_setting::set(db, &format!("{DISMISSED_PREFIX}{user}"), version).await?;
    Ok(())
}

/// At boot: a version other than the last start's is the history's
/// (`panel.version`, from → to: an installer run, a restart on a new
/// image).
pub async fn record_start<C: sea_orm::ConnectionTrait>(db: &C) -> AppResult<()> {
    let now = env!("RDP_VERSION");
    let commit = env!("RDP_COMMIT");
    let this = if commit.is_empty() {
        now.to_owned()
    } else {
        format!("{now} ({})", &commit[..commit.len().min(7)])
    };
    let seen = panel_setting::get(db, SEEN_KEY).await?;
    if seen.as_deref() == Some(this.as_str()) {
        return Ok(());
    }
    if let Some(before) = seen {
        crate::services::action_service::record(db, "panel.version", &format!("{before} → {this}"))
            .await?;
    }
    panel_setting::set(db, SEEN_KEY, &this).await?;
    Ok(())
}

/// The history: version changes and the panel's restarts (the newest first).
pub async fn history<C: sea_orm::ConnectionTrait>(db: &C) -> AppResult<Vec<Value>> {
    use crate::models::action;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
    let rows = action::Entity::find()
        .filter(action::Column::Kind.is_in(["panel.version", "panel.restart", "panel.update"]))
        .order_by_desc(action::Column::Id)
        .limit(20)
        .all(db)
        .await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            json!({
                "kind": r.kind,
                "target": r.target,
                "status": r.status,
                "user_id": r.user_id,
                "error": r.error,
                // An update's end ("updated to …", the dump's path).
                "log": (r.kind == "panel.update" && r.status != action::RUNNING).then_some(r.log),
                "at": r.created_at,
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_and_channels() {
        assert_eq!(parse("0.4.2"), Some((0, 4, 2)));
        assert_eq!(parse("v1.10.0"), Some((1, 10, 0)));
        for bad in ["0.4", "0.4.2-rc1", "main", "1.2.3.4", "1..2", "a.b.c", ""] {
            assert_eq!(parse(bad), None, "{bad}");
        }
        assert_eq!(
            newest([
                "main", "0.4.2", "0.10.0", "0.9.9", "latest", "v9.9.9", "0.4"
            ]),
            Some("0.10.0".to_owned())
        );
        assert!(newer("0.5.0", "0.4.2") && !newer("0.4.2", "0.4.2") && !newer("main", "0.4.2"));
        let ours = "ghcr.io/irisvn/rdockpanel";
        assert_eq!(channel(Some(&format!("{ours}:0.4.2@sha256:ab"))), "release");
        assert_eq!(channel(Some(&format!("{ours}:0.4"))), "release");
        assert_eq!(channel(Some(&format!("{ours}:latest"))), "release");
        assert_eq!(
            channel(Some(&format!("{ours}:main-f0b49c8"))),
            "development"
        );
        assert_eq!(
            channel(Some("example.com/fork/rdockpanel:1.0.0")),
            "development"
        );
        assert_eq!(channel(None), "source");
    }

    #[test]
    fn the_next_tags_page() {
        assert_eq!(
            next_page(r#"</v2/irisvn/rdockpanel/tags/list?last=0.4&n=1000>; rel="next""#)
                .as_deref(),
            Some("https://ghcr.io/v2/irisvn/rdockpanel/tags/list?last=0.4&n=1000")
        );
        assert_eq!(
            next_page(r#"<https://ghcr.io/v2/x/tags/list?last=a>; rel="next""#).as_deref(),
            Some("https://ghcr.io/v2/x/tags/list?last=a")
        );
        assert!(next_page("</v2/x?a>; rel=next").is_some());
        assert!(
            next_page(r#"</v2/x?b>; rel="prev", </v2/x?c>; rel="next prev""#)
                .is_some_and(|u| u.ends_with("?c"))
        );
        assert_eq!(next_page(r#"</v2/x?a>; rel="prev""#), None);
        assert_eq!(
            next_page(r#"<https://evil.example/v2/x>; rel="next""#),
            None
        );
    }

    #[test]
    fn notes_between_two_versions() {
        let r = |v: &str| Release {
            version: v.to_owned(),
            ..Default::default()
        };
        let all = [r("0.6.0"), r("0.4.2"), r("0.5.0"), r("0.4.3")];
        let found: Vec<_> = between(&all, "0.4.2", "0.5.0")
            .iter()
            .map(|r| r.version.clone())
            .collect();
        assert_eq!(found, ["0.5.0", "0.4.3"]);
        assert!(between(&all, "main", "0.5.0").is_empty());
    }

    #[test]
    fn the_update_is_a_release_channels_only() {
        let check = Check {
            latest: Some("999.0.0".to_owned()),
            ..Default::default()
        };
        assert_eq!(update_of(&check, "release"), Some("999.0.0".to_owned()));
        assert_eq!(update_of(&check, "development"), None);
        let same = Check {
            latest: Some(env!("RDP_VERSION").to_owned()),
            ..Default::default()
        };
        assert_eq!(update_of(&same, "release"), None);
    }
}
