use md_web_contracts::domains::persistence::{ReplayEvent, ReplayEventWrite, ReplayPage};
use sqlx::Row;

use super::PgPersistenceRepository;
use super::RepositoryError;
use super::validation::{clamp_limit, u64_to_i64, validate_payload, validate_segment};

impl PgPersistenceRepository {
    /// Appends one event with a namespace-local, gap-free stream sequence.
    /// Holding the stream-head row lock before the idempotency lookup prevents
    /// concurrent retries from consuming a second sequence.
    pub async fn append_replay_event(
        &self,
        request: &ReplayEventWrite,
    ) -> Result<ReplayEvent, RepositoryError> {
        validate_segment(&request.stream, 64, "event stream is invalid")?;
        validate_segment(&request.event_id, 64, "event id is invalid")?;
        validate_payload(&request.payload_json)?;

        let mut transaction = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO munder_difflin.web_event_stream_heads(namespace,stream,last_sequence) \
             VALUES($1,$2,0) ON CONFLICT(namespace,stream) DO NOTHING",
        )
        .bind(self.namespace.as_str())
        .bind(&request.stream)
        .execute(&mut *transaction)
        .await?;
        let head: i64 = sqlx::query_scalar(
            "SELECT last_sequence FROM munder_difflin.web_event_stream_heads \
             WHERE namespace=$1 AND stream=$2 FOR UPDATE",
        )
        .bind(self.namespace.as_str())
        .bind(&request.stream)
        .fetch_one(&mut *transaction)
        .await?;

        let existing = sqlx::query(
            "SELECT stream,sequence,event_id::text AS event_id,\
             (extract(epoch from occurred_at)*1000)::bigint AS occurred_at_ms,\
             payload::text AS payload_json FROM munder_difflin.web_event_replay \
             WHERE namespace=$1 AND stream=$2 AND event_id=$3::uuid",
        )
        .bind(self.namespace.as_str())
        .bind(&request.stream)
        .bind(&request.event_id)
        .fetch_optional(&mut *transaction)
        .await?;
        if let Some(row) = existing {
            let event = row_to_event(row)?;
            transaction.commit().await?;
            return Ok(event);
        }

        let sequence = head
            .checked_add(1)
            .ok_or(RepositoryError::SequenceExhausted)?;
        let row = sqlx::query(
            "INSERT INTO munder_difflin.web_event_replay\
               (namespace,stream,sequence,event_id,occurred_at,payload) \
             VALUES($1,$2,$3,$4::uuid,to_timestamp($5::double precision/1000.0),$6::jsonb) \
             RETURNING stream,sequence,event_id::text AS event_id,\
               (extract(epoch from occurred_at)*1000)::bigint AS occurred_at_ms,\
               payload::text AS payload_json",
        )
        .bind(self.namespace.as_str())
        .bind(&request.stream)
        .bind(sequence)
        .bind(&request.event_id)
        .bind(request.occurred_at_ms as f64)
        .bind(&request.payload_json)
        .fetch_one(&mut *transaction)
        .await?;
        sqlx::query(
            "UPDATE munder_difflin.web_event_stream_heads \
             SET last_sequence=$3,updated_at=now() WHERE namespace=$1 AND stream=$2",
        )
        .bind(self.namespace.as_str())
        .bind(&request.stream)
        .bind(sequence)
        .execute(&mut *transaction)
        .await?;
        let event = row_to_event(row)?;
        transaction.commit().await?;
        Ok(event)
    }

    /// Returns events newer than `after`. `gap` is true when retention removed
    /// at least one sequence the caller has not seen.
    pub async fn replay_after(
        &self,
        stream: &str,
        after: u64,
        limit: u16,
    ) -> Result<ReplayPage, RepositoryError> {
        validate_segment(stream, 64, "event stream is invalid")?;
        let after = u64_to_i64(after, "event cursor exceeds PostgreSQL range")?;
        let retention = sqlx::query(
            "SELECT h.last_sequence,min(e.sequence) AS oldest_sequence \
             FROM munder_difflin.web_event_stream_heads h \
             LEFT JOIN munder_difflin.web_event_replay e \
               ON e.namespace=h.namespace AND e.stream=h.stream \
             WHERE h.namespace=$1 AND h.stream=$2 GROUP BY h.last_sequence",
        )
        .bind(self.namespace.as_str())
        .bind(stream)
        .fetch_optional(&self.pool)
        .await?;
        let rows = sqlx::query(
            "SELECT stream,sequence,event_id::text AS event_id,\
             (extract(epoch from occurred_at)*1000)::bigint AS occurred_at_ms,\
             payload::text AS payload_json FROM munder_difflin.web_event_replay \
             WHERE namespace=$1 AND stream=$2 AND sequence>$3 \
             ORDER BY sequence LIMIT $4",
        )
        .bind(self.namespace.as_str())
        .bind(stream)
        .bind(after)
        .bind(clamp_limit(
            limit,
            md_web_contracts::domains::persistence::MAX_PAGE_LIMIT,
        ))
        .fetch_all(&self.pool)
        .await?;
        let (last_sequence, oldest) = match retention {
            Some(row) => (
                row.try_get("last_sequence")?,
                row.try_get("oldest_sequence")?,
            ),
            None => (0, None),
        };
        let gap = replay_has_gap(oldest, last_sequence, after);
        let events = rows
            .into_iter()
            .map(row_to_event)
            .collect::<Result<Vec<_>, RepositoryError>>()?;
        Ok(ReplayPage { gap, events })
    }
}

fn replay_has_gap(oldest: Option<i64>, last_sequence: i64, after: i64) -> bool {
    match oldest {
        Some(sequence) => after
            .checked_add(1)
            .is_some_and(|expected| sequence > expected),
        None => last_sequence > after,
    }
}

fn row_to_event(row: sqlx::postgres::PgRow) -> Result<ReplayEvent, RepositoryError> {
    let sequence: i64 = row.try_get("sequence")?;
    Ok(ReplayEvent {
        stream: row.try_get("stream")?,
        sequence: u64::try_from(sequence)
            .map_err(|_| RepositoryError::InvalidData("negative event sequence"))?,
        event_id: row.try_get("event_id")?,
        occurred_at_ms: row.try_get("occurred_at_ms")?,
        payload_json: row.try_get("payload_json")?,
    })
}

#[cfg(test)]
mod tests {
    use super::replay_has_gap;

    #[test]
    fn replay_reports_retention_gap() {
        assert!(replay_has_gap(Some(5), 10, 2));
    }

    #[test]
    fn replay_reports_gap_when_all_old_events_were_removed() {
        assert!(replay_has_gap(None, 10, 2));
    }

    #[test]
    fn replay_on_new_stream_has_no_gap() {
        assert!(!replay_has_gap(None, 0, 10));
    }
}
