//! Where backups are written: a storage provider opened as a [`Store`], a
//! directory of this server (`local`) or an S3-compatible bucket (`s3`,
//! requests signed by `rusty-s3`, sent by `reqwest`). Keys are folders and
//! file names the panel checks ([`valid_key`]): never `..`, never absolute.
//! Under a local root every folder is opened without following symlinks
//! (`openat`, `O_NOFOLLOW`): a link in the root never leads elsewhere.

use std::path::{Path, PathBuf};
use std::time::Duration;

use futures_util::StreamExt;
use rusty_s3::{Bucket, Credentials, S3Action, UrlStyle};
use tokio::io::AsyncWriteExt;

use crate::models::storage_provider;

/// How long a signed request stays valid.
const SIGNED_FOR: Duration = Duration::from_secs(60 * 60);

/// A folder or a file of a provider: segments of letters, digits, `.`, `_`,
/// `-` (no `.` or `..` alone), joined by `/`; at most 512 bytes.
pub fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 512
        && key.split('/').all(|segment| {
            !segment.is_empty()
                && segment != "."
                && segment != ".."
                && segment
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        })
}

/// An S3 endpoint the panel calls: `https://` (or `http://` for a bucket on
/// a private network), with a host.
pub fn valid_endpoint(endpoint: &str) -> bool {
    reqwest::Url::parse(endpoint).is_ok_and(|u| {
        matches!(u.scheme(), "https" | "http") && u.host_str().is_some() && u.username().is_empty()
    })
}

/// A provider, opened.
pub enum Store {
    Local(PathBuf),
    S3 {
        bucket: Bucket,
        credentials: Credentials,
        client: reqwest::Client,
    },
}

impl Store {
    /// Opens `row` (its secret key opened with `keyring`).
    pub fn open(
        row: &storage_provider::Model,
        keyring: &rustcms::crypto::Keyring,
    ) -> Result<Self, String> {
        match row.kind.as_str() {
            "local" => {
                let root = PathBuf::from(row.local_path.as_deref().unwrap_or_default());
                if !root.is_absolute() {
                    return Err("the directory is not an absolute path".into());
                }
                Ok(Self::Local(root))
            }
            "s3" => {
                let endpoint = row.endpoint.as_deref().unwrap_or_default();
                if !valid_endpoint(endpoint) {
                    return Err("the endpoint is not an http(s) URL".into());
                }
                let style = if row.path_style {
                    UrlStyle::Path
                } else {
                    UrlStyle::VirtualHost
                };
                let bucket = Bucket::new(
                    endpoint.parse().map_err(|e| format!("the endpoint: {e}"))?,
                    style,
                    row.bucket.clone().unwrap_or_default(),
                    row.region
                        .clone()
                        .filter(|r| !r.is_empty())
                        .unwrap_or_else(|| "us-east-1".to_owned()),
                )
                .map_err(|e| format!("the bucket: {e}"))?;
                let secret = match &row.secret_key {
                    Some(sealed) => sealed
                        .open(keyring, &storage_provider::secret_key_context())
                        .map_err(|_| "the secret key cannot be read (another APP_KEY)")?,
                    None => String::new(),
                };
                let credentials =
                    Credentials::new(row.access_key.clone().unwrap_or_default(), secret);
                let client = reqwest::Client::builder()
                    .connect_timeout(Duration::from_secs(15))
                    .build()
                    .map_err(|e| e.to_string())?;
                Ok(Self::S3 {
                    bucket,
                    credentials,
                    client,
                })
            }
            other => Err(format!("unknown provider kind {other}")),
        }
    }

    fn local_path(root: &Path, key: &str) -> Result<PathBuf, String> {
        if !valid_key(key) {
            return Err(format!("{key} is not a valid path"));
        }
        Ok(root.join(key))
    }

    /// Writes `file` as `key`: its size.
    pub async fn put(&self, key: &str, file: &Path) -> Result<u64, String> {
        let size = tokio::fs::metadata(file)
            .await
            .map_err(|e| e.to_string())?
            .len();
        match self {
            Self::Local(root) => {
                Self::local_path(root, key)?;
                let (root, key, file) = (root.clone(), key.to_owned(), file.to_path_buf());
                blocking(move || local::put(&root, &key, &file)).await?;
            }
            Self::S3 {
                bucket,
                credentials,
                client,
            } => {
                if !valid_key(key) {
                    return Err(format!("{key} is not a valid path"));
                }
                let url = bucket.put_object(Some(credentials), key).sign(SIGNED_FOR);
                let body = tokio::fs::File::open(file)
                    .await
                    .map_err(|e| e.to_string())?;
                let stream = tokio_util_stream(body);
                let response = client
                    .put(url.as_str())
                    .header(reqwest::header::CONTENT_LENGTH, size)
                    .body(reqwest::Body::wrap_stream(stream))
                    .send()
                    .await
                    .map_err(|e| format!("the upload: {}", e.without_url()))?;
                ok(response, "the upload").await?;
            }
        }
        Ok(size)
    }

    /// Reads `key` into `to`: its size.
    pub async fn get(&self, key: &str, to: &Path) -> Result<u64, String> {
        match self {
            Self::Local(root) => {
                Self::local_path(root, key)?;
                let (root, key, to) = (root.clone(), key.to_owned(), to.to_path_buf());
                blocking(move || local::get(&root, &key, &to)).await
            }
            Self::S3 {
                bucket,
                credentials,
                client,
            } => {
                if !valid_key(key) {
                    return Err(format!("{key} is not a valid path"));
                }
                let url = bucket.get_object(Some(credentials), key).sign(SIGNED_FOR);
                let response = client
                    .get(url.as_str())
                    .send()
                    .await
                    .map_err(|e| format!("the download: {}", e.without_url()))?;
                let response = ok(response, "the download").await?;
                let mut file = tokio::fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .mode(0o600)
                    .open(to)
                    .await
                    .map_err(|e| e.to_string())?;
                let mut size = 0;
                let mut body = response.bytes_stream();
                while let Some(chunk) = body.next().await {
                    let chunk = chunk.map_err(|e| format!("the download: {}", e.without_url()))?;
                    size += chunk.len() as u64;
                    file.write_all(&chunk).await.map_err(|e| e.to_string())?;
                }
                file.flush().await.map_err(|e| e.to_string())?;
                Ok(size)
            }
        }
    }

    /// The files directly in the folder `folder`: (key, size), by name.
    pub async fn list(&self, folder: &str) -> Result<Vec<(String, u64)>, String> {
        let mut found = vec![];
        match self {
            Self::Local(root) => {
                Self::local_path(root, folder)?;
                let (root, key) = (root.clone(), folder.to_owned());
                found = blocking(move || local::list(&root, &key)).await?;
            }
            Self::S3 {
                bucket,
                credentials,
                client,
            } => {
                if !valid_key(folder) {
                    return Err(format!("{folder} is not a valid path"));
                }
                let prefix = format!("{folder}/");
                let mut token: Option<String> = None;
                loop {
                    let mut action = bucket.list_objects_v2(Some(credentials));
                    action.with_prefix(prefix.as_str());
                    action.with_delimiter("/");
                    if let Some(token) = &token {
                        action.with_continuation_token(token.as_str());
                    }
                    let url = action.sign(SIGNED_FOR);
                    let response = client
                        .get(url.as_str())
                        .send()
                        .await
                        .map_err(|e| format!("the listing: {}", e.without_url()))?;
                    let text = ok(response, "the listing")
                        .await?
                        .text()
                        .await
                        .map_err(|e| e.without_url().to_string())?;
                    let page = rusty_s3::actions::ListObjectsV2::parse_response(&text)
                        .map_err(|e| format!("the listing: {e}"))?;
                    found.extend(page.contents.into_iter().map(|c| (c.key, c.size)));
                    match page.next_continuation_token {
                        Some(next) => token = Some(next),
                        None => break,
                    }
                }
            }
        }
        found.sort();
        Ok(found)
    }

    pub async fn delete(&self, key: &str) -> Result<(), String> {
        match self {
            Self::Local(root) => {
                Self::local_path(root, key)?;
                let (root, key) = (root.clone(), key.to_owned());
                blocking(move || local::delete(&root, &key)).await
            }
            Self::S3 {
                bucket,
                credentials,
                client,
            } => {
                if !valid_key(key) {
                    return Err(format!("{key} is not a valid path"));
                }
                let url = bucket
                    .delete_object(Some(credentials), key)
                    .sign(SIGNED_FOR);
                let response = client
                    .delete(url.as_str())
                    .send()
                    .await
                    .map_err(|e| format!("the deletion: {}", e.without_url()))?;
                ok(response, "the deletion").await.map(|_| ())
            }
        }
    }

    /// Writes, lists and deletes a small file (the "Test" button).
    pub async fn check(&self) -> Result<(), String> {
        let probe = std::env::temp_dir().join(format!(
            "rdp-probe-{}",
            crate::services::database_service::random_string(12).to_lowercase()
        ));
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&probe)
            .await
            .map_err(|e| e.to_string())?;
        file.write_all(b"rdockpanel")
            .await
            .map_err(|e| e.to_string())?;
        drop(file);
        let key = format!(
            "rdockpanel-check/{}.txt",
            crate::services::database_service::random_string(12).to_lowercase()
        );
        let result = async {
            self.put(&key, &probe).await?;
            if !self
                .list("rdockpanel-check")
                .await?
                .iter()
                .any(|(k, _)| *k == key)
            {
                return Err("the file written is not listed".to_owned());
            }
            self.delete(&key).await
        }
        .await;
        let _ = tokio::fs::remove_file(&probe).await;
        result
    }
}

/// `work` on a blocking thread; its error in words.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> std::io::Result<T> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

/// A local root's files, every folder opened without following links.
mod local {
    use std::fs::File;
    use std::io::{self, Read, Write};
    use std::os::fd::OwnedFd;
    use std::os::unix::fs::OpenOptionsExt;
    use std::path::Path;

    use rustix::fs::{AtFlags, Dir, FileType, Mode, OFlags};

    const DIR: OFlags = OFlags::DIRECTORY
        .union(OFlags::RDONLY)
        .union(OFlags::NOFOLLOW)
        .union(OFlags::CLOEXEC);

    /// The folders of `key` and its last name.
    fn split(key: &str) -> (Vec<&str>, &str) {
        let mut parts: Vec<&str> = key.split('/').collect();
        let name = parts.pop().unwrap_or_default();
        (parts, name)
    }

    /// The folder `dirs` under `root`, made (`0700`) when `create`.
    fn folder(root: &Path, dirs: &[&str], create: bool) -> io::Result<OwnedFd> {
        if create {
            std::fs::create_dir_all(root)?;
        }
        let mut fd = rustix::fs::open(
            root,
            OFlags::DIRECTORY | OFlags::RDONLY | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        for dir in dirs {
            if create {
                match rustix::fs::mkdirat(&fd, *dir, Mode::from_raw_mode(0o700)) {
                    Ok(()) | Err(rustix::io::Errno::EXIST) => {}
                    Err(e) => return Err(e.into()),
                }
            }
            fd = rustix::fs::openat(&fd, *dir, DIR, Mode::empty())?;
        }
        Ok(fd)
    }

    /// Writes `source` as `key`: aside (`0600`, a new file), then renamed.
    pub fn put(root: &Path, key: &str, source: &Path) -> io::Result<()> {
        let (dirs, name) = split(key);
        let dir = folder(root, &dirs, true)?;
        let partial = format!(
            ".{name}.{}.partial",
            crate::services::database_service::random_string(8).to_lowercase()
        );
        let fd = rustix::fs::openat(
            &dir,
            partial.as_str(),
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )?;
        let written = (|| {
            let mut out = File::from(fd);
            io::copy(&mut File::open(source)?, &mut out)?;
            out.sync_all()?;
            rustix::fs::renameat(&dir, partial.as_str(), &dir, name)?;
            Ok(())
        })();
        if written.is_err() {
            let _ = rustix::fs::unlinkat(&dir, partial.as_str(), AtFlags::empty());
        }
        written
    }

    /// Reads `key` (a regular file, not a link) into `to` (`0600`).
    pub fn get(root: &Path, key: &str, to: &Path) -> io::Result<u64> {
        let (dirs, name) = split(key);
        let dir = folder(root, &dirs, false)?;
        let fd = rustix::fs::openat(
            &dir,
            name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        let mut from = File::from(fd);
        if !from.metadata()?.is_file() {
            return Err(io::Error::other(format!("{key} is not a file")));
        }
        let mut out = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(to)?;
        let mut buf = vec![0; 256 * 1024];
        let mut size = 0;
        loop {
            let n = from.read(&mut buf)?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n])?;
            size += n as u64;
        }
        Ok(size)
    }

    /// The regular files of `folder` (none when it does not exist).
    pub fn list(root: &Path, folder_key: &str) -> io::Result<Vec<(String, u64)>> {
        let dirs: Vec<&str> = folder_key.split('/').collect();
        let dir = match folder(root, &dirs, false) {
            Ok(dir) => dir,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(e),
        };
        let mut found = vec![];
        for entry in Dir::read_from(&dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            // Dot files: the panel's own writes in progress, or hidden.
            if name.starts_with('.') {
                continue;
            }
            let stat = rustix::fs::statat(&dir, name.as_str(), AtFlags::SYMLINK_NOFOLLOW)?;
            if FileType::from_raw_mode(stat.st_mode as _) == FileType::RegularFile {
                found.push((format!("{folder_key}/{name}"), stat.st_size as u64));
            }
        }
        Ok(found)
    }

    /// Removes `key` (a link is removed, never followed).
    pub fn delete(root: &Path, key: &str) -> io::Result<()> {
        let (dirs, name) = split(key);
        let dir = folder(root, &dirs, false)?;
        rustix::fs::unlinkat(&dir, name, AtFlags::empty())?;
        Ok(())
    }
}

/// A file as a stream of bytes (what `reqwest` sends).
fn tokio_util_stream(
    file: tokio::fs::File,
) -> impl futures_util::Stream<Item = std::io::Result<Vec<u8>>> + Send + 'static {
    futures_util::stream::unfold(file, |mut file| async move {
        use tokio::io::AsyncReadExt;
        let mut buf = vec![0; 256 * 1024];
        match file.read(&mut buf).await {
            Ok(0) => None,
            Ok(n) => {
                buf.truncate(n);
                Some((Ok(buf), file))
            }
            Err(e) => Some((Err(e), file)),
        }
    })
}

/// The response when it succeeded; else S3's message.
async fn ok(response: reqwest::Response, what: &str) -> Result<reqwest::Response, String> {
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    let code = text
        .split("<Code>")
        .nth(1)
        .and_then(|t| t.split("</Code>").next())
        .unwrap_or("");
    Err(format!("{what}: {status} {code}").trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_stay_in_their_folder() {
        assert!(valid_key("production/orders/db-20261019T020000Z.sql.gz"));
        for bad in [
            "", "/etc", "a/../b", "a//b", "a/./b", "a b", "a/b/", "..", "c:\\x",
        ] {
            assert!(!valid_key(bad), "{bad}");
        }
        assert!(valid_endpoint("https://s3.eu-west-1.amazonaws.com"));
        assert!(valid_endpoint("http://minio:9000"));
        assert!(!valid_endpoint("file:///etc") && !valid_endpoint("https://u:p@x"));
    }

    #[tokio::test]
    async fn a_local_store_writes_lists_reads_and_deletes() {
        let root = std::env::temp_dir().join(format!(
            "rdp-store-{}",
            crate::services::database_service::random_string(10).to_lowercase()
        ));
        let store = Store::Local(root.clone());
        store.check().await.unwrap();
        let file = root.with_extension("src");
        tokio::fs::write(&file, b"dump").await.unwrap();
        assert_eq!(store.put("db/a-1.sql.gz", &file).await.unwrap(), 4);
        store.put("db/a-2.sql.gz", &file).await.unwrap();
        let listed = store.list("db").await.unwrap();
        assert_eq!(
            listed,
            [
                ("db/a-1.sql.gz".to_owned(), 4),
                ("db/a-2.sql.gz".to_owned(), 4)
            ]
        );
        let back = root.with_extension("back");
        store.get("db/a-1.sql.gz", &back).await.unwrap();
        assert_eq!(tokio::fs::read(&back).await.unwrap(), b"dump");
        store.delete("db/a-1.sql.gz").await.unwrap();
        assert_eq!(store.list("db").await.unwrap().len(), 1);
        assert!(store.put("../x", &file).await.is_err());
        // A link in the root never leads out of it.
        let outside = root.with_extension("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret.sql.gz"), "theirs").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("escape")).unwrap();
        assert!(store.put("escape/x.sql.gz", &file).await.is_err());
        assert!(store.get("escape/secret.sql.gz", &back).await.is_err());
        assert!(store.list("escape").await.is_err());
        assert!(store.delete("escape/secret.sql.gz").await.is_err());
        std::os::unix::fs::symlink(outside.join("secret.sql.gz"), root.join("db/link.sql.gz"))
            .unwrap();
        assert!(store.get("db/link.sql.gz", &back).await.is_err());
        assert!(
            !store
                .list("db")
                .await
                .unwrap()
                .iter()
                .any(|(k, _)| k.contains("link"))
        );
        assert!(outside.join("secret.sql.gz").exists());
        let _ = std::fs::remove_dir_all(&outside);
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_file(&file);
        let _ = std::fs::remove_file(&back);
    }
}
