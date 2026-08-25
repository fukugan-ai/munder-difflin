use std::env;
use std::fmt::{Display, Formatter};
use std::time::Duration;

use md_web_contracts::domains::persistence::Namespace;
use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgSslMode};
use tokio::time::timeout;

use super::{EXPECTED_SCHEMA_VERSION, PgPersistenceRepository};

const DEFAULT_PORT: u16 = 5432;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(4);
const QUERY_TIMEOUT: Duration = Duration::from_secs(5);
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_CONNECTIONS: u32 = 4;

/// Safe, non-secret startup failure for the process-lifetime repository owner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeOpenError {
    MissingConfiguration,
    InvalidConfiguration,
    Unavailable,
    SchemaMismatch,
}

impl Display for RuntimeOpenError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::MissingConfiguration => "PostgreSQL configuration is missing",
            Self::InvalidConfiguration => "PostgreSQL configuration is invalid",
            Self::Unavailable => "PostgreSQL persistence is unavailable",
            Self::SchemaMismatch => "PostgreSQL schema version is incompatible",
        })
    }
}

impl std::error::Error for RuntimeOpenError {}

/// Owns the one bounded PostgreSQL pool shared by server persistence adapters.
pub struct PgPersistenceRuntime {
    pool: PgPool,
    namespace: Namespace,
}

impl PgPersistenceRuntime {
    /// Opens the local/self-hosted PostgreSQL runtime from server-only `MD_PG_*`.
    /// Connection details are never included in returned errors.
    pub async fn from_environment() -> Result<Self, RuntimeOpenError> {
        let config = load_runtime_config(|key| env::var(key).ok())?;
        let options = PgConnectOptions::new_without_pgpass()
            .host(&config.host)
            .port(config.port)
            .database(&config.database)
            .username(&config.user)
            .password(&config.password)
            .ssl_mode(PgSslMode::Disable)
            .options([
                ("statement_timeout", "5000"),
                ("lock_timeout", "2000"),
                ("search_path", "pg_catalog"),
            ]);
        let pool = timeout(
            CONNECT_TIMEOUT,
            PgPoolOptions::new()
                .min_connections(0)
                .max_connections(MAX_CONNECTIONS)
                .acquire_timeout(CONNECT_TIMEOUT)
                .idle_timeout(Some(IDLE_TIMEOUT))
                .connect_with(options),
        )
        .await
        .map_err(|_| RuntimeOpenError::Unavailable)?
        .map_err(|_| RuntimeOpenError::Unavailable)?;
        let version = timeout(
            QUERY_TIMEOUT,
            sqlx::query_scalar::<_, i32>(
                "SELECT COALESCE(MAX(version),0)::int \
                 FROM munder_difflin.schema_migrations",
            )
            .fetch_one(&pool),
        )
        .await;
        let version = match version {
            Ok(Ok(version)) => version,
            Ok(Err(_)) | Err(_) => {
                pool.close().await;
                return Err(RuntimeOpenError::Unavailable);
            }
        };
        if version != EXPECTED_SCHEMA_VERSION {
            pool.close().await;
            return Err(RuntimeOpenError::SchemaMismatch);
        }
        Ok(Self {
            pool,
            namespace: config.namespace,
        })
    }

    /// Creates a cheap repository handle for one server adapter.
    #[must_use]
    pub fn repository(&self) -> PgPersistenceRepository {
        PgPersistenceRepository::new(self.pool.clone(), self.namespace.clone())
    }

    /// Returns the validated namespace shared by every repository handle.
    #[must_use]
    pub fn namespace(&self) -> &Namespace {
        &self.namespace
    }

    /// Stops acquisition and waits for checked-out connections during shutdown.
    pub async fn close(self) {
        self.pool.close().await;
    }
}

struct RuntimeConfig {
    host: String,
    port: u16,
    database: String,
    user: String,
    password: String,
    namespace: Namespace,
}

fn load_runtime_config(
    value: impl Fn(&str) -> Option<String>,
) -> Result<RuntimeConfig, RuntimeOpenError> {
    let host = value("MD_PG_HOST");
    let database = value("MD_PG_DATABASE");
    let user = value("MD_PG_USER");
    let password = value("MD_PG_PASSWORD");
    let namespace = value("MD_PG_NAMESPACE");
    let ca_path = value("MD_PG_TLS_CA");
    let (Some(host), Some(database), Some(user), Some(password), Some(namespace)) =
        (host, database, user, password, namespace)
    else {
        return Err(RuntimeOpenError::MissingConfiguration);
    };
    let host = host.trim();
    let database = database.trim();
    let user = user.trim();
    let namespace = Namespace::parse(namespace.trim().to_owned())
        .ok_or(RuntimeOpenError::InvalidConfiguration)?;
    let port = match value("MD_PG_PORT") {
        Some(raw) => raw
            .parse::<u16>()
            .ok()
            .filter(|port| *port > 0)
            .ok_or(RuntimeOpenError::InvalidConfiguration)?,
        None => DEFAULT_PORT,
    };
    let local = matches!(host, "localhost" | "127.0.0.1" | "::1");
    let has_ca = ca_path.is_some_and(|path| !path.trim().is_empty());
    if host.is_empty()
        || database.is_empty()
        || user.is_empty()
        || password.is_empty()
        || !local
        || has_ca
    {
        return Err(RuntimeOpenError::InvalidConfiguration);
    }
    Ok(RuntimeConfig {
        host: String::from(host),
        port,
        database: String::from(database),
        user: String::from(user),
        password,
        namespace,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{RuntimeOpenError, load_runtime_config};

    fn valid_values() -> BTreeMap<&'static str, String> {
        BTreeMap::from([
            ("MD_PG_HOST", String::from("127.0.0.1")),
            ("MD_PG_DATABASE", String::from("munder")),
            ("MD_PG_USER", String::from("munder")),
            ("MD_PG_PASSWORD", String::from("password")),
            ("MD_PG_NAMESPACE", String::from("local")),
        ])
    }

    #[test]
    fn missing_runtime_config_is_explicit() {
        assert!(matches!(
            load_runtime_config(|_| None),
            Err(RuntimeOpenError::MissingConfiguration)
        ));
    }

    #[test]
    fn remote_runtime_without_supported_tls_is_invalid() {
        let mut values = valid_values();
        values.insert("MD_PG_HOST", String::from("db.example.invalid"));
        assert!(matches!(
            load_runtime_config(|key| values.get(key).cloned()),
            Err(RuntimeOpenError::InvalidConfiguration)
        ));
    }

    #[test]
    fn invalid_namespace_is_rejected_before_connect() {
        let mut values = valid_values();
        values.insert("MD_PG_NAMESPACE", String::from("other/team"));
        assert!(matches!(
            load_runtime_config(|key| values.get(key).cloned()),
            Err(RuntimeOpenError::InvalidConfiguration)
        ));
    }

    #[test]
    fn runtime_error_display_contains_no_configuration_value() {
        assert_eq!(
            RuntimeOpenError::Unavailable.to_string(),
            "PostgreSQL persistence is unavailable"
        );
    }
}
