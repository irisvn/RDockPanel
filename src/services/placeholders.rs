//! Placeholders in a service's environment, as EasyPanel's templates write
//! them: `$(PROJECT_NAME)`, `$(SERVICE_NAME)`, `$(PRIMARY_DOMAIN)`,
//! `$(EASYPANEL_DOMAIN)` (the service's generated domain),
//! `$(EASYPANEL_HOST)` (the domain generated ones live under) and
//! `$(EASYPANEL_HOST_IP)`. Replaced at each deployment; an unknown name, or
//! one without a value, is left as written. Pure functions.

/// What placeholders stand for, for one service.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Vars {
    pub project: String,
    pub service: String,
    /// The host of its primary (first) domain.
    pub primary_domain: Option<String>,
    /// `[panel] service_domain`: generated domains are `<project>-<service>.<this>`.
    pub host: Option<String>,
    /// `[panel] public_ip`.
    pub host_ip: Option<String>,
}

impl Vars {
    /// The service's generated domain, when `[panel] service_domain` is set.
    pub fn auto_domain(&self) -> Option<String> {
        auto_domain(&self.project, &self.service, self.host.as_deref()?)
    }

    fn value(&self, name: &str) -> Option<String> {
        match name {
            "PROJECT_NAME" => Some(self.project.clone()),
            "SERVICE_NAME" => Some(self.service.clone()),
            "PRIMARY_DOMAIN" => self.primary_domain.clone().or_else(|| self.auto_domain()),
            "EASYPANEL_DOMAIN" => self.auto_domain(),
            "EASYPANEL_HOST" => self.host.clone(),
            "EASYPANEL_HOST_IP" => self.host_ip.clone(),
            _ => None,
        }
    }
}

/// `<project>-<service>.<host>`. When the project's name has a dash (`a-b`
/// with `c`, and `a` with `b-c`, would both be `a-b-c`) or the label would
/// pass 63 characters, a short hash of both names keeps it unique and short.
pub fn auto_domain(project: &str, service: &str, host: &str) -> Option<String> {
    use sha2::{Digest, Sha256};
    let host = host.trim().trim_matches('.');
    if host.is_empty() {
        return None;
    }
    let mut label = format!("{project}-{service}");
    if project.contains('-') || label.len() > 63 {
        let hash = Sha256::digest(format!("{project}\0{service}").as_bytes());
        let suffix: String = hash.iter().take(3).map(|b| format!("{b:02x}")).collect();
        label.truncate(55);
        let label = label.trim_end_matches('-');
        return Some(format!("{label}-{suffix}.{host}"));
    }
    Some(format!("{label}.{host}"))
}

/// `text` with its placeholders replaced.
pub fn expand(text: &str, vars: &Vars) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("$(") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match after.find(')') {
            Some(end) => {
                let name = &after[..end];
                match vars.value(name) {
                    Some(value) => out.push_str(&value),
                    None => out.push_str(&rest[start..start + 2 + end + 1]),
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars() -> Vars {
        Vars {
            project: "shop".into(),
            service: "web".into(),
            primary_domain: Some("shop.example.com".into()),
            host: Some("apps.example.com".into()),
            host_ip: Some("203.0.113.7".into()),
        }
    }

    #[test]
    fn names_are_replaced() {
        let v = vars();
        assert_eq!(expand("$(PROJECT_NAME)_db", &v), "shop_db");
        assert_eq!(
            expand(
                "https://$(PRIMARY_DOMAIN)/ $(EASYPANEL_DOMAIN) $(EASYPANEL_HOST) $(EASYPANEL_HOST_IP) $(SERVICE_NAME)",
                &v
            ),
            "https://shop.example.com/ shop-web.apps.example.com apps.example.com 203.0.113.7 web"
        );
        assert_eq!(
            expand("https://$(PROJECT_NAME)-api.$(EASYPANEL_HOST)", &v),
            "https://shop-api.apps.example.com"
        );
    }

    #[test]
    fn unknown_or_unset_names_stay() {
        let v = Vars {
            project: "shop".into(),
            service: "web".into(),
            ..Vars::default()
        };
        assert_eq!(
            expand("$(NOPE) $(EASYPANEL_DOMAIN) $(unclosed", &v),
            "$(NOPE) $(EASYPANEL_DOMAIN) $(unclosed"
        );
        // Without a domain, the primary one is the generated one (none here).
        assert_eq!(expand("$(PRIMARY_DOMAIN)", &v), "$(PRIMARY_DOMAIN)");
        assert_eq!(auto_domain("shop", "web", " "), None);
        assert_eq!(
            auto_domain("shop", "web", "localhost"),
            Some("shop-web.localhost".into())
        );
        // Never the same for two services; never a label over 63.
        let a = auto_domain("a-b", "c", "h").unwrap();
        let b = auto_domain("a", "b-c", "h").unwrap();
        assert_ne!(a, b);
        let long = auto_domain(&"p".repeat(40), &"s".repeat(40), "h").unwrap();
        assert!(long.split('.').next().unwrap().len() <= 63, "{long}");
        assert!(
            crate::services::deploy_rules_service::valid_hostname(&long),
            "{long}"
        );
    }
}
