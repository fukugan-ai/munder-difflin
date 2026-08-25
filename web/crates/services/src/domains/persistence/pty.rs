use std::collections::BTreeSet;

use md_web_contracts::domains::persistence::{
    FLOOR_AGENT_KIND, FloorAgentRevision, FloorAgentWrite, NaturalExitDisposition,
    NaturalExitReceipt, NaturalExitWrite, PersistedFloorAgent, PersistedTerminalQueue,
    RecordDomain, RecordKey, RecordWrite, TERMINAL_QUEUE_KIND, TerminalQueueEnqueue,
    TerminalQueueFailureReceipt, TerminalQueueHeadMutation,
};
use md_web_contracts::domains::pty_agents::{AgentRecord, AgentStatus, QueuedTerminalMessage};
use serde::{Deserialize, Serialize};
use sqlx::Row;

use super::validation::{clamp_limit, validate_payload, validate_segment};
use super::{PgPersistenceRepository, RepositoryError};

const MAX_FLOOR_ID_BYTES: usize = 48;
const MAX_AGENT_ID_BYTES: usize = 128;
const MAX_MESSAGE_ID_BYTES: usize = 128;
const MAX_MESSAGE_BYTES: usize = 256 * 1024;
const MAX_SEND_ATTEMPTS: u8 = 3;
const EXIT_EVENT_TYPE: &str = "agent_natural_exit";

#[derive(Deserialize, Serialize)]
struct AgentPayload {
    floor_id: String,
    agent: AgentRecord,
}

#[derive(Deserialize, Serialize)]
struct QueuePayload {
    floor_id: String,
    agent_id: String,
    messages: Vec<QueuedTerminalMessage>,
}

#[derive(Deserialize, Serialize)]
struct ExitEventPayload {
    event_type: String,
    floor_id: String,
    agent_id: String,
    exit_code: Option<i32>,
    agent_revision: i64,
    queue_revision: Option<i64>,
    cleared_messages: u64,
    disposition: NaturalExitDisposition,
}

impl PgPersistenceRepository {
    /// Loads one durable agent recipe and lifecycle state.
    pub async fn get_floor_agent(
        &self,
        floor_id: &str,
        agent_id: &str,
    ) -> Result<Option<PersistedFloorAgent>, RepositoryError> {
        let key = agent_key(floor_id, agent_id)?;
        self.get_record(&key)
            .await?
            .map(decode_agent_record)
            .transpose()
    }

    /// Lists durable agents for exactly one floor, newest-first.
    pub async fn list_floor_agents(
        &self,
        floor_id: &str,
        limit: u16,
    ) -> Result<Vec<PersistedFloorAgent>, RepositoryError> {
        validate_identifier(floor_id, MAX_FLOOR_ID_BYTES, "floor id is invalid")?;
        let rows = sqlx::query(
            "SELECT revision,payload::text AS payload_json,\
             (extract(epoch from updated_at)*1000)::bigint AS updated_at_ms \
             FROM munder_difflin.web_durable_records \
             WHERE namespace=$1 AND domain='floors' AND kind='agent' \
               AND payload->>'floor_id'=$2 \
             ORDER BY updated_at DESC,record_id LIMIT $3",
        )
        .bind(self.namespace.as_str())
        .bind(floor_id)
        .bind(clamp_limit(
            limit,
            md_web_contracts::domains::persistence::MAX_PAGE_LIMIT,
        ))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                decode_agent_parts(
                    row.try_get("revision")?,
                    row.try_get("payload_json")?,
                    row.try_get("updated_at_ms")?,
                )
            })
            .collect()
    }

    /// Creates or compare-and-swap updates one durable floor agent.
    pub async fn upsert_floor_agent(
        &self,
        request: &FloorAgentWrite,
    ) -> Result<PersistedFloorAgent, RepositoryError> {
        validate_agent_write(request)?;
        let payload_json = encode(&AgentPayload {
            floor_id: request.floor_id.clone(),
            agent: request.agent.clone(),
        })?;
        let record = self
            .write_record(&RecordWrite {
                key: agent_key(&request.floor_id, &request.agent.id)?,
                expected_revision: request.expected_revision,
                payload_json,
            })
            .await?;
        decode_agent_record(record)
    }

    /// Marks one durable agent archived using optimistic versioning.
    pub async fn archive_floor_agent(
        &self,
        request: &FloorAgentRevision,
    ) -> Result<PersistedFloorAgent, RepositoryError> {
        self.transition_floor_agent(request, NaturalExitDisposition::Archived)
            .await
    }

    /// Marks one non-privileged durable agent restorable using optimistic versioning.
    pub async fn mark_floor_agent_restorable(
        &self,
        request: &FloorAgentRevision,
    ) -> Result<PersistedFloorAgent, RepositoryError> {
        self.transition_floor_agent(request, NaturalExitDisposition::Restorable)
            .await
    }

    /// Loads the durable terminal FIFO for one floor agent.
    pub async fn load_terminal_queue(
        &self,
        floor_id: &str,
        agent_id: &str,
    ) -> Result<Option<PersistedTerminalQueue>, RepositoryError> {
        let key = queue_key(floor_id, agent_id)?;
        self.get_record(&key)
            .await?
            .map(decode_queue_record)
            .transpose()
    }

    /// Appends one message through queue-level compare-and-swap. A retry with the
    /// same stable id and identical message returns the committed queue.
    pub async fn enqueue_terminal_message(
        &self,
        request: &TerminalQueueEnqueue,
    ) -> Result<PersistedTerminalQueue, RepositoryError> {
        validate_queue_message(&request.floor_id, &request.message)?;
        if request.expected_revision < 0 {
            return Err(RepositoryError::InvalidInput(
                "queue revision must be non-negative",
            ));
        }
        let current = self
            .load_terminal_queue(&request.floor_id, &request.message.agent_id)
            .await?;
        let mut queue = match current {
            Some(queue) => {
                if let Some(existing) = queue
                    .messages
                    .iter()
                    .find(|message| message.id == request.message.id)
                {
                    if existing == &request.message {
                        return Ok(queue);
                    }
                    return Err(RepositoryError::Conflict);
                }
                ensure_revision(queue.revision, request.expected_revision)?;
                queue
            }
            None if request.expected_revision == 0 => PersistedTerminalQueue {
                floor_id: request.floor_id.clone(),
                agent_id: request.message.agent_id.clone(),
                revision: 0,
                messages: Vec::new(),
                updated_at_ms: request.message.queued_at_ms,
            },
            None => return Err(RepositoryError::Conflict),
        };
        queue.messages.push(request.message.clone());
        self.write_queue(queue, request.expected_revision).await
    }

    /// Removes the acknowledged head only when id and revision still match.
    pub async fn acknowledge_terminal_message(
        &self,
        request: &TerminalQueueHeadMutation,
    ) -> Result<PersistedTerminalQueue, RepositoryError> {
        let mut queue = self.load_queue_for_mutation(request).await?;
        ensure_head(&queue, &request.message_id)?;
        queue.messages.remove(0);
        self.write_queue(queue, request.expected_revision).await
    }

    /// Increments the head failure count and drops it on the third failed delivery.
    pub async fn record_terminal_failure(
        &self,
        request: &TerminalQueueHeadMutation,
    ) -> Result<TerminalQueueFailureReceipt, RepositoryError> {
        let mut queue = self.load_queue_for_mutation(request).await?;
        ensure_head(&queue, &request.message_id)?;
        let head = queue
            .messages
            .first_mut()
            .ok_or(RepositoryError::NotFound)?;
        head.failed_attempts = head.failed_attempts.saturating_add(1);
        let dropped = if head.failed_attempts >= MAX_SEND_ATTEMPTS {
            Some(queue.messages.remove(0))
        } else {
            None
        };
        let queue = self.write_queue(queue, request.expected_revision).await?;
        Ok(TerminalQueueFailureReceipt { queue, dropped })
    }

    /// Explicitly drops the matching queue head after a delivery-time precondition fails.
    pub async fn drop_terminal_message(
        &self,
        request: &TerminalQueueHeadMutation,
    ) -> Result<(PersistedTerminalQueue, QueuedTerminalMessage), RepositoryError> {
        let mut queue = self.load_queue_for_mutation(request).await?;
        ensure_head(&queue, &request.message_id)?;
        let dropped = queue.messages.remove(0);
        let queue = self.write_queue(queue, request.expected_revision).await?;
        Ok((queue, dropped))
    }

    /// Persists explicit kill or natural exit as one idempotent transaction:
    /// transition the agent, clear its queue, and append one ordered floor event.
    pub async fn persist_agent_exit(
        &self,
        request: &NaturalExitWrite,
    ) -> Result<NaturalExitReceipt, RepositoryError> {
        validate_exit_write(request)?;
        let record_id = record_id(&request.floor_id, &request.agent_id);
        let stream = format!("floor:{}", request.floor_id);
        let mut transaction = self.pool.begin().await?;

        sqlx::query(
            "INSERT INTO munder_difflin.web_event_stream_heads(namespace,stream,last_sequence) \
             VALUES($1,$2,0) ON CONFLICT(namespace,stream) DO NOTHING",
        )
        .bind(self.namespace.as_str())
        .bind(&stream)
        .execute(&mut *transaction)
        .await?;
        let head: i64 = sqlx::query_scalar(
            "SELECT last_sequence FROM munder_difflin.web_event_stream_heads \
             WHERE namespace=$1 AND stream=$2 FOR UPDATE",
        )
        .bind(self.namespace.as_str())
        .bind(&stream)
        .fetch_one(&mut *transaction)
        .await?;

        let existing = sqlx::query(
            "SELECT sequence,payload::text AS payload_json \
             FROM munder_difflin.web_event_replay \
             WHERE namespace=$1 AND stream=$2 AND event_id=$3::uuid",
        )
        .bind(self.namespace.as_str())
        .bind(&stream)
        .bind(&request.event_id)
        .fetch_optional(&mut *transaction)
        .await?;
        if let Some(row) = existing {
            let receipt = decode_exit_receipt(
                request,
                row.try_get("sequence")?,
                row.try_get("payload_json")?,
            )?;
            transaction.commit().await?;
            return Ok(receipt);
        }

        let agent_row = sqlx::query(
            "SELECT revision,payload::text AS payload_json \
             FROM munder_difflin.web_durable_records \
             WHERE namespace=$1 AND domain='floors' AND kind='agent' AND record_id=$2 \
             FOR UPDATE",
        )
        .bind(self.namespace.as_str())
        .bind(&record_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(RepositoryError::NotFound)?;
        let agent_revision: i64 = agent_row.try_get("revision")?;
        ensure_revision(agent_revision, request.expected_agent_revision)?;
        let mut agent_payload: AgentPayload = decode(agent_row.try_get("payload_json")?)?;
        validate_agent_payload(&agent_payload, &request.floor_id, &request.agent_id)?;
        apply_disposition(&mut agent_payload.agent, request.disposition)?;
        let agent_json = encode(&agent_payload)?;
        let next_agent_revision: i64 = sqlx::query_scalar(
            "UPDATE munder_difflin.web_durable_records \
             SET revision=revision+1,payload=$4::jsonb,updated_at=now() \
             WHERE namespace=$1 AND domain='floors' AND kind='agent' \
               AND record_id=$2 AND revision=$3 RETURNING revision",
        )
        .bind(self.namespace.as_str())
        .bind(&record_id)
        .bind(request.expected_agent_revision)
        .bind(agent_json)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(RepositoryError::Conflict)?;

        let queue_row = sqlx::query(
            "SELECT revision,payload::text AS payload_json \
             FROM munder_difflin.web_durable_records \
             WHERE namespace=$1 AND domain='floors' AND kind='terminal_queue' \
               AND record_id=$2 FOR UPDATE",
        )
        .bind(self.namespace.as_str())
        .bind(&record_id)
        .fetch_optional(&mut *transaction)
        .await?;
        let (queue_revision, cleared_messages) = match queue_row {
            Some(row) => {
                let revision: i64 = row.try_get("revision")?;
                let mut payload: QueuePayload = decode(row.try_get("payload_json")?)?;
                validate_queue_payload(&payload, &request.floor_id, &request.agent_id)?;
                let cleared = u64::try_from(payload.messages.len())
                    .map_err(|_| RepositoryError::InvalidData("queue length exceeds range"))?;
                payload.messages.clear();
                let next: i64 = sqlx::query_scalar(
                    "UPDATE munder_difflin.web_durable_records \
                     SET revision=revision+1,payload=$4::jsonb,updated_at=now() \
                     WHERE namespace=$1 AND domain='floors' AND kind='terminal_queue' \
                       AND record_id=$2 AND revision=$3 RETURNING revision",
                )
                .bind(self.namespace.as_str())
                .bind(&record_id)
                .bind(revision)
                .bind(encode(&payload)?)
                .fetch_optional(&mut *transaction)
                .await?
                .ok_or(RepositoryError::Conflict)?;
                (Some(next), cleared)
            }
            None => (None, 0),
        };

        let sequence = head
            .checked_add(1)
            .ok_or(RepositoryError::SequenceExhausted)?;
        let event_payload = ExitEventPayload {
            event_type: String::from(EXIT_EVENT_TYPE),
            floor_id: request.floor_id.clone(),
            agent_id: request.agent_id.clone(),
            exit_code: request.exit_code,
            agent_revision: next_agent_revision,
            queue_revision,
            cleared_messages,
            disposition: request.disposition,
        };
        sqlx::query(
            "INSERT INTO munder_difflin.web_event_replay\
               (namespace,stream,sequence,event_id,occurred_at,payload) \
             VALUES($1,$2,$3,$4::uuid,to_timestamp($5::double precision/1000.0),$6::jsonb)",
        )
        .bind(self.namespace.as_str())
        .bind(&stream)
        .bind(sequence)
        .bind(&request.event_id)
        .bind(request.occurred_at_ms as f64)
        .bind(encode(&event_payload)?)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "UPDATE munder_difflin.web_event_stream_heads \
             SET last_sequence=$3,updated_at=now() WHERE namespace=$1 AND stream=$2",
        )
        .bind(self.namespace.as_str())
        .bind(&stream)
        .bind(sequence)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;

        Ok(NaturalExitReceipt {
            event_id: request.event_id.clone(),
            event_sequence: u64::try_from(sequence)
                .map_err(|_| RepositoryError::InvalidData("negative event sequence"))?,
            agent_revision: next_agent_revision,
            queue_revision,
            cleared_messages,
            disposition: request.disposition,
        })
    }

    async fn transition_floor_agent(
        &self,
        request: &FloorAgentRevision,
        disposition: NaturalExitDisposition,
    ) -> Result<PersistedFloorAgent, RepositoryError> {
        validate_ids(&request.floor_id, &request.agent_id)?;
        if request.expected_revision < 1 {
            return Err(RepositoryError::InvalidInput(
                "agent revision must be positive",
            ));
        }
        let current = self
            .get_floor_agent(&request.floor_id, &request.agent_id)
            .await?
            .ok_or(RepositoryError::NotFound)?;
        ensure_revision(current.revision, request.expected_revision)?;
        let mut agent = current.agent;
        apply_disposition(&mut agent, disposition)?;
        self.upsert_floor_agent(&FloorAgentWrite {
            floor_id: request.floor_id.clone(),
            expected_revision: request.expected_revision,
            agent,
        })
        .await
    }

    async fn load_queue_for_mutation(
        &self,
        request: &TerminalQueueHeadMutation,
    ) -> Result<PersistedTerminalQueue, RepositoryError> {
        validate_ids(&request.floor_id, &request.agent_id)?;
        validate_identifier(
            &request.message_id,
            MAX_MESSAGE_ID_BYTES,
            "queue message id is invalid",
        )?;
        if request.expected_revision < 1 {
            return Err(RepositoryError::InvalidInput(
                "queue revision must be positive",
            ));
        }
        let queue = self
            .load_terminal_queue(&request.floor_id, &request.agent_id)
            .await?
            .ok_or(RepositoryError::NotFound)?;
        ensure_revision(queue.revision, request.expected_revision)?;
        Ok(queue)
    }

    async fn write_queue(
        &self,
        queue: PersistedTerminalQueue,
        expected_revision: i64,
    ) -> Result<PersistedTerminalQueue, RepositoryError> {
        let payload_json = encode(&QueuePayload {
            floor_id: queue.floor_id.clone(),
            agent_id: queue.agent_id.clone(),
            messages: queue.messages,
        })?;
        let record = self
            .write_record(&RecordWrite {
                key: queue_key(&queue.floor_id, &queue.agent_id)?,
                expected_revision,
                payload_json,
            })
            .await?;
        decode_queue_record(record)
    }
}

fn validate_agent_write(request: &FloorAgentWrite) -> Result<(), RepositoryError> {
    validate_ids(&request.floor_id, &request.agent.id)?;
    if request.expected_revision < 0 {
        return Err(RepositoryError::InvalidInput(
            "agent revision must be non-negative",
        ));
    }
    if request.agent.archived != (request.agent.status == AgentStatus::Archived) {
        return Err(RepositoryError::InvalidInput(
            "agent archive fields are inconsistent",
        ));
    }
    Ok(())
}

fn validate_exit_write(request: &NaturalExitWrite) -> Result<(), RepositoryError> {
    validate_ids(&request.floor_id, &request.agent_id)?;
    validate_identifier(&request.event_id, 64, "natural exit event id is invalid")?;
    if request.expected_agent_revision < 1 {
        return Err(RepositoryError::InvalidInput(
            "agent revision must be positive",
        ));
    }
    Ok(())
}

fn validate_queue_message(
    floor_id: &str,
    message: &QueuedTerminalMessage,
) -> Result<(), RepositoryError> {
    validate_ids(floor_id, &message.agent_id)?;
    validate_identifier(
        &message.id,
        MAX_MESSAGE_ID_BYTES,
        "queue message id is invalid",
    )?;
    if message.text.trim().is_empty()
        || message.text.len() > MAX_MESSAGE_BYTES
        || message
            .instruction
            .as_deref()
            .is_some_and(|instruction| instruction.len() > MAX_MESSAGE_BYTES)
    {
        return Err(RepositoryError::InvalidInput(
            "queue message is empty or too large",
        ));
    }
    Ok(())
}

fn validate_ids(floor_id: &str, agent_id: &str) -> Result<(), RepositoryError> {
    validate_identifier(floor_id, MAX_FLOOR_ID_BYTES, "floor id is invalid")?;
    validate_identifier(agent_id, MAX_AGENT_ID_BYTES, "agent id is invalid")
}

fn validate_identifier(
    value: &str,
    maximum_bytes: usize,
    message: &'static str,
) -> Result<(), RepositoryError> {
    validate_segment(value, maximum_bytes, message)?;
    if value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        Ok(())
    } else {
        Err(RepositoryError::InvalidInput(message))
    }
}

fn validate_agent_payload(
    payload: &AgentPayload,
    floor_id: &str,
    agent_id: &str,
) -> Result<(), RepositoryError> {
    if payload.floor_id != floor_id || payload.agent.id != agent_id {
        return Err(RepositoryError::InvalidData(
            "durable agent identity mismatch",
        ));
    }
    validate_agent_consistency(&payload.agent)
}

fn validate_queue_payload(
    payload: &QueuePayload,
    floor_id: &str,
    agent_id: &str,
) -> Result<(), RepositoryError> {
    if payload.floor_id != floor_id || payload.agent_id != agent_id {
        return Err(RepositoryError::InvalidData(
            "durable queue identity mismatch",
        ));
    }
    let mut ids = BTreeSet::new();
    for message in &payload.messages {
        validate_queue_message(floor_id, message)
            .map_err(|_| RepositoryError::InvalidData("invalid durable queue message"))?;
        if message.agent_id != agent_id || !ids.insert(message.id.as_str()) {
            return Err(RepositoryError::InvalidData(
                "durable queue identity mismatch",
            ));
        }
    }
    Ok(())
}

fn validate_agent_consistency(agent: &AgentRecord) -> Result<(), RepositoryError> {
    if agent.archived == (agent.status == AgentStatus::Archived) {
        Ok(())
    } else {
        Err(RepositoryError::InvalidData(
            "durable agent archive fields are inconsistent",
        ))
    }
}

fn apply_disposition(
    agent: &mut AgentRecord,
    disposition: NaturalExitDisposition,
) -> Result<(), RepositoryError> {
    if disposition == NaturalExitDisposition::Restorable
        && (agent.role.orchestrator || agent.role.assistant)
    {
        return Err(RepositoryError::InvalidInput(
            "privileged agent cannot be marked restorable",
        ));
    }
    agent.pty_id = None;
    match disposition {
        NaturalExitDisposition::Exited => {
            agent.status = AgentStatus::Exited;
            agent.action_ja = String::from("終了");
            agent.archived = false;
        }
        NaturalExitDisposition::Archived => {
            agent.status = AgentStatus::Archived;
            agent.action_ja = String::from("アーカイブ済み");
            agent.archived = true;
        }
        NaturalExitDisposition::Restorable => {
            agent.status = AgentStatus::Restorable;
            agent.action_ja = String::from("復元可能");
            agent.archived = false;
        }
    }
    Ok(())
}

fn ensure_revision(actual: i64, expected: i64) -> Result<(), RepositoryError> {
    if actual == expected {
        Ok(())
    } else {
        Err(RepositoryError::Conflict)
    }
}

fn ensure_head(queue: &PersistedTerminalQueue, message_id: &str) -> Result<(), RepositoryError> {
    if queue
        .messages
        .first()
        .is_some_and(|message| message.id == message_id)
    {
        Ok(())
    } else {
        Err(RepositoryError::Conflict)
    }
}

fn agent_key(floor_id: &str, agent_id: &str) -> Result<RecordKey, RepositoryError> {
    validate_ids(floor_id, agent_id)?;
    Ok(RecordKey {
        domain: RecordDomain::Floors,
        kind: String::from(FLOOR_AGENT_KIND),
        record_id: record_id(floor_id, agent_id),
    })
}

fn queue_key(floor_id: &str, agent_id: &str) -> Result<RecordKey, RepositoryError> {
    validate_ids(floor_id, agent_id)?;
    Ok(RecordKey {
        domain: RecordDomain::Floors,
        kind: String::from(TERMINAL_QUEUE_KIND),
        record_id: record_id(floor_id, agent_id),
    })
}

fn record_id(floor_id: &str, agent_id: &str) -> String {
    format!("{floor_id}:{agent_id}")
}

fn decode_agent_record(
    record: md_web_contracts::domains::persistence::DurableRecord,
) -> Result<PersistedFloorAgent, RepositoryError> {
    decode_agent_parts(record.revision, record.payload_json, record.updated_at_ms)
}

fn decode_agent_parts(
    revision: i64,
    payload_json: String,
    updated_at_ms: i64,
) -> Result<PersistedFloorAgent, RepositoryError> {
    let payload: AgentPayload = decode(payload_json)?;
    validate_ids(&payload.floor_id, &payload.agent.id)?;
    validate_agent_consistency(&payload.agent)?;
    Ok(PersistedFloorAgent {
        floor_id: payload.floor_id,
        revision,
        agent: payload.agent,
        updated_at_ms,
    })
}

fn decode_queue_record(
    record: md_web_contracts::domains::persistence::DurableRecord,
) -> Result<PersistedTerminalQueue, RepositoryError> {
    let payload: QueuePayload = decode(record.payload_json)?;
    validate_queue_payload(&payload, &payload.floor_id, &payload.agent_id)?;
    Ok(PersistedTerminalQueue {
        floor_id: payload.floor_id,
        agent_id: payload.agent_id,
        revision: record.revision,
        messages: payload.messages,
        updated_at_ms: record.updated_at_ms,
    })
}

fn decode_exit_receipt(
    request: &NaturalExitWrite,
    sequence: i64,
    payload_json: String,
) -> Result<NaturalExitReceipt, RepositoryError> {
    let payload: ExitEventPayload = decode(payload_json)?;
    if payload.event_type != EXIT_EVENT_TYPE
        || payload.floor_id != request.floor_id
        || payload.agent_id != request.agent_id
        || payload.exit_code != request.exit_code
        || payload.disposition != request.disposition
    {
        return Err(RepositoryError::Conflict);
    }
    Ok(NaturalExitReceipt {
        event_id: request.event_id.clone(),
        event_sequence: u64::try_from(sequence)
            .map_err(|_| RepositoryError::InvalidData("negative event sequence"))?,
        agent_revision: payload.agent_revision,
        queue_revision: payload.queue_revision,
        cleared_messages: payload.cleared_messages,
        disposition: payload.disposition,
    })
}

fn encode<T: Serialize>(value: &T) -> Result<String, RepositoryError> {
    let payload = serde_json::to_string(value)
        .map_err(|_| RepositoryError::InvalidData("persistence serialization failed"))?;
    validate_payload(&payload)?;
    Ok(payload)
}

fn decode<T: for<'de> Deserialize<'de>>(payload: String) -> Result<T, RepositoryError> {
    serde_json::from_str(&payload)
        .map_err(|_| RepositoryError::InvalidData("invalid persistence JSON"))
}

#[cfg(test)]
mod tests {
    use md_web_contracts::domains::persistence::{NaturalExitDisposition, PersistedTerminalQueue};
    use md_web_contracts::domains::pty_agents::{
        AgentProvider, AgentRecord, AgentRole, AgentStatus,
    };

    use super::{apply_disposition, ensure_head, record_id, validate_identifier};

    fn agent() -> AgentRecord {
        AgentRecord {
            id: String::from("dev-1"),
            name: String::from("Dev"),
            provider: AgentProvider::Codex,
            role: AgentRole::default(),
            description: String::new(),
            cwd: String::from("/repo"),
            command: String::from("codex"),
            args: Vec::new(),
            model: None,
            status: AgentStatus::Idle,
            action_ja: String::from("待機中"),
            pty_id: Some(String::from("pty-1")),
            worktree_path: None,
            session_id: None,
            archived: false,
        }
    }

    #[test]
    fn record_identity_separates_floor_and_agent() {
        assert_eq!(record_id("floor-1", "dev-1"), "floor-1:dev-1");
    }

    #[test]
    fn identifier_rejects_record_delimiter() {
        assert!(validate_identifier("floor:1", 48, "invalid").is_err());
    }

    #[test]
    fn archive_clears_live_terminal() {
        let mut record = agent();
        assert!(apply_disposition(&mut record, NaturalExitDisposition::Archived).is_ok());
        assert!(record.pty_id.is_none());
        assert!(record.archived);
        assert_eq!(record.status, AgentStatus::Archived);
    }

    #[test]
    fn privileged_agent_is_not_restorable() {
        let mut record = agent();
        record.role.orchestrator = true;
        assert!(apply_disposition(&mut record, NaturalExitDisposition::Restorable).is_err());
    }

    #[test]
    fn empty_queue_has_no_matching_head() {
        let queue = PersistedTerminalQueue {
            floor_id: String::from("floor-1"),
            agent_id: String::from("dev-1"),
            revision: 1,
            messages: Vec::new(),
            updated_at_ms: 0,
        };
        assert!(ensure_head(&queue, "q-1").is_err());
    }
}
