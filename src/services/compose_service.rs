//! Compose services (as EasyPanel's): the containers of a Compose file, run
//! by `docker compose` on this node (not as Swarm services). The file is
//! written in the panel, or one of the service's Git repository. The panel
//! writes it again as `.rdockpanel-compose.yml` with each container also on
//! the project's network under the alias `<project>_<service>_<container>`
//! (what its domains lead to) and labelled as the panel's; the service's
//! environment is the Compose project's `.env` (`${VAR}` in the file).
//! Deploy runs `up -d --build --remove-orphans`, Start `up -d`, Stop `stop`,
//! Restart `restart`, Destroy `down` (named volumes are kept).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_yaml_ng::{Mapping, Value};

use crate::services::build_service::{inside, sync_repo};
use crate::services::deploy_rules_service::config;
use crate::services::deploy_service::{
    Compose, Snapshot, compose_alias, network_name, valid_compose_service,
};
use crate::services::deployment_service::{Outcome, RunLog, Secrets, Trigger};
use crate::services::docker_service::{Docker, MANAGED_LABEL};

/// The file the panel writes (next to the Compose project's own files).
pub const FILE: &str = ".rdockpanel-compose.yml";
/// The longest Compose file taken.
pub const MAX_FILE: usize = 256 * 1024;

fn failed(message: impl Into<String>) -> Outcome {
    Outcome::Failed(message.into())
}

/// A Compose file as the panel runs it: its containers (in order), the file
/// with the project's network and labels added, and what may not work.
#[derive(Debug)]
pub struct Prepared {
    pub services: Vec<String>,
    pub yaml: String,
    pub warnings: Vec<String>,
}

/// Reads `text` (a Compose file) and adds, to each container, the project's
/// network (alias `<base>_<container>`) and the panel's labels.
pub fn prepare(text: &str, project: &str, service: &str) -> Result<Prepared, String> {
    if text.len() > MAX_FILE {
        return Err("the Compose file is over 256 KB".into());
    }
    let mut root: Value =
        serde_yaml_ng::from_str(text).map_err(|e| format!("the Compose file: {e}"))?;
    // `<<` merges resolved first: what a service inherits is what it has.
    root.apply_merge()
        .map_err(|e| format!("the Compose file: {e}"))?;
    let base = crate::services::deploy_service::service_name(project, service);
    let network = network_name(project);
    let top = root
        .as_mapping_mut()
        .ok_or("the Compose file is not a mapping")?;
    let services = top
        .get_mut("services")
        .and_then(Value::as_mapping_mut)
        .ok_or("the Compose file has no services")?;
    let mut names = vec![];
    let mut warnings = vec![];
    for (name, definition) in services.iter_mut() {
        let name = name
            .as_str()
            .filter(|n| valid_compose_service(n))
            .ok_or("a service of the Compose file has a name of other characters than letters, digits, _, - and .")?
            .to_owned();
        let definition = definition
            .as_mapping_mut()
            .ok_or_else(|| format!("the service {name} is not a mapping"))?;
        if definition.contains_key("container_name") {
            warnings.push(format!(
                "{name}: container_name may collide with another project's container"
            ));
        }
        if definition.contains_key("ports") {
            warnings.push(format!(
                "{name}: ports publishes on the server (each port once); domains are better for HTTP"
            ));
        }
        if definition.contains_key("network_mode") {
            warnings.push(format!(
                "{name}: network_mode leaves it off the project's network (no domain reaches it)"
            ));
        } else {
            let networks = match definition.remove("networks") {
                None | Some(Value::Null) => {
                    let mut m = Mapping::new();
                    m.insert("default".into(), Value::Null);
                    m
                }
                Some(Value::Sequence(list)) => list
                    .into_iter()
                    .map(|n| (n, Value::Null))
                    .collect::<Mapping>(),
                Some(Value::Mapping(m)) => m,
                Some(_) => return Err(format!("{name}: networks is neither a list nor a mapping")),
            };
            let mut networks = networks;
            let mut alias = Mapping::new();
            alias.insert(
                "aliases".into(),
                Value::Sequence(vec![compose_alias(&base, &name).into()]),
            );
            networks.insert("rdockpanel".into(), Value::Mapping(alias));
            definition.insert("networks".into(), Value::Mapping(networks));
        }
        let labels = [
            (MANAGED_LABEL.to_owned(), "true".to_owned()),
            ("rdockpanel.project".to_owned(), project.to_owned()),
            ("rdockpanel.service".to_owned(), service.to_owned()),
        ];
        match definition.get_mut("labels") {
            Some(Value::Sequence(list)) => {
                list.retain(|l| !l.as_str().is_some_and(|l| l.starts_with("rdockpanel.")));
                list.extend(labels.iter().map(|(k, v)| Value::from(format!("{k}={v}"))));
            }
            Some(Value::Mapping(map)) => {
                map.retain(|k, _| !k.as_str().is_some_and(|k| k.starts_with("rdockpanel.")));
                for (k, v) in &labels {
                    map.insert(k.as_str().into(), v.as_str().into());
                }
            }
            _ => {
                let map: Mapping = labels
                    .iter()
                    .map(|(k, v)| (Value::from(k.as_str()), Value::from(v.as_str())))
                    .collect();
                definition.insert("labels".into(), Value::Mapping(map));
            }
        }
        names.push(name);
    }
    if names.is_empty() {
        return Err("the Compose file has no services".into());
    }
    let networks = top
        .entry("networks".into())
        .or_insert_with(|| Value::Mapping(Mapping::new()));
    if networks.is_null() {
        *networks = Value::Mapping(Mapping::new());
    }
    let networks = networks
        .as_mapping_mut()
        .ok_or("networks of the Compose file is not a mapping")?;
    let mut external = Mapping::new();
    external.insert("external".into(), true.into());
    external.insert("name".into(), network.into());
    networks.insert("rdockpanel".into(), Value::Mapping(external));
    let yaml = serde_yaml_ng::to_string(&root).map_err(|e| e.to_string())?;
    Ok(Prepared {
        services: names,
        yaml,
        warnings,
    })
}

/// Where the service was last deployed (in its directory, recorded at each
/// deployment), if anywhere.
fn deployed_dir(dir: &Path) -> Option<PathBuf> {
    let relative = std::fs::read_to_string(dir.join(DEPLOYED)).ok()?;
    let path = inside(relative.trim())?;
    Some(dir.join(path))
}

/// The record of [`deployed_dir`].
const DEPLOYED: &str = ".rdockpanel-deployed";

/// A Compose service's directory (`compose_dir/<project>_<service>`).
pub fn directory(snapshot: &Snapshot) -> PathBuf {
    let root = &config().compose_dir;
    let root = if root.is_absolute() {
        root.clone()
    } else {
        std::env::current_dir().unwrap_or_default().join(root)
    };
    root.join(snapshot.service_name())
}

/// `.env` of the Compose project (`0600`), quoted as Compose reads it.
fn write_env(dir: &Path, env: &BTreeMap<String, String>) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut text = String::new();
    for (name, value) in env {
        let escaped = value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('$', "$$");
        text.push_str(&format!("{name}=\"{escaped}\"\n"));
    }
    let path = dir.join(".env");
    let _ = std::fs::remove_file(&path);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)?;
    file.write_all(text.as_bytes())
}

/// Runs `docker compose` there, its output in the log: whether it worked.
async fn compose(
    docker: &Docker,
    dir: &Path,
    project: &str,
    args: &[&str],
    log: &mut RunLog<'_>,
) -> Result<(), Outcome> {
    log.line(format!("docker compose {}", args.join(" "))).await;
    log.hold().await?;
    // The deployment's lock renewed meanwhile; lost, the command is
    // stopped (dropped: its process killed).
    let engine = docker.engine();
    let work = engine.compose(dir, project, FILE, args);
    tokio::pin!(work);
    let mut renew = tokio::time::interval(std::time::Duration::from_secs(30));
    renew.tick().await;
    let done = loop {
        tokio::select! {
            done = &mut work => break done,
            _ = renew.tick() => log.hold().await?,
        }
    };
    let (code, output) = done.map_err(|e| failed(e.to_string()))?;
    for line in output.lines().filter(|l| !l.trim().is_empty()).take(400) {
        log.line(line).await;
    }
    log.hold().await?;
    if code == 0 {
        Ok(())
    } else {
        Err(failed(format!(
            "docker compose {} exited with {code}",
            args[0]
        )))
    }
}

/// Applies a deployment of a Compose service.
pub(crate) async fn apply(
    docker: &Docker,
    traefik: Option<&crate::services::traefik_service::Traefik>,
    snapshot: &Snapshot,
    source: &Compose,
    (trigger, rebuild): (Trigger, bool),
    secrets: &Secrets,
    log: &mut RunLog<'_>,
) -> Result<(Outcome, Option<String>), Outcome> {
    let name = snapshot.service_name();
    let dir = directory(snapshot);
    // The Compose project's directory: the service's, or a directory of its
    // checkout.
    let project_dir = match &source.git_url {
        Some(_) => {
            let path = inside(source.git_path.as_deref().unwrap_or("/"))
                .ok_or_else(|| failed("the path leaves the repository"))?;
            dir.join("src").join(path)
        }
        None => dir.clone(),
    };
    // Stop, Start, Restart and Destroy act where it was last deployed (a
    // Source changed since leaves the running containers where they are).
    let project_dir = match deployed_dir(&dir) {
        Some(deployed) if trigger != Trigger::Deploy => deployed,
        _ => project_dir,
    };
    let deployed = project_dir.join(FILE).is_file();
    match trigger {
        Trigger::Destroy | Trigger::Stop if !deployed => {
            log.line(format!("{name} was never deployed: nothing to do"))
                .await;
            return Ok((Outcome::Succeeded, None));
        }
        Trigger::Destroy => {
            compose(
                docker,
                &project_dir,
                &name,
                &["down", "--remove-orphans"],
                log,
            )
            .await?;
            return Ok((Outcome::Succeeded, None));
        }
        Trigger::Stop => {
            compose(docker, &project_dir, &name, &["stop"], log).await?;
            return Ok((Outcome::Succeeded, None));
        }
        Trigger::Rollback => {
            return Err(failed(
                "a Compose service is not rolled back: deploy its file as it was",
            ));
        }
        Trigger::Restart if deployed => {
            compose(docker, &project_dir, &name, &["restart"], log).await?;
            return Ok((Outcome::Succeeded, None));
        }
        Trigger::Start if deployed => {
            crate::services::deployment_service::project_network(docker, traefik, snapshot, log)
                .await?;
            compose(
                docker,
                &project_dir,
                &name,
                &["up", "-d", "--remove-orphans"],
                log,
            )
            .await?;
            return Ok((Outcome::Succeeded, None));
        }
        _ => {}
    }
    // Deploy (or a first Start/Restart): the file, the network, then up.
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&dir)
            .map_err(|e| failed(format!("{}: {e}", dir.display())))?;
    }
    let text = match (&source.git_url, &source.inline) {
        (Some(url), _) => {
            let commit = sync_repo(
                url,
                source.git_branch.as_deref(),
                &dir.join("src"),
                secrets.git(),
                log,
            )
            .await?;
            log.line(format!("commit {commit}")).await;
            if !project_dir.is_dir() {
                return Err(failed("the path is not a directory of the repository"));
            }
            let file = source.file.as_deref().unwrap_or("docker-compose.yml");
            let path = inside(file)
                .map(|p| project_dir.join(p))
                .ok_or_else(|| failed("the Compose file's path leaves the repository"))?;
            // Never a link out of the checkout.
            let resolved = path
                .canonicalize()
                .ok()
                .filter(|p| project_dir.canonicalize().is_ok_and(|d| p.starts_with(d)))
                .ok_or_else(|| failed(format!("no {file} in the repository")))?;
            std::fs::read_to_string(resolved).map_err(|e| failed(format!("{file}: {e}")))?
        }
        (None, Some(text)) => text.clone(),
        (None, None) => return Err(failed("the service has no Compose file")),
    };
    let prepared = prepare(&text, &snapshot.project, &snapshot.service).map_err(failed)?;
    for warning in &prepared.warnings {
        log.line(format!("warning: {warning}")).await;
    }
    for domain in &snapshot.domains {
        let sub = domain.compose_service.as_deref().unwrap_or_default();
        if !prepared.services.iter().any(|s| s == sub) {
            return Err(failed(format!(
                "the domain {} leads to {sub:?}, not a service of the Compose file ({})",
                domain.host,
                prepared.services.join(", ")
            )));
        }
    }
    // The containers deployed from elsewhere before: taken down first.
    if let Some(before) = deployed_dir(&dir)
        && before != project_dir
        && before.join(FILE).is_file()
    {
        compose(docker, &before, &name, &["down", "--remove-orphans"], log).await?;
    }
    std::fs::write(project_dir.join(FILE), &prepared.yaml)
        .map_err(|e| failed(format!("{FILE}: {e}")))?;
    let relative = project_dir
        .strip_prefix(&dir)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    std::fs::write(
        dir.join(DEPLOYED),
        if relative.is_empty() { "." } else { &relative },
    )
    .map_err(|e| failed(format!("{DEPLOYED}: {e}")))?;
    // Placeholders resolved as for any service (`$(PROJECT_NAME)`, …).
    let vars = crate::services::deployment_service::placeholder_vars(snapshot);
    let env: BTreeMap<String, String> = secrets
        .env
        .iter()
        .map(|(k, v)| (k.clone(), crate::services::placeholders::expand(v, &vars)))
        .collect();
    write_env(&project_dir, &env).map_err(|e| failed(format!(".env: {e}")))?;
    crate::services::deployment_service::project_network(docker, traefik, snapshot, log).await?;
    // Force rebuild: its images built again without the builder's cache
    // (and pulled again), every container made again.
    if rebuild {
        compose(
            docker,
            &project_dir,
            &name,
            &["build", "--no-cache", "--pull"],
            log,
        )
        .await?;
    }
    // A stopped service is deployed stopped (its containers made, not started).
    let up: &[&str] = match (snapshot.running, rebuild) {
        (true, false) => &["up", "-d", "--build", "--remove-orphans"],
        (true, true) => &[
            "up",
            "-d",
            "--build",
            "--pull",
            "always",
            "--force-recreate",
            "--remove-orphans",
        ],
        (false, false) => &["create", "--build", "--remove-orphans"],
        (false, true) => &[
            "create",
            "--build",
            "--pull",
            "always",
            "--force-recreate",
            "--remove-orphans",
        ],
    };
    compose(docker, &project_dir, &name, up, log).await?;
    Ok((Outcome::Succeeded, None))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_compose_file_joins_the_project() {
        let file = r#"
services:
  web:
    image: nginx
    ports: ["8080:80"]
    networks: [back]
    labels: ["app=web", "rdockpanel.project=forged"]
  db:
    image: postgres
    container_name: db
networks:
  back: {}
"#;
        let prepared = prepare(file, "shop", "site").unwrap();
        assert_eq!(prepared.services, ["web", "db"]);
        assert_eq!(prepared.warnings.len(), 2, "{:?}", prepared.warnings);
        let out: Value = serde_yaml_ng::from_str(&prepared.yaml).unwrap();
        let web = &out["services"]["web"];
        assert!(web["networks"]["back"].is_null());
        assert_eq!(web["networks"]["rdockpanel"]["aliases"][0], "shop_site_web");
        let labels: Vec<&str> = web["labels"]
            .as_sequence()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert!(
            labels.contains(&"rdockpanel.project=shop")
                && !labels.contains(&"rdockpanel.project=forged")
        );
        let db = &out["services"]["db"];
        assert!(db["networks"]["default"].is_null());
        assert_eq!(db["labels"]["rdockpanel.service"], "site");
        assert_eq!(out["networks"]["rdockpanel"]["name"], "rdp_shop");
        assert_eq!(out["networks"]["rdockpanel"]["external"], true);
        // What a service inherits (`<<`) is what it has.
        let merged = prepare(
            "x-net: &net\n  network_mode: host\nservices:\n  agent:\n    <<: *net\n    image: busybox\n",
            "shop",
            "site",
        )
        .unwrap();
        let out: Value = serde_yaml_ng::from_str(&merged.yaml).unwrap();
        assert!(
            out["services"]["agent"]["networks"].is_null(),
            "{}",
            merged.yaml
        );
        assert!(merged.warnings[0].contains("network_mode"));
        for bad in [
            "",
            "services: []",
            "services: {}",
            "x: 1",
            "services:\n  'a b': {image: x}",
        ] {
            assert!(prepare(bad, "shop", "site").is_err(), "{bad}");
        }
    }
}
