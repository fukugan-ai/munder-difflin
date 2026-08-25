use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use std::{env, time::Duration};

use md_web_contracts::domains::memory_skills::{
    AgentCostTotal, AgentUsageSample, CommandHistoryEntry, HistoryQuery,
};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgSslMode};
use sqlx::{PgPool, Row};

use super::DomainError;

const MAX_HISTORY_LIMIT: u16 = 1_000;
static EVENT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub struct HistoryRepository {
    pool: PgPool,
    namespace: String,
}

impl HistoryRepository {
    pub async fn connect_from_environment() -> Result<Self, DomainError> {
        let host = required_env("MD_PG_HOST")?;
        if !matches!(host.as_str(), "localhost" | "127.0.0.1" | "::1") {
            return Err(DomainError::Unavailable(
                "remote PostgreSQL requires the shared TLS pool",
            ));
        }
        let port = env::var("MD_PG_PORT").ok().map_or(Ok(5432_u16), |value| {
            value
                .parse::<u16>()
                .map_err(|_| DomainError::InvalidInput("invalid PostgreSQL port"))
        })?;
        let options = PgConnectOptions::new_without_pgpass()
            .host(&host)
            .port(port)
            .database(&required_env("MD_PG_DATABASE")?)
            .username(&required_env("MD_PG_USER")?)
            .password(&required_env("MD_PG_PASSWORD")?)
            .ssl_mode(PgSslMode::Disable)
            .options([
                ("statement_timeout", "5000"),
                ("lock_timeout", "2000"),
                ("search_path", "pg_catalog"),
            ]);
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .acquire_timeout(Duration::from_secs(4))
            .connect_with(options)
            .await?;
        Self::new(pool, required_env("MD_PG_NAMESPACE")?)
    }

    pub fn new(pool: PgPool, namespace: String) -> Result<Self, DomainError> {
        if namespace.is_empty()
            || namespace.len() > 128
            || !namespace
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        {
            return Err(DomainError::InvalidInput("invalid PostgreSQL namespace"));
        }
        Ok(Self { pool, namespace })
    }

    pub async fn add_history(
        &self,
        agent_id: &str,
        cwd: Option<&str>,
        text: &str,
    ) -> Result<bool, DomainError> {
        let text = text.trim();
        if agent_id.is_empty() || text.is_empty() {
            return Err(DomainError::InvalidInput("agent and prompt are required"));
        }
        let result = sqlx::query(
            "INSERT INTO munder_difflin.command_history(namespace,event_id,agent_id,cwd,text,occurred_at) \
             VALUES($1,$2::uuid,$3,$4,$5,now()) ON CONFLICT(namespace,event_id) DO NOTHING",
        )
        .bind(&self.namespace)
        .bind(event_id())
        .bind(agent_id)
        .bind(cwd)
        .bind(text)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn query_history(
        &self,
        request: &HistoryQuery,
    ) -> Result<Vec<CommandHistoryEntry>, DomainError> {
        let limit = request.limit.clamp(1, MAX_HISTORY_LIMIT);
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
                 WHERE namespace=$1 AND text ILIKE $2 ESCAPE '\\\\' \
                 AND ($3::text IS NULL OR agent_id=$3) \
                 ORDER BY occurred_at DESC,id DESC LIMIT $4",
            )
            .bind(&self.namespace)
            .bind(pattern)
            .bind(request.agent_id.as_deref())
            .bind(i64::from(limit))
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
        .bind(&self.namespace)
        .bind(request.agent_id.as_deref())
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(row_to_history).collect()
    }

    pub async fn append_cost(&self, sample: &AgentUsageSample) -> Result<bool, DomainError> {
        if sample.agent_id.is_empty()
            || sample.session_id.is_empty()
            || !sample.usd.is_finite()
            || sample.usd < 0.0
        {
            return Err(DomainError::InvalidInput("invalid cost sample"));
        }
        let result = sqlx::query(
            "INSERT INTO munder_difflin.cost_ledger \
             (namespace,event_id,agent_id,session_id,occurred_at,input_tokens,output_tokens,\
              cache_read_tokens,cache_creation_tokens,model,usd) \
             VALUES($1,$2::uuid,$3,$4,to_timestamp($5::double precision/1000.0),$6,$7,$8,$9,$10,\
                    $11::double precision::numeric) \
             ON CONFLICT(namespace,event_id) DO NOTHING",
        )
        .bind(&self.namespace)
        .bind(event_id())
        .bind(&sample.agent_id)
        .bind(&sample.session_id)
        .bind(sample.timestamp_ms as f64)
        .bind(
            i64::try_from(sample.input_tokens)
                .map_err(|_| DomainError::InvalidInput("input token overflow"))?,
        )
        .bind(
            i64::try_from(sample.output_tokens)
                .map_err(|_| DomainError::InvalidInput("output token overflow"))?,
        )
        .bind(
            i64::try_from(sample.cache_read_tokens)
                .map_err(|_| DomainError::InvalidInput("cache token overflow"))?,
        )
        .bind(
            i64::try_from(sample.cache_creation_tokens)
                .map_err(|_| DomainError::InvalidInput("cache creation token overflow"))?,
        )
        .bind(&sample.model)
        .bind(sample.usd)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn lifetime_cost_totals(&self) -> Result<Vec<AgentCostTotal>, DomainError> {
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
             ) SELECT agent_id,sum(peak)::double precision AS usd FROM peaks GROUP BY agent_id",
        )
        .bind(&self.namespace)
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
            .map_err(DomainError::Database)
    }
}

fn required_env(key: &'static str) -> Result<String, DomainError> {
    env::var(key)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .ok_or(DomainError::Unavailable(key))
}

fn row_to_history(row: sqlx::postgres::PgRow) -> Result<CommandHistoryEntry, DomainError> {
    Ok(CommandHistoryEntry {
        id: row.try_get("id")?,
        agent_id: row.try_get("agent_id")?,
        cwd: row.try_get("cwd")?,
        text: row.try_get("text")?,
        timestamp_ms: row.try_get("timestamp_ms")?,
    })
}

fn escape_like(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        if matches!(character, '\\' | '%' | '_') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

fn event_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0_u128, |duration| duration.as_nanos());
    let sequence = u128::from(EVENT_SEQUENCE.fetch_add(1, Ordering::Relaxed));
    let hex = format!("{:032x}", nanos ^ sequence);
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

#[cfg(test)]
mod tests {
    use super::{escape_like, event_id};

    #[test]
    fn history_pattern_escapes_wildcards() {
        assert_eq!(escape_like(r"100%_done\ok"), r"100\%\_done\\ok");
    }

    #[test]
    fn event_id_has_uuid_text_shape() {
        assert_eq!(event_id().len(), 36);
    }
}
