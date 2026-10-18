//! Where backups are written: a storage provider opened as a [`Store`], a
//! directory of this server (`local`) or an S3-compatible bucket (`s3`,
//! requests signed by `rusty-s3`, sent by `reqwest`). Keys are folders and
//! file names the panel checks ([`valid_key`]): never `..`, never absolute.

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
                let to = Self::local_path(root, key)?;
                if let Some(dir) = to.parent() {
                    tokio::fs::create_dir_all(dir)
                        .await
                        .map_err(|e| format!("{}: {e}", dir.display()))?;
                }
                // Written aside, then renamed: never a half file under its name.
                let partial = to.with_extension("partial");
                tokio::fs::copy(file, &partial)
                    .await
                    .map_err(|e| format!("{}: {e}", partial.display()))?;
                tokio::fs::rename(&partial, &to)
                    .await
                    .map_err(|e| e.to_string())?;
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
                    .map_err(|e| format!("the upload: {e}"))?;
                ok(response, "the upload").await?;
            }
        }
        Ok(size)
    }

    /// Reads `key` into `to`: its size.
    pub async fn get(&self, key: &str, to: &Path) -> Result<u64, String> {
        match self {
            Self::Local(root) => {
                let from = Self::local_path(root, key)?;
                tokio::fs::copy(&from, to)
                    .await
                    .map_err(|e| format!("{key}: {e}"))
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
                    .map_err(|e| format!("the download: {e}"))?;
                let response = ok(response, "the download").await?;
                let mut file = tokio::fs::File::create(to)
                    .await
                    .map_err(|e| e.to_string())?;
                let mut size = 0;
                let mut body = response.bytes_stream();
                while let Some(chunk) = body.next().await {
                    let chunk = chunk.map_err(|e| format!("the download: {e}"))?;
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
                let dir = Self::local_path(root, folder)?;
                let mut entries = match tokio::fs::read_dir(&dir).await {
                    Ok(entries) => entries,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
                    Err(e) => return Err(format!("{}: {e}", dir.display())),
                };
                while let Some(entry) = entries.next_entry().await.map_err(|e| e.to_string())? {
                    let meta = entry.metadata().await.map_err(|e| e.to_string())?;
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if meta.is_file() && !name.ends_with(".partial") {
                        found.push((format!("{folder}/{name}"), meta.len()));
                    }
                }
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
                        .map_err(|e| format!("the listing: {e}"))?;
                    let text = ok(response, "the listing")
                        .await?
                        .text()
                        .await
                        .map_err(|e| e.to_string())?;
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
                let path = Self::local_path(root, key)?;
                tokio::fs::remove_file(&path)
                    .await
                    .map_err(|e| format!("{key}: {e}"))
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
                    .map_err(|e| format!("the deletion: {e}"))?;
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
        tokio::fs::write(&probe, b"rdockpanel")
            .await
            .map_err(|e| e.to_string())?;
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
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_file(&file);
        let _ = std::fs::remove_file(&back);
    }
}
