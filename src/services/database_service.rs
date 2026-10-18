//! Database services: what each engine needs to run on Swarm (image, data
//! volume, environment holding its credentials, health check) and how apps
//! reach it. A database is an app service the panel configures: it deploys,
//! rolls back and logs like any other. Pure functions (passwords aside).

use std::collections::BTreeMap;

use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};

/// A database engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    Postgres,
    Mysql,
    Mariadb,
    Mongo,
    Redis,
}

/// Who connects, with what, to which database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credentials {
    pub user: String,
    pub password: String,
    pub database: String,
    /// MySQL and MariaDB's root.
    pub root_password: Option<String>,
}

/// What a database service is made of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Setup {
    pub image: String,
    pub env: BTreeMap<String, String>,
    pub command: Option<String>,
    pub healthcheck: String,
    /// The data volume's path in the container (volume `data`).
    pub data_path: &'static str,
}

impl Engine {
    pub const ALL: [Engine; 5] = [
        Engine::Postgres,
        Engine::Mysql,
        Engine::Mariadb,
        Engine::Mongo,
        Engine::Redis,
    ];

    pub fn parse(kind: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|e| e.kind() == kind)
    }

    /// `services.kind`.
    pub fn kind(self) -> &'static str {
        match self {
            Self::Postgres => "postgres",
            Self::Mysql => "mysql",
            Self::Mariadb => "mariadb",
            Self::Mongo => "mongo",
            Self::Redis => "redis",
        }
    }

    pub fn default_image(self) -> &'static str {
        match self {
            Self::Postgres => "postgres:17",
            Self::Mysql => "mysql:9",
            Self::Mariadb => "mariadb:11",
            Self::Mongo => "mongo:8",
            Self::Redis => "redis:7",
        }
    }

    pub fn port(self) -> u16 {
        match self {
            Self::Postgres => 5432,
            Self::Mysql | Self::Mariadb => 3306,
            Self::Mongo => 27017,
            Self::Redis => 6379,
        }
    }

    /// The user when none is given (Redis has only `default`).
    pub fn default_user(self) -> &'static str {
        match self {
            Self::Postgres => "postgres",
            Self::Mysql => "mysql",
            Self::Mariadb => "mariadb",
            Self::Mongo => "mongo",
            Self::Redis => "default",
        }
    }

    pub fn has_root_password(self) -> bool {
        matches!(self, Self::Mysql | Self::Mariadb)
    }

    /// The service's image, environment, command and health check.
    pub fn setup(self, image: Option<&str>, credentials: &Credentials) -> Setup {
        let image = image
            .map(str::trim)
            .filter(|i| !i.is_empty())
            .unwrap_or(self.default_image())
            .to_owned();
        let c = credentials;
        let root = c.root_password.clone().unwrap_or_default();
        let pairs: Vec<(&str, String)> = match self {
            Self::Postgres => vec![
                ("POSTGRES_USER", c.user.clone()),
                ("POSTGRES_PASSWORD", c.password.clone()),
                ("POSTGRES_DB", c.database.clone()),
            ],
            Self::Mysql => vec![
                ("MYSQL_ROOT_PASSWORD", root),
                ("MYSQL_DATABASE", c.database.clone()),
                ("MYSQL_USER", c.user.clone()),
                ("MYSQL_PASSWORD", c.password.clone()),
            ],
            Self::Mariadb => vec![
                ("MARIADB_ROOT_PASSWORD", root),
                ("MARIADB_DATABASE", c.database.clone()),
                ("MARIADB_USER", c.user.clone()),
                ("MARIADB_PASSWORD", c.password.clone()),
            ],
            Self::Mongo => vec![
                ("MONGO_INITDB_ROOT_USERNAME", c.user.clone()),
                ("MONGO_INITDB_ROOT_PASSWORD", c.password.clone()),
            ],
            Self::Redis => vec![("REDIS_PASSWORD", c.password.clone())],
        };
        let env = pairs.into_iter().map(|(k, v)| (k.to_owned(), v)).collect();
        let (healthcheck, data_path, command) = match self {
            Self::Postgres => (
                r#"pg_isready -U "$POSTGRES_USER" -d "$POSTGRES_DB""#,
                "/var/lib/postgresql/data",
                None,
            ),
            Self::Mysql => (
                r#"mysqladmin ping -h 127.0.0.1 -u root -p"$MYSQL_ROOT_PASSWORD" --silent"#,
                "/var/lib/mysql",
                None,
            ),
            Self::Mariadb => (
                "healthcheck.sh --connect --innodb_initialized",
                "/var/lib/mysql",
                None,
            ),
            Self::Mongo => (
                r#"mongosh --quiet --eval "db.adminCommand('ping').ok" | grep -q 1"#,
                "/data/db",
                None,
            ),
            Self::Redis => (
                r#"redis-cli -a "$REDIS_PASSWORD" --no-auth-warning ping | grep -q PONG"#,
                "/data",
                // The password from the environment, never in the arguments.
                Some(
                    r#"sh -c 'exec redis-server --requirepass "$REDIS_PASSWORD" --appendonly yes'"#,
                ),
            ),
        };
        Setup {
            image,
            env,
            command: command.map(str::to_owned),
            healthcheck: healthcheck.to_owned(),
            data_path,
        }
    }

    /// The credentials back from a service's environment (as `setup` wrote
    /// it): what the Credentials tab and templates read.
    pub fn credentials_of(self, env: &BTreeMap<String, String>) -> Credentials {
        let get = |k: &str| env.get(k).cloned().unwrap_or_default();
        match self {
            Self::Postgres => Credentials {
                user: get("POSTGRES_USER"),
                password: get("POSTGRES_PASSWORD"),
                database: get("POSTGRES_DB"),
                root_password: None,
            },
            Self::Mysql => Credentials {
                user: get("MYSQL_USER"),
                password: get("MYSQL_PASSWORD"),
                database: get("MYSQL_DATABASE"),
                root_password: Some(get("MYSQL_ROOT_PASSWORD")),
            },
            Self::Mariadb => Credentials {
                user: get("MARIADB_USER"),
                password: get("MARIADB_PASSWORD"),
                database: get("MARIADB_DATABASE"),
                root_password: Some(get("MARIADB_ROOT_PASSWORD")),
            },
            Self::Mongo => Credentials {
                user: get("MONGO_INITDB_ROOT_USERNAME"),
                password: get("MONGO_INITDB_ROOT_PASSWORD"),
                database: String::new(),
                root_password: None,
            },
            Self::Redis => Credentials {
                user: "default".to_owned(),
                password: get("REDIS_PASSWORD"),
                database: String::new(),
                root_password: None,
            },
        }
    }

    /// How an app of the same project connects (`host`: `<project>_<service>`).
    pub fn url(self, host: &str, c: &Credentials) -> String {
        let user = utf8_percent_encode(&c.user, NON_ALPHANUMERIC);
        let password = utf8_percent_encode(&c.password, NON_ALPHANUMERIC);
        let port = self.port();
        let database = utf8_percent_encode(&c.database, NON_ALPHANUMERIC);
        match self {
            Self::Postgres => {
                format!("postgres://{user}:{password}@{host}:{port}/{database}?sslmode=disable")
            }
            Self::Mysql | Self::Mariadb => {
                format!("mysql://{user}:{password}@{host}:{port}/{database}")
            }
            Self::Mongo => format!("mongodb://{user}:{password}@{host}:{port}/?authSource=admin"),
            Self::Redis => format!("redis://default:{password}@{host}:{port}"),
        }
    }
}

/// A random password: 24 letters and digits (about 143 bits).
pub fn random_password() -> String {
    random_string(24)
}

/// `len` random lowercase letters, uppercase letters and digits.
pub fn random_string(len: usize) -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let mut out = String::with_capacity(len);
    while out.len() < len {
        let mut bytes = [0u8; 64];
        getrandom::fill(&mut bytes).expect("the system's random source");
        for b in bytes {
            // Unbiased: 248 = 4 × 62.
            if b < 248 && out.len() < len {
                out.push(ALPHABET[usize::from(b) % ALPHABET.len()] as char);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn creds() -> Credentials {
        Credentials {
            user: "app".into(),
            password: "p@ss word".into(),
            database: "shop".into(),
            root_password: Some("root".into()),
        }
    }

    #[test]
    fn each_engine_runs_and_connects() {
        for engine in Engine::ALL {
            let setup = engine.setup(None, &creds());
            assert_eq!(setup.image, engine.default_image());
            assert!(setup.data_path.starts_with('/'));
            // The credentials come back from the environment.
            let back = engine.credentials_of(&setup.env);
            assert_eq!(back.password, "p@ss word", "{engine:?}");
            // Passwords never in the command's arguments.
            assert!(!setup.command.unwrap_or_default().contains("p@ss"));
            assert_eq!(Engine::parse(engine.kind()), Some(engine));
        }
        assert_eq!(
            Engine::Postgres.url("shop_db", &creds()),
            "postgres://app:p%40ss%20word@shop_db:5432/shop?sslmode=disable"
        );
        assert_eq!(
            Engine::Redis.url("shop_cache", &creds()),
            "redis://default:p%40ss%20word@shop_cache:6379"
        );
        assert_eq!(
            Engine::Mysql.setup(Some("mysql:8.4"), &creds()).image,
            "mysql:8.4"
        );
        let odd = Credentials {
            database: "sales#archive".into(),
            ..creds()
        };
        assert!(Engine::Mysql.url("h", &odd).ends_with("/sales%23archive"));
    }

    #[test]
    fn passwords_are_random() {
        let a = random_password();
        assert_eq!(a.len(), 24);
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric()));
        assert_ne!(a, random_password());
    }
}
