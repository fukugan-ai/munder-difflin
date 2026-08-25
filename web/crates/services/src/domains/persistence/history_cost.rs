use md_web_contracts::domains::memory_skills::{AgentCostTotal, CommandHistoryEntry, HistoryQuery};
use md_web_contracts::domains::persistence::{CostAppend, HistoryAppend};
use sqlx::Row;

use super::PgPersistenceRepository;
use super::RepositoryError;
use super::validation::{clamp_limit, escape_like, u64_to_i64, validate_segment};

impl PgPersistenceRepository {
    /// Appends one prompt idempotently to migration 001's command history.
    pub async fn append_history(&self, request: &HistoryAppend) -> Result<bool, RepositoryError> {
        validate_segment(&request.event_id, 64, "history event id is invalid")?;
        validate_segment(&request.agent_id, 256, "history agent id is invalid")?;
        let text = request.text.trim();
        if text.is_empty() {
            return Err(RepositoryError::InvalidInput("history text is required"));
        }
        let result = sqlx::query(
            "INSERT INTO munder_difflin.command_history\
               (namespace,event_id,agent_id,cwd,text,occurred_at) \
             VALUES($1,$2::uuid,$3,$4,$5,to_timestamp($6::double precision/1000.0)) \
             ON CONFLICT(namespace,event_id) DO NOTHING",
        )
        .bind(self.namespace.as_str())
        .bind(&request.event_id)
        .bind(&request.agent_id)
        .bind(request.cwd.as_deref())
        .bind(text)
        .bind(request.occurred_at_ms as f64)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    /// Lists or literal-substring searches command history newest-first.
    pub async fn query_history(
        &self,
        request: &HistoryQuery,
    ) -> Result<Vec<CommandHistoryEntry>, RepositoryError> {
        let limit = clamp_limit(
            request.limit,
            md_web_contracts::domains::persistence::MAX_PAGE_LIMIT,
        );
        if let Some(query) = request
            .query
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let pattern = format!("%{}%", escape_like(query));
            let rows = sqlx::query(
                "SELECT id::text AS id,agent_id,cwd,text,\
                 (extract(epoch from occurred_at)*1000)::bigint AS timestamp_ms \
                 FROM munder_difflin.command_history \
                 WHERE namespace=$1 AND text ILIKE $2 ESCAPE '\\' \
                 AND ($3::text IS NULL OR agent_id=$3) \
                 ORDER BY occurred_at DESC,id DESC LIMIT $4",
            )
            .bind(self.namespace.as_str())
            .bind(pattern)
            .bind(request.agent_id.as_deref())
            .bind(limit)
            .fetch_all(&self.pool)
            .await?;
            return rows.into_iter().map(row_to_history).collect();
        }
        let rows = sqlx::query(
            "SELECT id::text AS id,agent_id,cwd,text,\
             (extract(epoch from occurred_at)*1000)::bigint AS timestamp_ms \
             FROM munder_difflin.command_history \
             WHERE namespace=$1 AND ($2::text IS NULL OR agent_id=$2) \
             ORDER BY occurred_at DESC,id DESC LIMIT $3",
        )
        .bind(self.namespace.as_str())
        .bind(request.agent_id.as_deref())
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(row_to_history).collect()
    }

    /// Appends one cumulative cost snapshot idempotently to migration 001's ledger.
    pub async fn append_cost(&self, request: &CostAppend) -> Result<bool, RepositoryError> {
        validate_segment(&request.event_id, 64, "cost event id is invalid")?;
        validate_segment(&request.agent_id, 256, "cost agent id is invalid")?;
        validate_segment(&request.session_id, 256, "cost session id is invalid")?;
        if !request.usd.is_finite() || request.usd < 0.0 {
            return Err(RepositoryError::InvalidInput(
                "cost must be finite and non-negative",
            ));
        }
        let input = u64_to_i64(
            request.input_tokens,
            "input token count exceeds PostgreSQL range",
        )?;
        let output = u64_to_i64(
            request.output_tokens,
            "output token count exceeds PostgreSQL range",
        )?;
        let cache_read = u64_to_i64(
            request.cache_read_tokens,
            "cache read token count exceeds PostgreSQL range",
        )?;
        let cache_creation = u64_to_i64(
            request.cache_creation_tokens,
            "cache creation token count exceeds PostgreSQL range",
        )?;
        let result = sqlx::query(
            "INSERT INTO munder_difflin.cost_ledger\
               (namespace,event_id,agent_id,session_id,occurred_at,input_tokens,output_tokens,\
                cache_read_tokens,cache_creation_tokens,model,usd) \
             VALUES($1,$2::uuid,$3,$4,to_timestamp($5::double precision/1000.0),\
                    $6,$7,$8,$9,$10,$11::double precision::numeric) \
             ON CONFLICT(namespace,event_id) DO NOTHING",
        )
        .bind(self.namespace.as_str())
        .bind(&request.event_id)
        .bind(&request.agent_id)
        .bind(&request.session_id)
        .bind(request.occurred_at_ms as f64)
        .bind(input)
        .bind(output)
        .bind(cache_read)
        .bind(cache_creation)
        .bind(request.model.as_deref())
        .bind(request.usd)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    /// Reconstructs restart-safe lifetime spend in append order.
    pub async fn lifetime_cost_totals(&self) -> Result<Vec<AgentCostTotal>, RepositoryError> {
        let rows = sqlx::query(
            "WITH ordered AS (\
               SELECT id,agent_id,session_id,usd,lag(usd) OVER(\
                 PARTITION BY agent_id,session_id ORDER BY id) AS prior \
               FROM munder_difflin.cost_ledger WHERE namespace=$1\
             ), segmented AS (\
               SELECT *,sum(CASE WHEN prior IS NOT NULL AND usd<prior-0.000000001 THEN 1 ELSE 0 END) \
                 OVER(PARTITION BY agent_id,session_id ORDER BY id) AS segment FROM ordered\
             ), peaks AS (\
               SELECT agent_id,session_id,segment,max(usd) AS peak FROM segmented \
               GROUP BY agent_id,session_id,segment\
             ) SELECT agent_id,sum(peak)::double precision AS usd FROM peaks \
             GROUP BY agent_id ORDER BY agent_id",
        )
        .bind(self.namespace.as_str())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(AgentCostTotal {
                    agent_id: row.try_get("agent_id")?,
                    usd: row.try_get("usd")?,
                })
            })
            .collect::<Result<Vec<_>, sqlx::Error>>()
            .map_err(RepositoryError::Database)
    }
}

fn row_to_history(row: sqlx::postgres::PgRow) -> Result<CommandHistoryEntry, RepositoryError> {
    Ok(CommandHistoryEntry {
        id: row.try_get("id")?,
        agent_id: row.try_get("agent_id")?,
        cwd: row.try_get("cwd")?,
        text: row.try_get("text")?,
        timestamp_ms: row.try_get("timestamp_ms")?,
    })
}

#[cfg(test)]
mod tests {
    use super::super::validation::{escape_like, u64_to_i64};

    #[test]
    fn history_search_treats_wildcards_as_literals() {
        assert_eq!(escape_like(r"50%_done"), r"50\%\_done");
    }

    #[test]
    fn cost_tokens_reject_unsigned_overflow() {
        assert!(u64_to_i64(u64::MAX, "overflow").is_err());
    }
}
