//! Building a service's image: its Git repository cloned (a deploy key for a
//! private one), or the Dockerfile written in the panel; Nixpacks when asked
//! (it writes the Dockerfile); then Docker builds the context. The image is
//! tagged `rdp/<project>_<service>:d<deployment>` on this node (one node:
//! nothing is pushed to a registry). Every step goes to the deployment's log,
//! and the deployment's lock is renewed all along (a build takes minutes).

use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use futures_util::StreamExt;

use crate::services::database_service::random_string;
use crate::services::deploy_service::Build;
use crate::services::deployment_service::{Outcome, RunLog};
use crate::services::docker_service::{BuildLine, Docker};

/// How long a clone, or Nixpacks, may take.
const STEP_TIME: Duration = Duration::from_secs(10 * 60);
/// The largest `.dockerignore` read.
const MAX_IGNORE: u64 = 1024 * 1024;
/// How often the deployment's lock is renewed while a step runs.
const RENEW: Duration = Duration::from_secs(30);
/// The largest context sent to Docker.
const MAX_CONTEXT: u64 = 1024 * 1024 * 1024;

/// A working directory, removed when done (whatever happened).
struct WorkDir(PathBuf);

impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn failed(message: impl Into<String>) -> Outcome {
    Outcome::Failed(message.into())
}

/// A path inside the context: relative, no `..` (`/` and empty: the root).
pub fn inside(path: &str) -> Option<PathBuf> {
    let path = Path::new(path.trim().trim_start_matches('/'));
    path.components()
        .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
        .then(|| path.to_path_buf())
}

/// A branch, tag or commit as Git takes it (never an option).
pub fn valid_ref(reference: &str) -> bool {
    !reference.is_empty()
        && !reference.starts_with('-')
        && reference.len() <= 255
        && reference
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/'))
}

/// A repository URL Git may clone: https, ssh (`git@host:org/repo`), never
/// a local path or another transport (`ext::`, `file://`).
pub fn valid_git_url(url: &str) -> bool {
    let url = url.trim();
    !url.is_empty()
        && url.len() <= 512
        && !url.contains(char::is_whitespace)
        && (url.starts_with("https://")
            || url.starts_with("ssh://")
            || (url.starts_with("git@") && url.contains(':')))
}

/// `work`, the deployment's lock renewed meanwhile; dropped (a process
/// killed) when the lock is lost.
async fn held<T>(log: &mut RunLog<'_>, work: impl Future<Output = T>) -> Result<T, Outcome> {
    tokio::pin!(work);
    let mut renew = tokio::time::interval(RENEW);
    renew.tick().await;
    loop {
        tokio::select! {
            done = &mut work => return Ok(done),
            _ = renew.tick() => log.hold().await?,
        }
    }
}

async fn run(
    log: &mut RunLog<'_>,
    what: &str,
    command: &mut tokio::process::Command,
) -> Result<String, Outcome> {
    command.kill_on_drop(true);
    let output = held(log, tokio::time::timeout(STEP_TIME, command.output()))
        .await?
        .map_err(|_| failed(format!("{what} took too long")))?
        .map_err(|e| failed(format!("{what} could not start: {e}")))?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    for line in text.lines().filter(|l| !l.trim().is_empty()).take(200) {
        log.line(line.trim_end()).await;
    }
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(failed(format!("{what} failed")))
    }
}

/// How Git reaches a repository: its deploy key (ssh), a GitHub token
/// (https, handed to Git apart from the URL), or neither (public).
#[derive(Clone, Copy, Default)]
pub(crate) struct GitAuth<'a> {
    pub deploy_key: Option<&'a str>,
    pub token: Option<&'a str>,
    /// Why its GitHub token may not be used (removed, or taken from the
    /// project): said when Git would need it.
    pub denied: Option<&'a str>,
}

/// A token is only GitHub's: `https://github.com/<owner>/<repo>`.
fn check_token_url(url: &str, auth: GitAuth<'_>) -> Result<(), Outcome> {
    if let Some(why) = auth.denied {
        return Err(failed(why));
    }
    if auth.token.is_some() && crate::services::github_service::repository_of(url).is_none() {
        return Err(failed(
            "a GitHub token clones only https://github.com/<owner>/<repo>",
        ));
    }
    Ok(())
}

/// Writes Git's credentials in `home` (a directory of their own, never in
/// a checkout or a build context, removed with it): the deploy key, the
/// token and the program that hands it to Git (`GIT_ASKPASS`).
fn write_credentials(home: &Path, auth: GitAuth<'_>) -> Result<(), Outcome> {
    use std::io::Write;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    if auth.deploy_key.is_none() && auth.token.is_none() {
        return Ok(());
    }
    if !home.is_dir() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(home)
            .map_err(|e| failed(format!("the credentials: {e}")))?;
    }
    let write = |name: &str, mode: u32, content: &[u8]| {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(home.join(name))
            .and_then(|mut file| file.write_all(content))
            .map_err(|e| failed(format!("the credentials: {e}")))
    };
    if let Some(key) = auth.deploy_key {
        write("key", 0o600, format!("{key}\n").as_bytes())?;
    }
    if let Some(token) = auth.token {
        write("token", 0o600, token.trim().as_bytes())?;
        // Asked for a user name, then for a password: the token, read from
        // its file (never an argument, nor in the environment).
        write(
            "askpass",
            0o700,
            b"#!/bin/sh\ncase \"$1\" in\n  Username*) echo x-access-token ;;\n  *) cat \"$RDP_GIT_TOKEN_FILE\" ;;\nesac\n",
        )?;
    }
    Ok(())
}

/// `git args` with `home` for its credentials ([`write_credentials`]): no
/// environment of the panel's, no credential helper or system
/// configuration (nothing stored, nothing asked).
fn git_command(home: &Path, auth: GitAuth<'_>, args: &[&str]) -> tokio::process::Command {
    let mut command = tokio::process::Command::new("git");
    command
        .args(["-c", "credential.helper=", "-c", "core.askPass="])
        .args(args)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", home)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0");
    if auth.deploy_key.is_some() {
        command.env(
            "GIT_SSH_COMMAND",
            format!(
                "ssh -i {} -o IdentitiesOnly=yes -o StrictHostKeyChecking=accept-new -o UserKnownHostsFile={}",
                home.join("key").display(),
                home.join("known_hosts").display()
            ),
        );
    }
    if auth.token.is_some() {
        command
            .env("GIT_ASKPASS", home.join("askpass"))
            .env("RDP_GIT_TOKEN_FILE", home.join("token"));
    }
    command
}

/// Clones `build`'s repository into `dir/src`: its commit.
async fn clone(
    build: &Build,
    dir: &Path,
    auth: GitAuth<'_>,
    log: &mut RunLog<'_>,
) -> Result<String, Outcome> {
    let url = build.git_url.as_deref().unwrap_or_default();
    if !valid_git_url(url) {
        return Err(failed("the repository's URL is not an https or ssh URL"));
    }
    check_token_url(url, auth)?;
    let branch = build
        .git_branch
        .as_deref()
        .map(str::trim)
        .filter(|b| !b.is_empty());
    if branch.is_some_and(|b| !valid_ref(b)) {
        return Err(failed("the branch is not a branch, tag or commit"));
    }
    let src = dir.join("src");
    // The credentials beside the checkout, never in it (nor in a build
    // context), removed once cloned (before any builder runs).
    let ssh = WorkDir(dir.join("ssh"));
    let ssh = &ssh.0;
    if let Some(token) = auth.token {
        log.hide(token);
    }
    write_credentials(ssh, auth)?;
    let git = |args: &[&str]| git_command(ssh, auth, args);
    log.line(format!(
        "cloning {url}{}",
        branch.map(|b| format!(" ({b})")).unwrap_or_default()
    ))
    .await;
    let src_text = src.to_string_lossy().into_owned();
    let mut args = vec!["clone", "--depth", "1", "--single-branch"];
    if let Some(branch) = branch {
        args.extend(["--branch", branch]);
    }
    args.extend(["--", url, &src_text]);
    if run(log, "the clone", &mut git(&args)).await.is_err() {
        // A commit is not a branch: the whole history, then that commit.
        let Some(commit) =
            branch.filter(|b| b.len() >= 7 && b.chars().all(|c| c.is_ascii_hexdigit()))
        else {
            return Err(failed(
                "the clone failed (its URL, its branch, or the deploy key)",
            ));
        };
        let _ = std::fs::remove_dir_all(&src);
        run(log, "the clone", &mut git(&["clone", "--", url, &src_text])).await?;
        run(
            log,
            "the checkout",
            &mut git(&["-C", &src_text, "checkout", "--detach", commit]),
        )
        .await?;
    }
    let commit = run(
        log,
        "reading the commit",
        &mut git(&["-C", &src_text, "rev-parse", "HEAD"]),
    )
    .await?;
    Ok(commit.trim().to_owned())
}

/// Brings `checkout` to `branch` of `url` (cloned the first time, fetched
/// after: files the repository does not track, such as a Compose service's
/// `./data`, stay): its commit. The deploy key lives in a temporary
/// directory meanwhile.
pub(crate) async fn sync_repo(
    url: &str,
    branch: Option<&str>,
    checkout: &Path,
    auth: GitAuth<'_>,
    log: &mut RunLog<'_>,
) -> Result<String, Outcome> {
    if !valid_git_url(url) {
        return Err(failed("the repository's URL is not an https or ssh URL"));
    }
    check_token_url(url, auth)?;
    let branch = branch.map(str::trim).filter(|b| !b.is_empty());
    if branch.is_some_and(|b| !valid_ref(b)) {
        return Err(failed("the branch is not a branch, tag or commit"));
    }
    let keys =
        WorkDir(std::env::temp_dir().join(format!("rdp-git-{}", random_string(16).to_lowercase())));
    let ssh = keys.0.join("ssh");
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&ssh)
            .map_err(|e| failed(format!("the deploy key: {e}")))?;
    }
    if let Some(token) = auth.token {
        log.hide(token);
    }
    write_credentials(&ssh, auth)?;
    let git = |args: &[&str]| git_command(&ssh, auth, args);
    let at = checkout.to_string_lossy().into_owned();
    let reference = branch.unwrap_or("HEAD");
    // The first time an empty repository (any reference fetches then, a
    // commit too); after, tracked files back as committed (`reset`), never
    // an untracked or ignored file overwritten (no `--force`,
    // `--no-overwrite-ignore`: a commit replacing `./data` fails the
    // deployment rather than erase it).
    if !checkout.join(".git").is_dir() {
        log.line(format!("cloning {url} ({reference})")).await;
        std::fs::create_dir_all(checkout).map_err(|e| failed(e.to_string()))?;
        run(log, "git init", &mut git(&["init", "-q", &at])).await?;
        run(
            log,
            "setting the URL",
            &mut git(&["-C", &at, "remote", "add", "origin", "--", url]),
        )
        .await?;
    } else {
        log.line(format!("fetching {url} ({reference})")).await;
        run(
            log,
            "setting the URL",
            &mut git(&["-C", &at, "remote", "set-url", "origin", "--", url]),
        )
        .await?;
        // Tracked files back as committed (when there is a commit: a first
        // fetch that failed left none).
        if git(&["-C", &at, "rev-parse", "-q", "--verify", "HEAD"])
            .output()
            .await
            .is_ok_and(|o| o.status.success())
        {
            run(
                log,
                "resetting tracked files",
                &mut git(&["-C", &at, "reset", "-q", "--hard", "HEAD"]),
            )
            .await?;
        }
    }
    run(
        log,
        "the fetch",
        &mut git(&[
            "-C", &at, "fetch", "--depth", "1", "origin", "--", reference,
        ]),
    )
    .await?;
    run(
        log,
        "the checkout",
        &mut git(&[
            "-C",
            &at,
            "checkout",
            "-q",
            "--no-overwrite-ignore",
            "--detach",
            "FETCH_HEAD",
        ]),
    )
    .await?;
    let commit = run(
        log,
        "reading the commit",
        &mut git(&["-C", &at, "rev-parse", "HEAD"]),
    )
    .await?;
    Ok(commit.trim().to_owned())
}

/// A `.dockerignore`: its rules in order, the last that matches wins.
struct Ignore(Vec<(globset::GlobMatcher, bool)>);

impl Ignore {
    /// `<Dockerfile>.dockerignore` first, else `.dockerignore` (as Docker):
    /// a regular file of the context (no link out), at most 1 MB.
    fn read(context: &Path, dockerfile: &Path) -> std::io::Result<Self> {
        use std::io::Read;
        let own = PathBuf::from(format!("{}.dockerignore", dockerfile.display()));
        let file = [context.join(own), context.join(".dockerignore")]
            .into_iter()
            .find(|p| p.symlink_metadata().is_ok_and(|m| m.is_file()) || p.is_symlink());
        let Some(file) = file else {
            return Ok(Self(vec![]));
        };
        let file = contained(context, &file)
            .filter(|p| p.is_file())
            .ok_or_else(|| std::io::Error::other("the .dockerignore leaves the build context"))?;
        let mut text = String::new();
        std::fs::File::open(file)?
            .take(MAX_IGNORE + 1)
            .read_to_string(&mut text)?;
        if text.len() as u64 > MAX_IGNORE {
            return Err(std::io::Error::other("the .dockerignore is over 1 MB"));
        }
        let mut rules = vec![];
        for line in text.lines().map(str::trim) {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (keep, pattern) = match line.strip_prefix('!') {
                Some(p) => (true, p.trim()),
                None => (false, line),
            };
            let pattern = pattern
                .trim_start_matches("./")
                .trim_start_matches('/')
                .trim_end_matches('/');
            // Docker's patterns: `*`, `?`, `[…]`, `**`, `\` escapes; braces
            // are plain characters.
            let escaped = pattern.replace('{', "\\{").replace('}', "\\}");
            let glob = globset::GlobBuilder::new(&escaped)
                .literal_separator(true)
                .backslash_escape(true)
                .build()
                .map_err(|e| {
                    std::io::Error::other(format!("the .dockerignore pattern {pattern}: {e}"))
                })?;
            rules.push((glob.compile_matcher(), keep));
        }
        Ok(Self(rules))
    }

    /// Whether `path` (relative) is left out: a rule matching it or one of
    /// its directories.
    fn excludes(&self, path: &Path) -> bool {
        let mut excluded = false;
        for (glob, keep) in &self.0 {
            if path
                .ancestors()
                .any(|p| !p.as_os_str().is_empty() && glob.is_match(p))
            {
                excluded = !keep;
            }
        }
        excluded
    }
}

/// `dir` as a tar: `.git` and what its `.dockerignore` lists left out (the
/// Dockerfile always in), symlinks kept as links.
fn pack(dir: &Path, dockerfile: &Path) -> std::io::Result<Vec<u8>> {
    struct Pack<'a> {
        builder: tar::Builder<Vec<u8>>,
        root: &'a Path,
        ignore: Ignore,
        dockerfile: &'a Path,
        size: u64,
    }
    fn walk(pack: &mut Pack<'_>, at: &Path) -> std::io::Result<()> {
        for entry in std::fs::read_dir(at)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_name() == ".git" {
                continue;
            }
            let relative = path.strip_prefix(pack.root).unwrap_or(&path).to_path_buf();
            let kind = entry.file_type()?;
            // A directory is walked even when left out: `!` may keep a file in it.
            if kind.is_dir() {
                if !pack.ignore.excludes(&relative) {
                    pack.builder.append_dir(&relative, &path)?;
                }
                walk(pack, &path)?;
                continue;
            }
            if relative != pack.dockerfile && pack.ignore.excludes(&relative) {
                continue;
            }
            pack.size += entry.metadata()?.len();
            if pack.size > MAX_CONTEXT {
                return Err(std::io::Error::other("the build context is over 1 GB"));
            }
            pack.builder.append_path_with_name(&path, &relative)?;
        }
        Ok(())
    }
    let mut builder = tar::Builder::new(Vec::new());
    builder.follow_symlinks(false);
    let mut pack = Pack {
        builder,
        root: dir,
        ignore: Ignore::read(dir, dockerfile)?,
        dockerfile,
        size: 0,
    };
    walk(&mut pack, dir)?;
    pack.builder.into_inner()
}

/// `path` with its links resolved, when it stays within `root` (resolved).
fn contained(root: &Path, path: &Path) -> Option<PathBuf> {
    let root = root.canonicalize().ok()?;
    let path = path.canonicalize().ok()?;
    path.starts_with(&root).then_some(path)
}

/// Builds `build` as `tag`: the image's reference (its tag, on this node).
pub(crate) async fn build(
    docker: &Docker,
    build: &Build,
    tag: &str,
    auth: GitAuth<'_>,
    no_cache: bool,
    log: &mut RunLog<'_>,
) -> Result<String, Outcome> {
    let dir = WorkDir(
        std::env::temp_dir().join(format!("rdp-build-{}", random_string(16).to_lowercase())),
    );
    std::fs::create_dir_all(&dir.0).map_err(|e| failed(format!("the build directory: {e}")))?;
    let context = match build.source.as_str() {
        "git" => {
            let commit = clone(build, &dir.0, auth, log).await?;
            log.line(format!("commit {commit}")).await;
            let path = inside(build.git_path.as_deref().unwrap_or("/"))
                .ok_or_else(|| failed("the build path leaves the repository"))?;
            // Links resolved: a link in the repository cannot leave it.
            contained(&dir.0.join("src"), &dir.0.join("src").join(path))
                .ok_or_else(|| failed("the build path is not a directory of the repository"))?
        }
        "dockerfile" => {
            let context = dir.0.join("context");
            std::fs::create_dir_all(&context)
                .and_then(|()| {
                    std::fs::write(
                        context.join("Dockerfile"),
                        build.dockerfile.as_deref().unwrap_or_default(),
                    )
                })
                .map_err(|e| failed(format!("the Dockerfile: {e}")))?;
            context
        }
        other => return Err(failed(format!("unknown source {other}"))),
    };
    // Resolved (as the Dockerfile's path is, to compare).
    let context = context
        .canonicalize()
        .ok()
        .filter(|c| c.is_dir())
        .ok_or_else(|| failed("the build path is not a directory of the repository"))?;
    let mut dockerfile = build
        .dockerfile_path
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .unwrap_or("Dockerfile")
        .to_owned();
    if build.source == "dockerfile" {
        dockerfile = "Dockerfile".to_owned();
    }
    if build.builder == "nixpacks" {
        log.line("nixpacks: writing the Dockerfile").await;
        let context_text = context.to_string_lossy().into_owned();
        let mut command = tokio::process::Command::new("nixpacks");
        command.args(["build", &context_text, "--out", &context_text]);
        run(log, "nixpacks", &mut command).await?;
        dockerfile = ".nixpacks/Dockerfile".to_owned();
    }
    let dockerfile_path = inside(&dockerfile)
        .and_then(|p| contained(&context, &context.join(p)))
        .filter(|p| p.is_file())
        .ok_or_else(|| failed(format!("no {dockerfile} in the build context")))?;
    let dockerfile_path = dockerfile_path
        .strip_prefix(&context)
        .map_err(|e| failed(e.to_string()))?
        .to_path_buf();
    let (to_pack, file) = (context.clone(), dockerfile_path.clone());
    let tar = held(
        log,
        tokio::task::spawn_blocking(move || pack(&to_pack, &file)),
    )
    .await?
    .map_err(|e| failed(e.to_string()))?
    .map_err(|e| failed(format!("the build context: {e}")))?;
    log.line(format!(
        "building {tag} ({} KB of context)",
        tar.len() / 1024
    ))
    .await;
    let mut lines =
        docker
            .engine()
            .build_image(tar, &dockerfile_path.to_string_lossy(), tag, no_cache);
    let mut image = None;
    // A quiet step (a long `RUN`) keeps the lock too.
    let mut renew = tokio::time::interval(RENEW);
    renew.tick().await;
    loop {
        let line = tokio::select! {
            line = lines.next() => line,
            _ = renew.tick() => {
                log.hold().await?;
                continue;
            }
        };
        let Some(line) = line else { break };
        match line.map_err(|e| failed(format!("the build: {e}")))? {
            BuildLine::Log(text) => {
                for line in text.lines().filter(|l| !l.trim().is_empty()) {
                    log.line(line.trim_end()).await;
                }
            }
            BuildLine::Image(id) => image = Some(id),
        }
    }
    image.ok_or_else(|| failed("the build made no image"))?;
    Ok(tag.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_reaches_git_apart() {
        use std::os::unix::fs::PermissionsExt;
        const TOKEN: &str = "ghp_secret_0123456789";
        let home =
            std::env::temp_dir().join(format!("rdp-cred-{}", random_string(8).to_lowercase()));
        let auth = GitAuth {
            token: Some(TOKEN),
            ..Default::default()
        };
        write_credentials(&home, auth).unwrap();
        let mode = |name: &str| {
            std::fs::metadata(home.join(name))
                .unwrap()
                .permissions()
                .mode()
                & 0o777
        };
        assert_eq!(mode(""), 0o700);
        assert_eq!(mode("token"), 0o600);
        // Neither an argument nor a variable of Git's: the file, read by askpass.
        let command = git_command(
            &home,
            auth,
            &["ls-remote", "--", "https://github.com/o/r.git"],
        );
        let std = command.as_std();
        assert!(std.get_args().all(|a| !a.to_string_lossy().contains(TOKEN)));
        assert!(
            std.get_envs()
                .all(|(_, v)| v.is_none_or(|v| !v.to_string_lossy().contains(TOKEN)))
        );
        let ask = |prompt: &str| {
            let out = std::process::Command::new(home.join("askpass"))
                .arg(prompt)
                .env("RDP_GIT_TOKEN_FILE", home.join("token"))
                .output()
                .unwrap();
            String::from_utf8(out.stdout).unwrap()
        };
        assert_eq!(
            ask("Username for 'https://github.com': ").trim(),
            "x-access-token"
        );
        assert_eq!(
            ask("Password for 'https://x-access-token@github.com': "),
            TOKEN
        );
        // Only for GitHub.
        assert!(check_token_url("https://github.com/o/r.git", auth).is_ok());
        assert!(check_token_url("https://evil.example/o/r.git", auth).is_err());
        assert!(check_token_url("https://evil.example/o/r.git", GitAuth::default()).is_ok());
        let denied = GitAuth {
            denied: Some("its GitHub token was removed"),
            ..Default::default()
        };
        assert!(check_token_url("https://github.com/o/r.git", denied).is_err());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn paths_and_urls_stay_in_bounds() {
        assert_eq!(inside("/"), Some(PathBuf::new()));
        assert_eq!(inside("/app/web"), Some(PathBuf::from("app/web")));
        assert_eq!(inside("../etc"), None);
        assert_eq!(inside("a/../../b"), None);
        assert!(valid_git_url("https://github.com/org/app.git"));
        assert!(valid_git_url("git@github.com:org/app.git"));
        for bad in [
            "file:///etc",
            "/srv/repo",
            "ext::sh -c x",
            "https://a b",
            "--upload-pack=x",
        ] {
            assert!(!valid_git_url(bad), "{bad}");
        }
        assert!(valid_ref("main") && valid_ref("v1.2.3") && valid_ref("feature/x"));
        assert!(!valid_ref("--force") && !valid_ref("a b"));
    }

    fn scratch() -> WorkDir {
        let dir =
            std::env::temp_dir().join(format!("rdp-test-{}", random_string(12).to_lowercase()));
        std::fs::create_dir_all(&dir).unwrap();
        WorkDir(dir.canonicalize().unwrap())
    }

    #[test]
    fn a_link_cannot_leave_the_repository() {
        let dir = scratch();
        let src = dir.0.join("src");
        std::fs::create_dir_all(src.join("web")).unwrap();
        std::fs::write(dir.0.join("key"), "secret").unwrap();
        std::os::unix::fs::symlink("..", src.join("escape")).unwrap();
        assert_eq!(contained(&src, &src.join("web")), Some(src.join("web")));
        assert_eq!(contained(&src, &src.join("escape")), None);
        assert_eq!(contained(&src, &src.join("escape/key")), None);
    }

    #[test]
    fn dockerignore_leaves_files_out() {
        let dir = scratch();
        let root = &dir.0;
        for (path, body) in [
            ("Dockerfile", "FROM x"),
            (".env", "SECRET=1"),
            ("app/main.rs", ""),
            ("logs/a.log", ""),
            ("logs/keep.log", ""),
            ("docs/x.md", ""),
            (
                ".dockerignore",
                "# comment\n.env\nlogs\n!logs/keep.log\n**/*.md\nDockerfile\n",
            ),
        ] {
            std::fs::create_dir_all(root.join(path).parent().unwrap()).unwrap();
            std::fs::write(root.join(path), body).unwrap();
        }
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join(".git/config"), "").unwrap();
        let tar = pack(root, Path::new("Dockerfile")).unwrap();
        let mut archive = tar::Archive::new(tar.as_slice());
        let mut names: Vec<String> = archive
            .entries()
            .unwrap()
            .map(|e| e.unwrap().path().unwrap().display().to_string())
            .filter(|n| !n.ends_with('/'))
            .collect();
        names.sort();
        // The Dockerfile always; `!` keeps a file of a left-out directory.
        assert_eq!(
            names,
            [
                ".dockerignore",
                "Dockerfile",
                "app",
                "app/main.rs",
                "docs",
                "logs/keep.log"
            ]
        );
    }
}
