use md_web_contracts::domains::persistence::{TriggerHistoryRecord, TriggerHistoryWrite};
use sqlx::Row;

use super::PgPersistenceRepository;
use super::RepositoryError;
use super::validation::{clamp_limit, validate_payload, validate_segment};

impl PgPersistenceRepository {
    /// Appends one trigger-history entry and prunes the namespace to its newest
    /// 500 rows in the same serialized transaction.
    pub async fn append_trigger_history(
        &self,
        request: &TriggerHistoryWrite,
    ) -> Result<bool, RepositoryError> {
        validate_segment(&request.event_id, 64, "trigger event id is invalid")?;
        validate_segment(&request.source, 32, "trigger source is invalid")?;
        validate_segment(&request.source_id, 256, "trigger source id is invalid")?;
        validate_payload(&request.payload_json)?;

        let mut transaction = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1),hashtext('trigger-history'))")
            .bind(self.namespace.as_str())
            .execute(&mut *transaction)
            .await?;
        let inserted = sqlx::query(
            "INSERT INTO munder_difflin.web_trigger_history\
               (namespace,event_id,source,source_id,occurred_at,payload) \
             VALUES($1,$2::uuid,$3,$4,to_timestamp($5::double precision/1000.0),$6::jsonb) \
             ON CONFLICT(namespace,event_id) DO NOTHING",
        )
        .bind(self.namespace.as_str())
        .bind(&request.event_id)
        .bind(&request.source)
        .bind(&request.source_id)
        .bind(request.occurred_at_ms as f64)
        .bind(&request.payload_json)
        .execute(&mut *transaction)
        .await?
        .rows_affected()
            == 1;
        sqlx::query(
            "DELETE FROM munder_difflin.web_trigger_history WHERE namespace=$1 AND event_id IN (\
               SELECT event_id FROM munder_difflin.web_trigger_history \
               WHERE namespace=$1 ORDER BY occurred_at DESC,event_id DESC OFFSET $2\
             )",
        )
        .bind(self.namespace.as_str())
        .bind(i64::from(
            md_web_contracts::domains::persistence::TRIGGER_HISTORY_RETENTION,
        ))
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(inserted)
    }

    /// Lists trigger history newest-first, optionally scoped to one source.
    pub async fn list_trigger_history(
        &self,
        source: Option<&str>,
        limit: u16,
    ) -> Result<Vec<TriggerHistoryRecord>, RepositoryError> {
        if let Some(source) = source {
            validate_segment(source, 32, "trigger source is invalid")?;
        }
        let rows = sqlx::query(
            "SELECT event_id::text AS event_id,source,source_id,\
             (extract(epoch from occurred_at)*1000)::bigint AS occurred_at_ms,\
             payload::text AS payload_json FROM munder_difflin.web_trigger_history \
             WHERE namespace=$1 AND ($2::text IS NULL OR source=$2) \
             ORDER BY occurred_at DESC,event_id DESC LIMIT $3",
        )
        .bind(self.namespace.as_str())
        .bind(source)
        .bind(clamp_limit(
            limit,
            md_web_contracts::domains::persistence::TRIGGER_HISTORY_RETENTION,
        ))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(row_to_trigger_history).collect()
    }

    /// Clears trigger history for the namespace or one source only.
    pub async fn clear_trigger_history(
        &self,
        source: Option<&str>,
    ) -> Result<u64, RepositoryError> {
        if let Some(source) = source {
            validate_segment(source, 32, "trigger source is invalid")?;
        }
        let mut transaction = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1),hashtext('trigger-history'))")
            .bind(self.namespace.as_str())
            .execute(&mut *transaction)
            .await?;
        let result = sqlx::query(
            "DELETE FROM munder_difflin.web_trigger_history \
             WHERE namespace=$1 AND ($2::text IS NULL OR source=$2)",
        )
        .bind(self.namespace.as_str())
        .bind(source)
        .execute(&mut *transaction)
        .await?;
        let deleted = result.rows_affected();
        transaction.commit().await?;
        Ok(deleted)
    }
}

fn row_to_trigger_history(
    row: sqlx::postgres::PgRow,
) -> Result<TriggerHistoryRecord, RepositoryError> {
    Ok(TriggerHistoryRecord {
        event_id: row.try_get("event_id")?,
        source: row.try_get("source")?,
        source_id: row.try_get("source_id")?,
        occurred_at_ms: row.try_get("occurred_at_ms")?,
        payload_json: row.try_get("payload_json")?,
    })
}

#[cfg(test)]
mod tests {
    use md_web_contracts::domains::persistence::TRIGGER_HISTORY_RETENTION;

    #[test]
    fn retention_matches_browser_contract() {
        assert_eq!(TRIGGER_HISTORY_RETENTION, 500);
    }
}
