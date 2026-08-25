use std::env;
use std::fmt::{Display, Formatter};
use std::future::Future;
use std::time::Duration;

use md_web_contracts::{PersistenceCode, PersistenceStatus};
use sqlx::postgres::{PgConnectOptions, PgSslMode};
use sqlx::{Connection, PgConnection};
use tokio::time::timeout;

const DEFAULT_PORT: u16 = 5432;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(4);
const QUERY_TIMEOUT: Duration = Duration::from_secs(5);
const EXPECTED_SCHEMA_VERSION: i32 = 2;
const SCHEMA_QUERY: &str =
    "SELECT COALESCE(MAX(version), 0)::int FROM munder_difflin.schema_migrations";

/// Concrete failure from a bounded persistence connector.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceError {
    ConnectTimeout,
    ConnectFailed,
    QueryTimeout,
    QueryFailed,
    InvalidResponse,
}

impl Display for ServiceError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::ConnectTimeout => "PostgreSQL connection timed out",
            Self::ConnectFailed => "PostgreSQL connection failed",
            Self::QueryTimeout => "PostgreSQL health query timed out",
            Self::QueryFailed => "PostgreSQL health query failed",
            Self::InvalidResponse => "PostgreSQL health query returned an invalid response",
        })
    }
}

impl std::error::Error for ServiceError {}

struct PgConfig {
    host: String,
    port: u16,
    database: String,
    user: String,
    password: String,
}

enum ConfigState {
    Missing,
    Invalid,
    Configured(PgConfig),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProbeResult {
    Ready,
    SchemaMismatch,
}

trait PersistenceConnector {
    fn probe(
        &self,
        config: &PgConfig,
    ) -> impl Future<Output = Result<ProbeResult, ServiceError>> + Send;
}

struct PostgresConnector;

impl PersistenceConnector for PostgresConnector {
    async fn probe(&self, config: &PgConfig) -> Result<ProbeResult, ServiceError> {
        let postgres = PgConnectOptions::new_without_pgpass()
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

        let mut connection = timeout(CONNECT_TIMEOUT, PgConnection::connect_with(&postgres))
            .await
            .map_err(|_| ServiceError::ConnectTimeout)?
            .map_err(|_| ServiceError::ConnectFailed)?;
        let query_result = timeout(
            QUERY_TIMEOUT,
            sqlx::query_scalar::<_, i32>(SCHEMA_QUERY).fetch_one(&mut connection),
        )
        .await
        .map_err(|_| ServiceError::QueryTimeout)?
        .map_err(|error| {
            if is_schema_missing(&error) {
                ServiceError::InvalidResponse
            } else {
                ServiceError::QueryFailed
            }
        })?;
        if query_result == EXPECTED_SCHEMA_VERSION {
            Ok(ProbeResult::Ready)
        } else {
            Ok(ProbeResult::SchemaMismatch)
        }
    }
}

fn is_schema_missing(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::code)
        .is_some_and(|code| matches!(code.as_ref(), "42P01" | "3F000"))
}

pub(crate) async fn probe_from_environment() -> PersistenceStatus {
    probe_with(&PostgresConnector, |key| env::var(key).ok()).await
}

async fn probe_with(
    connector: &impl PersistenceConnector,
    value: impl Fn(&str) -> Option<String>,
) -> PersistenceStatus {
    let config = match load_config(value) {
        ConfigState::Missing => return degraded(PersistenceCode::MissingConfig),
        ConfigState::Invalid => return degraded(PersistenceCode::ConfigInvalid),
        ConfigState::Configured(config) => config,
    };

    match connector.probe(&config).await {
        Ok(ProbeResult::Ready) => PersistenceStatus::Ready { writes: true },
        Ok(ProbeResult::SchemaMismatch) | Err(ServiceError::InvalidResponse) => {
            degraded(PersistenceCode::SchemaMismatch)
        }
        Err(
            ServiceError::ConnectTimeout
            | ServiceError::ConnectFailed
            | ServiceError::QueryTimeout
            | ServiceError::QueryFailed,
        ) => degraded(PersistenceCode::Unreachable),
    }
}

fn load_config(value: impl Fn(&str) -> Option<String>) -> ConfigState {
    let host = value("MD_PG_HOST");
    let database = value("MD_PG_DATABASE");
    let user = value("MD_PG_USER");
    let password = value("MD_PG_PASSWORD");
    let namespace = value("MD_PG_NAMESPACE");
    let ca_path = value("MD_PG_TLS_CA");

    let (Some(host), Some(database), Some(user), Some(password), Some(namespace)) =
        (host, database, user, password, namespace)
    else {
        return ConfigState::Missing;
    };
    let host = host.trim();
    let database = database.trim();
    let user = user.trim();
    let namespace = namespace.trim();
    let port = match value("MD_PG_PORT") {
        Some(raw) => match raw.parse::<u16>() {
            Ok(port) if port > 0 => port,
            _ => return ConfigState::Invalid,
        },
        None => DEFAULT_PORT,
    };
    let namespace_valid = !namespace.is_empty()
        && namespace.len() <= 128
        && namespace
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'));
    if host.is_empty()
        || database.is_empty()
        || user.is_empty()
        || password.is_empty()
        || !namespace_valid
    {
        return ConfigState::Invalid;
    }

    let local = matches!(host, "localhost" | "127.0.0.1" | "::1");
    let has_ca = ca_path.is_some_and(|path| !path.trim().is_empty());
    if !local || has_ca {
        // The offline dependency set has no TLS connector. Never report Ready without TLS.
        return ConfigState::Invalid;
    }

    ConfigState::Configured(PgConfig {
        host: String::from(host),
        port,
        database: String::from(database),
        user: String::from(user),
        password,
    })
}

const fn degraded(code: PersistenceCode) -> PersistenceStatus {
    PersistenceStatus::Degraded { code }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use md_web_contracts::{PersistenceCode, PersistenceStatus};

    use super::{
        ConfigState, PersistenceConnector, PgConfig, ProbeResult, ServiceError, load_config,
        probe_with,
    };

    struct MockConnector(Result<ProbeResult, ServiceError>);

    impl PersistenceConnector for MockConnector {
        async fn probe(&self, _config: &PgConfig) -> Result<ProbeResult, ServiceError> {
            self.0
        }
    }

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
    fn empty_configuration_is_missing() {
        assert!(matches!(load_config(|_| None), ConfigState::Missing));
    }

    #[test]
    fn maximum_port_is_valid() {
        let mut values = valid_values();
        values.insert("MD_PG_PORT", u16::MAX.to_string());

        assert!(matches!(
            load_config(|key| values.get(key).cloned()),
            ConfigState::Configured(_)
        ));
    }

    #[test]
    fn zero_port_is_invalid() {
        let mut values = valid_values();
        values.insert("MD_PG_PORT", String::from("0"));

        assert!(matches!(
            load_config(|key| values.get(key).cloned()),
            ConfigState::Invalid
        ));
    }

    #[test]
    fn maximum_namespace_is_valid() {
        let mut values = valid_values();
        values.insert("MD_PG_NAMESPACE", "a".repeat(128));

        assert!(matches!(
            load_config(|key| values.get(key).cloned()),
            ConfigState::Configured(_)
        ));
    }

    #[test]
    fn oversized_namespace_is_invalid() {
        let mut values = valid_values();
        values.insert("MD_PG_NAMESPACE", "a".repeat(129));

        assert!(matches!(
            load_config(|key| values.get(key).cloned()),
            ConfigState::Invalid
        ));
    }

    #[tokio::test]
    async fn mock_ready_maps_to_ready() {
        let values = valid_values();

        assert_eq!(
            probe_with(&MockConnector(Ok(ProbeResult::Ready)), |key| values
                .get(key)
                .cloned())
            .await,
            PersistenceStatus::Ready { writes: true }
        );
    }

    #[tokio::test]
    async fn mock_unreachable_maps_to_unreachable() {
        let values = valid_values();

        assert_eq!(
            probe_with(&MockConnector(Err(ServiceError::ConnectFailed)), |key| {
                values.get(key).cloned()
            })
            .await,
            PersistenceStatus::Degraded {
                code: PersistenceCode::Unreachable
            }
        );
    }

    #[test]
    fn service_error_has_stable_display() {
        assert_eq!(
            ServiceError::ConnectTimeout.to_string(),
            "PostgreSQL connection timed out"
        );
    }
}
