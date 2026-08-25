use md_web_contracts::domains::persistence::{
    AppConfigDocument, AppConfigWrite, DurableRecord, Namespace, RecordDomain, RecordKey,
    RecordWrite,
};
use sqlx::{PgPool, Row};

use super::RepositoryError;
use super::validation::{clamp_limit, validate_payload, validate_segment};

const APP_CONFIG_CAS_SQL: &str = "WITH updated AS (\
       UPDATE munder_difflin.web_app_config SET \
         revision=revision+1,payload=$3::jsonb,updated_at=now() \
       WHERE namespace=$1 AND revision=$2 AND $2>0 \
       RETURNING revision,payload::text AS payload_json,\
         (extract(epoch from updated_at)*1000)::bigint AS updated_at_ms\
     ), inserted AS (\
       INSERT INTO munder_difflin.web_app_config(namespace,revision,payload) \
       SELECT $1,1,$3::jsonb WHERE $2=0 \
       ON CONFLICT(namespace) DO NOTHING \
       RETURNING revision,payload::text AS payload_json,\
         (extract(epoch from updated_at)*1000)::bigint AS updated_at_ms\
     ) SELECT * FROM updated UNION ALL SELECT * FROM inserted LIMIT 1";

const RECORD_CAS_SQL: &str = "WITH updated AS (\
       UPDATE munder_difflin.web_durable_records SET \
         revision=revision+1,payload=$6::jsonb,updated_at=now() \
       WHERE namespace=$1 AND domain=$2 AND kind=$3 AND record_id=$4 \
         AND revision=$5 AND $5>0 \
       RETURNING domain,kind,record_id,revision,payload::text AS payload_json,\
         (extract(epoch from created_at)*1000)::bigint AS created_at_ms,\
         (extract(epoch from updated_at)*1000)::bigint AS updated_at_ms\
     ), inserted AS (\
       INSERT INTO munder_difflin.web_durable_records\
         (namespace,domain,kind,record_id,revision,payload) \
       SELECT $1,$2,$3,$4,1,$6::jsonb WHERE $5=0 \
       ON CONFLICT(namespace,domain,kind,record_id) DO NOTHING \
       RETURNING domain,kind,record_id,revision,payload::text AS payload_json,\
         (extract(epoch from created_at)*1000)::bigint AS created_at_ms,\
         (extract(epoch from updated_at)*1000)::bigint AS updated_at_ms\
     ) SELECT * FROM updated UNION ALL SELECT * FROM inserted LIMIT 1";

const NAMESPACE_RESET_SQL: &[&str] = &[
    "DELETE FROM munder_difflin.web_event_replay WHERE namespace=$1",
    "DELETE FROM munder_difflin.web_trigger_history WHERE namespace=$1",
    "DELETE FROM munder_difflin.web_event_stream_heads WHERE namespace=$1",
    "DELETE FROM munder_difflin.web_durable_records WHERE namespace=$1",
    "DELETE FROM munder_difflin.web_app_config WHERE namespace=$1",
    "DELETE FROM munder_difflin.cost_ledger WHERE namespace=$1",
    "DELETE FROM munder_difflin.command_history WHERE namespace=$1",
    "DELETE FROM munder_difflin.kv WHERE namespace=$1",
    "DELETE FROM munder_difflin.legacy_imports WHERE namespace=$1",
];

/// Namespace-scoped PostgreSQL repository shared by Web parity domains.
#[derive(Clone)]
pub struct PgPersistenceRepository {
    pub(super) pool: PgPool,
    pub(super) namespace: Namespace,
}

impl PgPersistenceRepository {
    /// Creates a repository from an already-configured pool and validated namespace.
    #[must_use]
    pub fn new(pool: PgPool, namespace: Namespace) -> Self {
        Self { pool, namespace }
    }

    /// Returns the namespace applied to every repository query.
    #[must_use]
    pub fn namespace(&self) -> &Namespace {
        &self.namespace
    }

    /// Deletes every durable row owned by this repository's bound namespace.
    ///
    /// Callers must stop namespace producers first. The repository reuses its
    /// process-lifetime pool and serializes the reset under one transaction-scoped
    /// advisory lock; no credential or namespace is accepted from the request.
    pub async fn reset_bound_namespace(&self) -> Result<u64, RepositoryError> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1),hashtext('namespace-reset'))")
            .bind(self.namespace.as_str())
            .execute(&mut *transaction)
            .await?;
        let mut deleted_rows = 0_u64;
        for statement in NAMESPACE_RESET_SQL {
            deleted_rows = deleted_rows.saturating_add(
                sqlx::query(*statement)
                    .bind(self.namespace.as_str())
                    .execute(&mut *transaction)
                    .await?
                    .rows_affected(),
            );
        }
        transaction.commit().await?;
        Ok(deleted_rows)
    }

    /// Loads the browser-safe application configuration, if it has been seeded.
    pub async fn load_app_config(&self) -> Result<Option<AppConfigDocument>, RepositoryError> {
        let row = sqlx::query(
            "SELECT revision,payload::text AS payload_json,\
             (extract(epoch from updated_at)*1000)::bigint AS updated_at_ms \
             FROM munder_difflin.web_app_config WHERE namespace=$1",
        )
        .bind(self.namespace.as_str())
        .fetch_optional(&self.pool)
        .await?;
        row.map(row_to_app_config).transpose()
    }

    /// Creates or compare-and-swap updates browser-safe configuration.
    pub async fn write_app_config(
        &self,
        request: &AppConfigWrite,
    ) -> Result<AppConfigDocument, RepositoryError> {
        if request.expected_revision < 0 {
            return Err(RepositoryError::InvalidInput(
                "configuration revision must be non-negative",
            ));
        }
        validate_payload(&request.payload_json)?;
        let row = sqlx::query(APP_CONFIG_CAS_SQL)
            .bind(self.namespace.as_str())
            .bind(request.expected_revision)
            .bind(&request.payload_json)
            .fetch_optional(&self.pool)
            .await?;
        row.map(row_to_app_config)
            .transpose()?
            .ok_or(RepositoryError::Conflict)
    }

    /// Loads one lossless domain record.
    pub async fn get_record(
        &self,
        key: &RecordKey,
    ) -> Result<Option<DurableRecord>, RepositoryError> {
        validate_key(key)?;
        let row = sqlx::query(
            "SELECT domain,kind,record_id,revision,payload::text AS payload_json,\
             (extract(epoch from created_at)*1000)::bigint AS created_at_ms,\
             (extract(epoch from updated_at)*1000)::bigint AS updated_at_ms \
             FROM munder_difflin.web_durable_records \
             WHERE namespace=$1 AND domain=$2 AND kind=$3 AND record_id=$4",
        )
        .bind(self.namespace.as_str())
        .bind(key.domain.as_str())
        .bind(&key.kind)
        .bind(&key.record_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(row_to_record).transpose()
    }

    /// Creates or compare-and-swap updates a lossless domain record.
    pub async fn write_record(
        &self,
        request: &RecordWrite,
    ) -> Result<DurableRecord, RepositoryError> {
        validate_key(&request.key)?;
        validate_payload(&request.payload_json)?;
        if request.expected_revision < 0 {
            return Err(RepositoryError::InvalidInput(
                "record revision must be non-negative",
            ));
        }
        let row = sqlx::query(RECORD_CAS_SQL)
            .bind(self.namespace.as_str())
            .bind(request.key.domain.as_str())
            .bind(&request.key.kind)
            .bind(&request.key.record_id)
            .bind(request.expected_revision)
            .bind(&request.payload_json)
            .fetch_optional(&self.pool)
            .await?;
        row.map(row_to_record)
            .transpose()?
            .ok_or(RepositoryError::Conflict)
    }

    /// Deletes one record only when the caller holds its current revision.
    pub async fn delete_record(
        &self,
        key: &RecordKey,
        expected_revision: i64,
    ) -> Result<bool, RepositoryError> {
        validate_key(key)?;
        if expected_revision < 1 {
            return Err(RepositoryError::InvalidInput(
                "record revision must be positive",
            ));
        }
        let result = sqlx::query(
            "DELETE FROM munder_difflin.web_durable_records \
             WHERE namespace=$1 AND domain=$2 AND kind=$3 AND record_id=$4 AND revision=$5",
        )
        .bind(self.namespace.as_str())
        .bind(key.domain.as_str())
        .bind(&key.kind)
        .bind(&key.record_id)
        .bind(expected_revision)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    /// Lists one domain/kind newest-first with a bounded result size.
    pub async fn list_records(
        &self,
        domain: RecordDomain,
        kind: &str,
        limit: u16,
    ) -> Result<Vec<DurableRecord>, RepositoryError> {
        validate_segment(kind, 64, "record kind is invalid")?;
        let rows = sqlx::query(
            "SELECT domain,kind,record_id,revision,payload::text AS payload_json,\
             (extract(epoch from created_at)*1000)::bigint AS created_at_ms,\
             (extract(epoch from updated_at)*1000)::bigint AS updated_at_ms \
             FROM munder_difflin.web_durable_records \
             WHERE namespace=$1 AND domain=$2 AND kind=$3 \
             ORDER BY updated_at DESC,record_id LIMIT $4",
        )
        .bind(self.namespace.as_str())
        .bind(domain.as_str())
        .bind(kind)
        .bind(clamp_limit(
            limit,
            md_web_contracts::domains::persistence::MAX_PAGE_LIMIT,
        ))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(row_to_record).collect()
    }
}

fn validate_key(key: &RecordKey) -> Result<(), RepositoryError> {
    validate_segment(&key.kind, 64, "record kind is invalid")?;
    validate_segment(&key.record_id, 256, "record id is invalid")
}

fn row_to_app_config(row: sqlx::postgres::PgRow) -> Result<AppConfigDocument, RepositoryError> {
    Ok(AppConfigDocument {
        revision: row.try_get("revision")?,
        payload_json: row.try_get("payload_json")?,
        updated_at_ms: row.try_get("updated_at_ms")?,
    })
}

fn row_to_record(row: sqlx::postgres::PgRow) -> Result<DurableRecord, RepositoryError> {
    let domain: String = row.try_get("domain")?;
    Ok(DurableRecord {
        key: RecordKey {
            domain: parse_domain(&domain)?,
            kind: row.try_get("kind")?,
            record_id: row.try_get("record_id")?,
        },
        revision: row.try_get("revision")?,
        payload_json: row.try_get("payload_json")?,
        created_at_ms: row.try_get("created_at_ms")?,
        updated_at_ms: row.try_get("updated_at_ms")?,
    })
}

fn parse_domain(value: &str) -> Result<RecordDomain, RepositoryError> {
    match value {
        "tasks" => Ok(RecordDomain::Tasks),
        "hive" => Ok(RecordDomain::Hive),
        "connections" => Ok(RecordDomain::Connections),
        "triggers" => Ok(RecordDomain::Triggers),
        "floors" => Ok(RecordDomain::Floors),
        _ => Err(RepositoryError::InvalidData(
            "unknown durable record domain",
        )),
    }
}

#[cfg(test)]
mod tests {
    use md_web_contracts::domains::persistence::{RecordDomain, RecordKey};

    use super::{
        APP_CONFIG_CAS_SQL, NAMESPACE_RESET_SQL, RECORD_CAS_SQL, parse_domain, validate_key,
    };

    #[test]
    fn key_rejects_empty_kind() {
        let key = RecordKey {
            domain: RecordDomain::Tasks,
            kind: String::new(),
            record_id: String::from("task-1"),
        };

        assert!(validate_key(&key).is_err());
    }

    #[test]
    fn key_accepts_maximum_record_id() {
        let key = RecordKey {
            domain: RecordDomain::Hive,
            kind: String::from("message"),
            record_id: "m".repeat(256),
        };

        assert!(validate_key(&key).is_ok());
    }

    #[test]
    fn domain_parser_rejects_unknown_database_value() {
        assert!(parse_domain("secret").is_err());
    }

    #[test]
    fn record_cas_updates_revision_one_to_two() {
        assert!(RECORD_CAS_SQL.contains("revision=revision+1"));
        assert!(RECORD_CAS_SQL.contains("revision=$5 AND $5>0"));
        assert!(RECORD_CAS_SQL.contains("SELECT $1,$2,$3,$4,1,$6::jsonb WHERE $5=0"));
    }

    #[test]
    fn namespace_reset_covers_every_owned_table_and_binds_namespace() {
        for table in [
            "web_event_replay",
            "web_trigger_history",
            "web_event_stream_heads",
            "web_durable_records",
            "web_app_config",
            "cost_ledger",
            "command_history",
            "kv",
            "legacy_imports",
        ] {
            assert!(NAMESPACE_RESET_SQL.iter().any(|sql| sql.contains(table)));
        }
        assert!(
            NAMESPACE_RESET_SQL
                .iter()
                .all(|sql| sql.contains("namespace=$1"))
        );
    }

    #[test]
    fn record_cas_returns_no_row_for_stale_revision() {
        assert!(RECORD_CAS_SQL.contains("SELECT * FROM updated UNION ALL SELECT * FROM inserted"));
        assert!(RECORD_CAS_SQL.contains("ON CONFLICT(namespace,domain,kind,record_id) DO NOTHING"));
    }

    #[test]
    fn app_config_uses_the_same_update_or_insert_contract() {
        assert!(APP_CONFIG_CAS_SQL.contains("revision=$2 AND $2>0"));
        assert!(APP_CONFIG_CAS_SQL.contains("SELECT $1,1,$3::jsonb WHERE $2=0"));
        assert!(APP_CONFIG_CAS_SQL.contains("ON CONFLICT(namespace) DO NOTHING"));
    }
}
