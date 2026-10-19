//! The public API's tokens (Settings → API tokens): RustCMS's personal
//! access tokens (`rcms_…`), each with scopes that expand to exact
//! permissions (never `*`). A token acts as its user — their roles, and
//! their roles in projects — and never beyond its scopes: both must allow.

use rustcms::auth::{ADMIN_GUARD, AuthState};
use rustcms::{AppError, AppResult, AppState, Principal};

/// The scopes a token may have, and what each allows.
pub const SCOPES: &[(&str, &[&str])] = &[
    (
        "read",
        &[
            "projects.view",
            "projects.view_any",
            "services.view",
            "services.view_any",
            "deployments.view",
            "deployments.view_any",
            "service_domains.view",
            "service_domains.view_any",
        ],
    ),
    ("deploy", &["services.deploy"]),
    (
        "write",
        &[
            "services.update",
            "services.create",
            "service_domains.create",
            "service_domains.update",
            "service_domains.delete",
        ],
    ),
    ("secrets", &["services.secrets"]),
    ("logs", &["services.logs"]),
];

/// The permissions `scopes` allow: every scope known, at least one (a token
/// without scopes would be RustCMS's `*`).
pub fn abilities(scopes: &[String]) -> Result<Vec<String>, String> {
    if scopes.is_empty() {
        return Err("choose at least one scope".to_owned());
    }
    let mut out = vec![];
    for scope in scopes {
        let Some((_, permissions)) = SCOPES.iter().find(|(name, _)| name == scope) else {
            return Err(format!("{scope} is not a scope"));
        };
        out.extend(permissions.iter().map(|p| (*p).to_owned()));
    }
    out.sort();
    out.dedup();
    Ok(out)
}

/// The scopes behind `abilities` (a token's, as stored).
pub fn scopes_of(abilities: &std::collections::HashSet<String>) -> Vec<&'static str> {
    SCOPES
        .iter()
        .filter(|(_, permissions)| permissions.iter().all(|p| abilities.contains(*p)))
        .map(|(name, _)| *name)
        .collect()
}

/// Who a token acts as: its user's admin roles, capped by its abilities, and
/// those abilities (a project role's grant is capped by them too:
/// [`ApiCaller::can`]).
pub struct ApiCaller {
    pub principal: Principal,
    pub abilities: std::collections::HashSet<String>,
}

impl ApiCaller {
    /// The token's scopes allow `permission`.
    pub fn scoped(&self, permission: &str) -> bool {
        self.abilities.contains(permission)
    }

    /// `permission` in `project`: the token allows it, and the user may
    /// (a role's, or their role's in the project).
    pub async fn can<C: sea_orm::ConnectionTrait>(
        &self,
        db: &C,
        project: i64,
        permission: &str,
    ) -> AppResult<bool> {
        Ok(self.scoped(permission)
            && crate::services::access_service::can(db, &self.principal, project, permission)
                .await?)
    }

    /// The service `id` for `permission` (403 before any lookup when the
    /// token's scopes do not allow it).
    pub async fn service<C: sea_orm::ConnectionTrait>(
        &self,
        db: &C,
        id: i64,
        permission: &str,
    ) -> AppResult<crate::models::service::Model> {
        if !self.scoped(permission) {
            return Err(AppError::Forbidden);
        }
        crate::services::access_service::service(db, &self.principal, id, permission).await
    }
}

/// The caller of an API request: its token (`Auth` on the API guard), its
/// user as the admin panel knows them, capped by the token.
pub async fn caller(app: &AppState, auth: &rustcms::prelude::Auth) -> AppResult<ApiCaller> {
    let api = auth.principal();
    let Some(rustcms::Credential::Token(token_id)) = api.credential.clone() else {
        return Err(AppError::Unauthenticated);
    };
    use sea_orm::EntityTrait;
    let token = rustcms::auth::entities::personal_access_token::Entity::find_by_id(token_id)
        .one(app.db())
        .await?
        .ok_or(AppError::Unauthenticated)?;
    let abilities: std::collections::HashSet<String> =
        serde_json::from_str::<Vec<String>>(&token.abilities)
            .unwrap_or_default()
            .into_iter()
            .collect();
    // Only what the scopes allow: a token of RustCMS's own (`*`, or another
    // ability, console-made) is no API token here.
    let known = |a: &String| {
        SCOPES
            .iter()
            .any(|(_, permissions)| permissions.contains(&a.as_str()))
    };
    if abilities.is_empty() || !abilities.iter().all(known) {
        return Err(AppError::Forbidden);
    }
    let user = rustcms::models::user::Entity::find_by_id(token.user_id)
        .one(app.db())
        .await?
        .ok_or(AppError::Unauthenticated)?;
    let auth_state = app
        .service::<AuthState>()
        .ok_or_else(|| AppError::internal("AuthProvider not registered"))?;
    let mut principal = auth_state
        .resolver
        .principal(app.db(), ADMIN_GUARD, &user, Some(abilities.clone()))
        .await?;
    // Audited as the token it is.
    principal.credential = Some(rustcms::Credential::Token(token_id));
    Ok(ApiCaller {
        principal,
        abilities,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes() {
        assert!(abilities(&[]).is_err());
        assert!(abilities(&["everything".to_owned()]).is_err());
        let read = abilities(&["read".to_owned()]).unwrap();
        assert!(read.contains(&"services.view".to_owned()));
        assert!(!read.contains(&"services.deploy".to_owned()));
        assert!(!read.iter().any(|a| a == "*" || a.starts_with("system.")));
        let both = abilities(&["read".to_owned(), "deploy".to_owned()]).unwrap();
        let set: std::collections::HashSet<String> = both.into_iter().collect();
        assert_eq!(scopes_of(&set), ["read", "deploy"]);
    }
}
