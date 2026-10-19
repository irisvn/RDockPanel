//! GitHub, with a token of Settings → Git: who it signs in as, the
//! repositories it reaches, their branches. Only `api.github.com` is asked
//! (redirects never followed: the token goes nowhere else), and a
//! repository is cloned from `https://github.com/<owner>/<repo>.git`, its
//! token given to Git apart from the URL (`build_service`).

use std::time::Duration;

use rustcms::{AppError, AppResult, ValidationErrors};
use serde::Deserialize;
use serde_json::{Value, json};

/// The API's address (tests point it to a local server).
pub const API: &str = "https://api.github.com";
/// Where repositories are cloned from.
pub const CLONE_HOST: &str = "https://github.com/";
const TIMEOUT: Duration = Duration::from_secs(10);
/// The most pages of 100 read (repositories, branches).
const PAGES: u32 = 10;

/// The client, bound in the app (its address replaced in tests).
#[derive(Clone)]
pub struct GitHub {
    pub api: String,
}

impl Default for GitHub {
    fn default() -> Self {
        Self {
            api: API.to_owned(),
        }
    }
}

/// An owner or repository name as GitHub allows it.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 100
        && name != "."
        && name != ".."
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// `owner/repo` of a clone URL of GitHub (`https://github.com/o/r.git`).
pub fn repository_of(url: &str) -> Option<(String, String)> {
    let rest = url.trim().strip_prefix(CLONE_HOST)?;
    let rest = rest.strip_suffix(".git").unwrap_or(rest);
    let (owner, repo) = rest.split_once('/')?;
    (valid_name(owner) && valid_name(repo)).then(|| (owner.to_owned(), repo.to_owned()))
}

/// The clone URL of `owner/repo` (checked names).
pub fn clone_url(owner: &str, repo: &str) -> String {
    format!("{CLONE_HOST}{owner}/{repo}.git")
}

#[derive(Debug, Deserialize)]
struct User {
    login: String,
}

#[derive(Debug, Deserialize)]
struct Repo {
    full_name: String,
    private: bool,
    default_branch: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Branch {
    name: String,
}

impl GitHub {
    fn client(&self) -> AppResult<reqwest::Client> {
        reqwest::Client::builder()
            .timeout(TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("RDockPanel/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(AppError::internal)
    }

    /// GET `path` with `token`: its JSON, or what GitHub said (never the token).
    async fn get(&self, token: &str, path: &str) -> AppResult<reqwest::Response> {
        let response = self
            .client()?
            .get(format!("{}{path}", self.api))
            .bearer_auth(token)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await
            .map_err(|e| refused(format!("GitHub did not answer: {}", e.without_url())))?;
        match response.status().as_u16() {
            200 => Ok(response),
            401 => Err(refused("GitHub refused the token (revoked or mistyped?)")),
            403 | 429 => Err(refused(
                "GitHub refused: the token lacks access, or its rate limit is reached",
            )),
            404 => Err(refused("GitHub: not found, or the token cannot see it")),
            status => Err(refused(format!("GitHub answered {status}"))),
        }
    }

    /// Who the token signs in as, and its scopes (classic tokens say them).
    pub async fn check(&self, token: &str) -> AppResult<Value> {
        let response = self.get(token, "/user").await?;
        let scopes = response
            .headers()
            .get("x-oauth-scopes")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let user: User = response
            .json()
            .await
            .map_err(|_| refused("GitHub's answer was not understood"))?;
        Ok(json!({ "account": user.login, "scopes": scopes }))
    }

    /// The repositories the token reaches (owned, collaborated, its
    /// organizations'), by name.
    pub async fn repositories(&self, token: &str) -> AppResult<Vec<Value>> {
        let mut all = vec![];
        for page in 1..=PAGES {
            let repos: Vec<Repo> = self
                .get(
                    token,
                    &format!("/user/repos?per_page=100&page={page}&sort=pushed&affiliation=owner,collaborator,organization_member"),
                )
                .await?
                .json()
                .await
                .map_err(|_| refused("GitHub's answer was not understood"))?;
            let last = repos.len() < 100;
            all.extend(repos.into_iter().filter_map(|r| {
                let (owner, name) = r.full_name.split_once('/')?;
                (valid_name(owner) && valid_name(name)).then(|| {
                    json!({
                        "full_name": r.full_name,
                        "private": r.private,
                        "default_branch": r.default_branch,
                        "url": clone_url(owner, name),
                    })
                })
            }));
            if last {
                break;
            }
        }
        Ok(all)
    }

    /// `owner/repo`'s branches.
    pub async fn branches(&self, token: &str, owner: &str, repo: &str) -> AppResult<Vec<String>> {
        if !valid_name(owner) || !valid_name(repo) {
            return Err(ValidationErrors::field("repo", "is not a repository of GitHub").into());
        }
        let mut all = vec![];
        for page in 1..=PAGES {
            let branches: Vec<Branch> = self
                .get(
                    token,
                    &format!("/repos/{owner}/{repo}/branches?per_page=100&page={page}"),
                )
                .await?
                .json()
                .await
                .map_err(|_| refused("GitHub's answer was not understood"))?;
            let last = branches.len() < 100;
            all.extend(branches.into_iter().map(|b| b.name));
            if last {
                break;
            }
        }
        Ok(all)
    }
}

fn refused(message: impl Into<String>) -> AppError {
    ValidationErrors::field("token", message.into()).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repositories() {
        assert_eq!(
            repository_of("https://github.com/irisvn/rdockpanel.git"),
            Some(("irisvn".into(), "rdockpanel".into()))
        );
        assert_eq!(
            repository_of("https://github.com/a-b/c.d"),
            Some(("a-b".into(), "c.d".into()))
        );
        for bad in [
            "https://gitlab.com/a/b.git",
            "https://github.com.evil.io/a/b.git",
            "https://github.com/a/b/c.git",
            "https://github.com/../b.git",
            "https://x@github.com/a/b.git",
            "git@github.com:a/b.git",
        ] {
            assert_eq!(repository_of(bad), None, "{bad}");
        }
        assert_eq!(clone_url("o", "r"), "https://github.com/o/r.git");
    }
}
