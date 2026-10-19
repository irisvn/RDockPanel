//! EasyPanel templates: the catalog and each template's files, fetched from
//! their repository when asked (never bundled: the repository has no
//! license), and what a generator's `{services}` output becomes here
//! (`plan`). The same output pasted by hand is "Create from Schema".

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::services::creation_service::{
    DatabaseOptions, NewBuild, NewDomain, NewMount, NewPort, NewService,
};
use crate::services::database_service::Engine;
use crate::services::placeholders::{self, Vars};
use crate::services::template_engine::{MAX_SOURCE, Runner};

/// The largest catalog (`list.json` is about 600 KB) and logo.
const MAX_CATALOG: usize = 4 * 1024 * 1024;
const MAX_LOGO: usize = 512 * 1024;

/// `url`'s body, at most `limit` bytes (read as it streams: a larger one,
/// with or without its length announced, is refused before it is kept).
async fn fetch(url: &str, limit: usize) -> Result<Vec<u8>, String> {
    let mut response = reqwest::Client::new()
        .get(url)
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| format!("the templates could not be fetched: {e}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "{}: {}",
            url.rsplit('/').next().unwrap_or(url),
            response.status()
        ));
    }
    if response.content_length().is_some_and(|n| n > limit as u64) {
        return Err("too large".to_owned());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        if body.len() + chunk.len() > limit {
            return Err("too large".to_owned());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// How long the catalog and a template's files are kept.
const KEEP: Duration = Duration::from_secs(6 * 60 * 60);

/// Where templates come from (bound by the app; tests bind fixed files).
pub struct Templates {
    /// `https://raw.githubusercontent.com/easypanel-io/templates/main/templates`.
    base: String,
    /// Files by path (`list.json`, `<slug>/meta.yaml`, …) when fixed (tests).
    fixed: Option<HashMap<String, String>>,
    cache: Mutex<HashMap<String, (Instant, String)>>,
    logos: Mutex<HashMap<String, Logo>>,
    /// Where generators run (a child process; in-process in tests).
    pub runner: Runner,
}

/// A logo kept: when fetched, its type, its bytes.
type Logo = (Instant, &'static str, Vec<u8>);

impl Templates {
    pub fn new(base: impl Into<String>) -> Self {
        Self {
            base: base.into().trim_end_matches('/').to_owned(),
            fixed: None,
            cache: Mutex::default(),
            logos: Mutex::default(),
            runner: Runner::child(),
        }
    }

    /// Templates from these files only (tests).
    pub fn fixed(files: HashMap<String, String>) -> Self {
        Self {
            base: "https://templates.invalid".to_owned(),
            fixed: Some(files),
            cache: Mutex::default(),
            logos: Mutex::default(),
            runner: Runner::InProcess,
        }
    }

    /// `path` below the repository's `templates/`.
    async fn file(&self, path: &str) -> Result<String, String> {
        if let Some(files) = &self.fixed {
            return files
                .get(path)
                .cloned()
                .ok_or_else(|| format!("{path}: not found"));
        }
        if let Some((at, text)) = self.cache.lock().unwrap().get(path)
            && at.elapsed() < KEEP
        {
            return Ok(text.clone());
        }
        let limit = if path == "list.json" {
            MAX_CATALOG
        } else {
            MAX_SOURCE
        };
        let body = fetch(&format!("{}/{path}", self.base), limit)
            .await
            .map_err(|e| format!("{path}: {e}"))?;
        let text = String::from_utf8(body).map_err(|_| format!("{path}: not text"))?;
        self.cache
            .lock()
            .unwrap()
            .insert(path.to_owned(), (Instant::now(), text.clone()));
        Ok(text)
    }

    /// Forgets what was fetched ("Refresh").
    pub fn refresh(&self) {
        self.cache.lock().unwrap().clear();
    }

    /// A template's logo (its type and bytes), through the panel: the admin
    /// panel's Content Security Policy loads images from itself only.
    pub async fn logo(&self, slug: &str) -> Result<(&'static str, Vec<u8>), String> {
        if !valid_slug(slug) {
            return Err("not a template".to_owned());
        }
        if let Some((at, kind, bytes)) = self.logos.lock().unwrap().get(slug)
            && at.elapsed() < KEEP
        {
            return Ok((kind, bytes.clone()));
        }
        let file = self
            .list()
            .await?
            .into_iter()
            .find(|t| t["slug"] == slug)
            .and_then(|t| t["logo_file"].as_str().map(str::to_owned))
            .ok_or("no logo")?;
        let kind = match file.rsplit('.').next() {
            Some("svg") => "image/svg+xml",
            Some("jpg" | "jpeg") => "image/jpeg",
            Some("webp") => "image/webp",
            _ => "image/png",
        };
        if self.fixed.is_some() {
            return Err("no logo".to_owned());
        }
        // The file name comes from the catalog: a plain name only.
        if !file
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
            || file.starts_with('.')
        {
            return Err("no logo".to_owned());
        }
        let bytes = fetch(&format!("{}/{slug}/assets/{file}", self.base), MAX_LOGO).await?;
        self.logos
            .lock()
            .unwrap()
            .insert(slug.to_owned(), (Instant::now(), kind, bytes.clone()));
        Ok((kind, bytes))
    }

    /// The catalog: slug, name, description, tags, logo.
    pub async fn list(&self) -> Result<Vec<Value>, String> {
        let text = self.file("list.json").await?;
        let list: Vec<Value> =
            serde_json::from_str(&text).map_err(|e| format!("the catalog: {e}"))?;
        Ok(list
            .into_iter()
            .filter_map(|t| {
                let slug = t.get("slug")?.as_str()?.to_owned();
                valid_slug(&slug).then(|| {
                    json!({
                        "slug": slug,
                        "name": t.get("name").cloned().unwrap_or(json!(slug)),
                        "description": t.get("description").cloned().unwrap_or(Value::Null),
                        "tags": t.get("tags").cloned().unwrap_or(json!([])),
                        "logo_file": t.get("logo").and_then(Value::as_str),
                    })
                })
            })
            .collect())
    }

    /// A template's `meta.yaml` (as JSON) and generator (`index.ts`).
    pub async fn template(&self, slug: &str) -> Result<(Value, String), String> {
        if !valid_slug(slug) {
            return Err("not a template".to_owned());
        }
        let meta = self.file(&format!("{slug}/meta.yaml")).await?;
        let meta: Value =
            serde_yaml_ng::from_str(&meta).map_err(|e| format!("{slug}/meta.yaml: {e}"))?;
        let code = self.file(&format!("{slug}/index.ts")).await?;
        Ok((meta, code))
    }

    /// A template's page: what its form needs.
    pub async fn page(&self, slug: &str) -> Result<Value, String> {
        let (meta, _) = self.template(slug).await?;
        let logo = self
            .list()
            .await
            .ok()
            .and_then(|l| l.into_iter().find(|t| t["slug"] == slug))
            .is_some_and(|t| t["logo_file"].is_string());
        Ok(json!({
            "slug": slug,
            "name": meta.get("name"),
            "description": meta.get("description"),
            "instructions": meta.get("instructions"),
            "schema": meta.get("schema"),
            "links": meta.get("links"),
            "logo": logo,
        }))
    }
}

/// A template's slug as the repository names its folders.
fn valid_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug.len() <= 64
        && slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// What a generator output becomes: services to create (databases first)
/// and what was left out (each said, never silently).
#[derive(Debug, Default)]
pub struct Plan {
    pub services: Vec<NewService>,
    pub warnings: Vec<String>,
    /// What the services need and do not get (authentication, Linux
    /// capabilities): they are created, never deployed at once.
    pub blocking: Vec<String>,
}

/// An EasyPanel environment: `.env` text, as EasyPanel reads it (quoted
/// values unquoted, several lines when quoted); refused when malformed.
fn env_lines(name: &str, text: &str) -> Result<BTreeMap<String, String>, String> {
    crate::services::dotenv::parse(text)
        .map_err(|e| format!("{name}: its environment, line {}: {}", e.line, e.message))
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key)?.as_str().map(str::to_owned)
}

fn number(v: &Value, key: &str) -> Option<i64> {
    v.get(key)?.as_i64()
}

/// An EasyPanel source and build as a panel build.
fn template_build(
    name: &str,
    kind: &str,
    source: &Value,
    build: Option<&Value>,
) -> Result<NewBuild, String> {
    let builder = match build.and_then(|b| text(b, "type")).as_deref() {
        None | Some("dockerfile") => "dockerfile",
        Some("nixpacks") => "nixpacks",
        Some(other) => {
            return Err(format!(
                "{name}: builds with {other}, not supported (Dockerfile and Nixpacks are)"
            ));
        }
    };
    let dockerfile_path = build.and_then(|b| text(b, "file"));
    let path = text(source, "path").filter(|p| !p.is_empty() && p != "/");
    Ok(match kind {
        "github"
            if [text(source, "owner"), text(source, "repo")]
                .iter()
                .any(|p| p.as_deref().unwrap_or_default().is_empty()) =>
        {
            return Err(format!(
                "{name}: its GitHub source has no owner or repository"
            ));
        }
        "github" => NewBuild {
            source: "git".to_owned(),
            git_url: Some(format!(
                "https://github.com/{}/{}.git",
                text(source, "owner").unwrap_or_default(),
                text(source, "repo").unwrap_or_default()
            )),
            git_branch: text(source, "ref"),
            git_path: path,
            builder: builder.to_owned(),
            dockerfile_path,
            dockerfile: None,
        },
        "git" => NewBuild {
            source: "git".to_owned(),
            git_url: text(source, "repo"),
            git_branch: text(source, "ref"),
            git_path: path,
            builder: builder.to_owned(),
            dockerfile_path,
            dockerfile: None,
        },
        "dockerfile" => NewBuild {
            source: "dockerfile".to_owned(),
            git_url: None,
            git_branch: None,
            git_path: None,
            builder: "dockerfile".to_owned(),
            dockerfile_path: None,
            dockerfile: text(source, "dockerfile"),
        },
        other => return Err(format!("{name}: its source {other} is not supported")),
    })
}

/// The services of a generator's `output` for `project` (`host`: the
/// generated domains' parent, `[panel] service_domain`).
pub fn plan(output: &Value, project: &str, host: Option<&str>) -> Result<Plan, String> {
    let list = output
        .get("services")
        .and_then(Value::as_array)
        .ok_or("the template made no services")?;
    let mut plan = Plan::default();
    let mut apps = vec![];
    for item in list {
        let kind = text(item, "type").unwrap_or_default();
        let data = item.get("data").cloned().unwrap_or(json!({}));
        let name = text(&data, "serviceName").ok_or("a service has no name")?;
        let warn = |plan: &mut Plan, what: &str| plan.warnings.push(format!("{name}: {what}"));
        if let Some(engine) = Engine::parse(&kind) {
            let mut new = NewService::database(name.clone(), engine);
            new.image = text(&data, "image");
            new.database = DatabaseOptions {
                database: text(&data, "databaseName"),
                user: text(&data, "user"),
                password: text(&data, "password"),
                root_password: text(&data, "rootPassword"),
            };
            if let Some(env) = text(&data, "env") {
                new.env = env_lines(&name, &env)?;
            }
            new.command = text(&data, "command");
            plan.services.push(new);
            continue;
        }
        if kind != "app" {
            return Err(format!(
                "{name}: {kind} services are not supported yet (apps and databases are)"
            ));
        }
        let source = data.get("source").cloned().unwrap_or(json!({}));
        let source_type = text(&source, "type").unwrap_or_default();
        let mut new = NewService::app(name.clone(), text(&source, "image").unwrap_or_default());
        if source_type != "image" {
            new.build = Some(template_build(
                &name,
                &source_type,
                &source,
                data.get("build"),
            )?);
        }
        if text(&source, "username").is_some() {
            warn(
                &mut plan,
                "registry credentials left out: set them on its Source tab",
            );
        }
        if let Some(env) = text(&data, "env") {
            new.env = env_lines(&name, &env)?;
        }
        let deploy = data.get("deploy").cloned().unwrap_or(json!({}));
        new.replicas = number(&deploy, "replicas").unwrap_or(1).clamp(1, 50) as i32;
        new.command = text(&deploy, "command");
        new.zero_downtime = deploy
            .get("zeroDowntime")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let vars = Vars {
            project: project.to_owned(),
            service: name.clone(),
            primary_domain: None,
            host: host.map(str::to_owned),
            host_ip: None,
        };
        for domain in data
            .get("domains")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let raw = text(domain, "host").unwrap_or_default();
            let host = placeholders::expand(&raw, &vars);
            if host.contains("$(") {
                warn(
                    &mut plan,
                    &format!(
                        "domain {raw} left out (set [panel] service_domain for generated domains)"
                    ),
                );
                continue;
            }
            new.domains.push(NewDomain {
                host,
                path: text(domain, "path").unwrap_or_else(|| "/".to_owned()),
                port: number(domain, "port").unwrap_or(80).clamp(1, 65535) as u16,
                https: domain.get("https").and_then(Value::as_bool).unwrap_or(true),
            });
        }
        for mount in data
            .get("mounts")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let target = text(mount, "mountPath").unwrap_or_default();
            match text(mount, "type").as_deref() {
                Some("volume") => new.mounts.push(NewMount::Volume {
                    name: text(mount, "name").unwrap_or_default(),
                    target,
                }),
                Some("file") => new.mounts.push(NewMount::File {
                    content: text(mount, "content").unwrap_or_default(),
                    target,
                }),
                _ => warn(
                    &mut plan,
                    &format!(
                        "host directory {} left out (needs system.manage: add it on its Storage tab)",
                        text(mount, "hostPath").unwrap_or_default()
                    ),
                ),
            }
        }
        for port in data
            .get("ports")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            new.ports.push(NewPort {
                published: number(port, "published").unwrap_or(0).clamp(0, 65535) as u16,
                target: number(port, "target").unwrap_or(0).clamp(0, 65535) as u16,
                protocol: text(port, "protocol").unwrap_or_else(|| "tcp".to_owned()),
            });
        }
        for unsupported in ["redirects", "resources", "maintenance", "scripts"] {
            if data.get(unsupported).is_some_and(|v| !v.is_null()) {
                warn(&mut plan, &format!("{unsupported} left out"));
            }
        }
        // What it needs to be safe or to run: never deployed without.
        let mut needs = vec![];
        if data
            .get("basicAuth")
            .and_then(Value::as_array)
            .is_some_and(|a| !a.is_empty())
        {
            needs.push("basic authentication");
        }
        for field in ["capAdd", "capDrop", "sysctls", "groups"] {
            if deploy.get(field).is_some_and(|v| {
                v.as_array().is_some_and(|a| !a.is_empty())
                    || v.as_object().is_some_and(|o| !o.is_empty())
            }) {
                needs.push(field);
            }
        }
        if !needs.is_empty() {
            let what = format!(
                "{name}: needs {} (not supported yet): created, not deployed",
                needs.join(", ")
            );
            plan.warnings.push(what.clone());
            plan.blocking.push(what);
        }
        apps.push(new);
    }
    // Databases first: apps find them deployed.
    plan.services.extend(apps);
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_generator_output_becomes_services() {
        let output = json!({"services": [
            {"type": "app", "data": {
                "serviceName": "ghost",
                "source": {"type": "image", "image": "ghost:6"},
                "domains": [{"host": "$(EASYPANEL_DOMAIN)", "port": 2368}],
                "mounts": [
                    {"type": "volume", "name": "content", "mountPath": "/var/lib/ghost/content"},
                    {"type": "file", "content": "a=1\n", "mountPath": "/etc/x.ini"},
                    {"type": "bind", "hostPath": "/var/run/docker.sock", "mountPath": "/var/run/docker.sock"}
                ],
                "env": "url=https://$(PRIMARY_DOMAIN)\ndb=$(PROJECT_NAME)_ghost-db\nquoted=\"x\"",
                "deploy": {"command": "node index.js", "zeroDowntime": false}
            }},
            {"type": "mysql", "data": {"serviceName": "ghost-db", "password": "pw"}}
        ]});
        let plan = plan(&output, "blog", Some("apps.example.com")).unwrap();
        assert_eq!(plan.services[0].kind, "mysql", "databases first");
        assert_eq!(plan.services[0].database.password.as_deref(), Some("pw"));
        let app = &plan.services[1];
        assert_eq!(app.domains[0].host, "blog-ghost.apps.example.com");
        assert_eq!(app.domains[0].port, 2368);
        assert!(app.domains[0].https);
        assert_eq!(
            app.env["url"], "https://$(PRIMARY_DOMAIN)",
            "expanded at deploy"
        );
        assert_eq!(app.env["quoted"], "x", "unquoted, as EasyPanel reads .env");
        assert_eq!(app.mounts.len(), 2);
        assert_eq!(app.command.as_deref(), Some("node index.js"));
        assert!(!app.zero_downtime);
        assert!(
            plan.warnings.iter().any(|w| w.contains("docker.sock")),
            "{:?}",
            plan.warnings
        );
        assert!(plan.blocking.is_empty());
        let guarded = json!({"services": [{"type": "app", "data": {"serviceName": "registry", "source": {"type": "image", "image": "registry:2"}, "basicAuth": [{"username": "u", "password": "p"}], "deploy": {"capAdd": ["NET_ADMIN"]}}}]});
        let p = super::plan(&guarded, "p", None).unwrap();
        assert!(
            p.blocking[0].contains("basic authentication") && p.blocking[0].contains("capAdd"),
            "{:?}",
            p.blocking
        );
    }

    #[test]
    fn what_cannot_be_done_is_said() {
        let git = json!({"services": [{"type": "app", "data": {"serviceName": "x", "source": {"type": "github", "owner": "org", "repo": "app", "ref": "main", "path": "/web"}, "build": {"type": "nixpacks"}}}]});
        let built = plan(&git, "p", None).unwrap();
        let build = built.services[0].build.as_ref().unwrap();
        assert_eq!(
            build.git_url.as_deref(),
            Some("https://github.com/org/app.git")
        );
        assert_eq!(
            (build.builder.as_str(), build.git_path.as_deref()),
            ("nixpacks", Some("/web"))
        );
        let packs = json!({"services": [{"type": "app", "data": {"serviceName": "x", "source": {"type": "git", "repo": "https://x/y.git"}, "build": {"type": "heroku-buildpacks"}}}]});
        assert!(
            plan(&packs, "p", None)
                .unwrap_err()
                .contains("heroku-buildpacks")
        );
        let compose = json!({"services": [{"type": "compose", "data": {"serviceName": "x"}}]});
        assert!(
            plan(&compose, "p", None)
                .unwrap_err()
                .contains("not supported")
        );
        // No generated domains configured: the domain is left out, said.
        let app = json!({"services": [{"type": "app", "data": {"serviceName": "x", "source": {"type": "image", "image": "nginx"}, "domains": [{"host": "$(EASYPANEL_DOMAIN)"}]}}]});
        let p = plan(&app, "p", None).unwrap();
        assert!(p.services[0].domains.is_empty() && !p.warnings.is_empty());
    }
}
