//! The Compose file of a project being imported: where it is, whether the
//! panel may read it, and what it says that Docker does not (services with
//! no container, `build:`, secrets, `${VAR}` in commands). The text is
//! parsed at once and never stored: only the facts are kept.

use std::io::Read;
use std::path::Path;

use serde::Serialize;
use serde_yaml_ng::Value;

use crate::services::deploy_rules_service::forbidden_path;

/// A file is at most this big (bytes), and the files of a project together.
pub const MAX_FILE: usize = 256 * 1024;
pub const MAX_FILES: usize = 20;
pub const MAX_TOTAL: usize = 1024 * 1024;

/// The files Compose was started with (`com.docker.compose.project.config_files`).
pub fn config_files(label: &str) -> Vec<String> {
    label
        .split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Whether a path may be read: a `.yml`/`.yaml` file under the project's
/// working directory, with no `..`, outside the forbidden places and the
/// installer's directory. By its text: the helper reads what Docker mounts.
pub fn path_ok(path: &str, working_dir: &str, state_dir: &Path) -> bool {
    let p = Path::new(path);
    let dir = Path::new(working_dir);
    p.is_absolute()
        && dir.is_absolute()
        && !path.contains('\0')
        && !p.components().any(|c| c.as_os_str() == "..")
        && p.starts_with(dir)
        && dir != Path::new("/")
        && matches!(p.extension().and_then(|e| e.to_str()), Some("yml" | "yaml"))
        && !forbidden_path(p, state_dir)
}

/// What a Compose file says.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Facts {
    /// Every service the file defines.
    pub services: Vec<String>,
    /// Services with a `profiles:` list (they have no container unless asked).
    pub profiled: Vec<String>,
    /// Services built from a Dockerfile (`build:`).
    pub built: Vec<String>,
    /// Services using `secrets:` or `configs:`.
    pub with_secrets: Vec<String>,
    /// Services whose command uses `${VAR}`: Compose filled it in at `up`.
    pub command_vars: Vec<String>,
}

/// The facts of a Compose file's text.
pub fn parse(text: &str) -> Result<Facts, String> {
    let doc: Value =
        serde_yaml_ng::from_str(text).map_err(|_| "it is not valid YAML".to_owned())?;
    let services = doc
        .get("services")
        .and_then(Value::as_mapping)
        .ok_or_else(|| "it has no `services`".to_owned())?;
    let mut facts = Facts::default();
    for (name, service) in services {
        let Some(name) = name.as_str() else { continue };
        facts.services.push(name.to_owned());
        if service.get("profiles").is_some() {
            facts.profiled.push(name.to_owned());
        }
        if service.get("build").is_some() {
            facts.built.push(name.to_owned());
        }
        if service.get("secrets").is_some() || service.get("configs").is_some() {
            facts.with_secrets.push(name.to_owned());
        }
        let command = ["command", "entrypoint"].iter().any(|key| {
            service.get(*key).is_some_and(|v| match v {
                Value::String(text) => text.contains("${"),
                Value::Sequence(items) => items
                    .iter()
                    .any(|i| i.as_str().is_some_and(|t| t.contains("${"))),
                _ => false,
            })
        });
        if command {
            facts.command_vars.push(name.to_owned());
        }
    }
    Ok(facts)
}

/// The text of the one regular file in the tar archive Docker returns for
/// a file (`GET /containers/{id}/archive`), at most [`MAX_FILE`] bytes.
pub fn file_of_archive(archive: &[u8]) -> Result<String, String> {
    let mut tar = tar::Archive::new(archive);
    let mut entries = tar
        .entries()
        .map_err(|_| "the archive is unreadable".to_owned())?;
    let mut entry = entries
        .next()
        .ok_or_else(|| "the archive is empty".to_owned())?
        .map_err(|_| "the archive is unreadable".to_owned())?;
    if !entry.header().entry_type().is_file() {
        return Err("it is not a regular file".to_owned());
    }
    if entry.header().size().unwrap_or(u64::MAX) > MAX_FILE as u64 {
        return Err("it is too big".to_owned());
    }
    let mut text = String::new();
    entry
        .by_ref()
        .take(MAX_FILE as u64)
        .read_to_string(&mut text)
        .map_err(|_| "it is not text".to_owned())?;
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn archive(name: &str, content: &str, kind: tar::EntryType) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_entry_type(kind);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, name, content.as_bytes())
            .unwrap();
        builder.into_inner().unwrap()
    }

    #[test]
    fn only_a_compose_file_in_the_working_directory_is_read() {
        let state = Path::new("/etc/rdockpanel");
        let ok = |p: &str| path_ok(p, "/srv/shop", state);
        assert!(ok("/srv/shop/docker-compose.yml"));
        assert!(ok("/srv/shop/deploy/prod.yaml"));
        for bad in [
            "/srv/shop/../etc/passwd.yml",
            "/srv/other/docker-compose.yml",
            "/srv/shop/docker-compose.json",
            "/srv/shop/.env",
            "docker-compose.yml",
            "/srv/shopx/docker-compose.yml",
        ] {
            assert!(!ok(bad), "{bad}");
        }
        // A forbidden place, or the installer's directory, even as a working directory.
        assert!(!path_ok("/boot/x.yml", "/boot", state));
        assert!(!path_ok("/etc/rdockpanel/x.yml", "/etc/rdockpanel", state));
        assert!(!path_ok("/x.yml", "/", state));
    }

    #[test]
    fn config_files_are_split_from_the_label() {
        assert_eq!(
            config_files("/a/docker-compose.yml, /a/override.yml,"),
            ["/a/docker-compose.yml", "/a/override.yml"]
        );
        assert!(config_files("").is_empty());
    }

    #[test]
    fn the_facts_of_a_compose_file() {
        let facts = parse(
            r#"
services:
  web:
    image: nginx
    command: ["serve", "--key", "${KEY}"]
    build: .
  db:
    image: postgres
    secrets: [pw]
  tika:
    image: tika
    profiles: [extra]
    command: run
"#,
        )
        .unwrap();
        assert_eq!(facts.services, ["web", "db", "tika"]);
        assert_eq!(facts.profiled, ["tika"]);
        assert_eq!(facts.built, ["web"]);
        assert_eq!(facts.with_secrets, ["db"]);
        assert_eq!(facts.command_vars, ["web"]);
        assert!(parse("not: [a compose").is_err());
        assert!(parse("version: '3'").is_err());
    }

    #[test]
    fn a_file_is_read_from_its_archive_or_refused() {
        let ok = archive(
            "docker-compose.yml",
            "services: {}",
            tar::EntryType::Regular,
        );
        assert_eq!(file_of_archive(&ok).unwrap(), "services: {}");
        // A symlink or a directory is not a file.
        let link = archive("x", "", tar::EntryType::Directory);
        assert!(file_of_archive(&link).is_err());
        let big = archive(
            "big.yml",
            &"a".repeat(MAX_FILE + 1),
            tar::EntryType::Regular,
        );
        assert!(file_of_archive(&big).is_err());
        assert!(file_of_archive(b"not a tar").is_err());
    }
}
