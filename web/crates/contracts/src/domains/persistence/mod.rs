//! Browser-safe persistence contracts for PostgreSQL-backed Web parity.
//!
//! JSON documents are carried as strings so this contract crate does not choose
//! a JSON implementation for callers. The repository validates them through a
//! PostgreSQL `jsonb` cast. Secret values have no persistence contract here.

use serde::{Deserialize, Serialize};

mod pty;

pub use pty::{
    FLOOR_AGENT_KIND, FloorAgentRevision, FloorAgentWrite, NaturalExitDisposition,
    NaturalExitReceipt, NaturalExitWrite, PersistedFloorAgent, PersistedTerminalQueue,
    TERMINAL_QUEUE_KIND, TerminalQueueEnqueue, TerminalQueueFailureReceipt,
    TerminalQueueHeadMutation,
};

pub const MAX_PAGE_LIMIT: u16 = 1_000;
pub const TRIGGER_HISTORY_RETENTION: u16 = 500;

/// Validated tenant/install namespace shared with the desktop PostgreSQL store.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Namespace(String);

impl Namespace {
    /// Parses the canonical ASCII namespace representation.
    #[must_use]
    pub fn parse(value: String) -> Option<Self> {
        let valid = !value.is_empty()
            && value.len() <= 128
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'));
        valid.then_some(Self(value))
    }

    /// Borrows the validated namespace for a database bind parameter.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for Namespace {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).ok_or_else(|| serde::de::Error::custom("invalid PostgreSQL namespace"))
    }
}

/// Stable ownership boundary for a lossless durable document.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordDomain {
    Tasks,
    Hive,
    Connections,
    Triggers,
    Floors,
}

impl RecordDomain {
    /// Database representation constrained by migration 002.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Tasks => "tasks",
            Self::Hive => "hive",
            Self::Connections => "connections",
            Self::Triggers => "triggers",
            Self::Floors => "floors",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RecordKey {
    pub domain: RecordDomain,
    pub kind: String,
    pub record_id: String,
}

/// Compare-and-swap write. Revision zero creates a record; later writes must
/// carry the exact revision returned by the preceding successful write.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RecordWrite {
    pub key: RecordKey,
    pub expected_revision: i64,
    pub payload_json: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DurableRecord {
    pub key: RecordKey,
    pub revision: i64,
    pub payload_json: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AppConfigWrite {
    pub expected_revision: i64,
    pub payload_json: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AppConfigDocument {
    pub revision: i64,
    pub payload_json: String,
    pub updated_at_ms: i64,
}

/// Stable event id makes an ambiguous commit safe to retry.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReplayEventWrite {
    pub stream: String,
    pub event_id: String,
    pub occurred_at_ms: i64,
    pub payload_json: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReplayEvent {
    pub stream: String,
    pub sequence: u64,
    pub event_id: String,
    pub occurred_at_ms: i64,
    pub payload_json: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReplayPage {
    pub gap: bool,
    pub events: Vec<ReplayEvent>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TriggerHistoryWrite {
    pub event_id: String,
    pub source: String,
    pub source_id: String,
    pub occurred_at_ms: i64,
    pub payload_json: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TriggerHistoryRecord {
    pub event_id: String,
    pub source: String,
    pub source_id: String,
    pub occurred_at_ms: i64,
    pub payload_json: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HistoryAppend {
    pub event_id: String,
    pub agent_id: String,
    pub cwd: Option<String>,
    pub text: String,
    pub occurred_at_ms: i64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CostAppend {
    pub event_id: String,
    pub agent_id: String,
    pub session_id: String,
    pub occurred_at_ms: i64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    pub model: Option<String>,
    pub usd: f64,
}

#[cfg(test)]
mod tests {
    use super::{Namespace, RecordDomain};

    #[test]
    fn namespace_rejects_empty_value() {
        assert_eq!(Namespace::parse(String::new()), None);
    }

    #[test]
    fn namespace_accepts_maximum_length() {
        let namespace = Namespace::parse("a".repeat(128));

        assert_eq!(
            namespace.as_ref().map(Namespace::as_str).map(str::len),
            Some(128)
        );
    }

    #[test]
    fn namespace_rejects_path_separator() {
        assert_eq!(Namespace::parse(String::from("team/one")), None);
    }

    #[test]
    fn namespace_deserialization_preserves_validation() {
        let decoded = serde_json::from_str::<Namespace>(r#""team/one""#);

        assert!(decoded.is_err());
    }

    #[test]
    fn record_domain_has_stable_database_value() {
        assert_eq!(RecordDomain::Floors.as_str(), "floors");
    }
}
