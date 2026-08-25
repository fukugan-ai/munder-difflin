use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingModel {
    MiniLm,
    EmbeddingGemma,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MemoryStatus {
    pub available: bool,
    pub enabled: bool,
    pub active: bool,
    pub initialized: bool,
    pub model: EmbeddingModel,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MemorySearchRequest {
    pub query: String,
    pub wing: Option<String>,
    pub results: u16,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CommandOutcome {
    pub ok: bool,
    pub output: String,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TextSearchHit {
    pub source: String,
    pub excerpt: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TextSearchResponse {
    pub ok: bool,
    pub results: Vec<TextSearchHit>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReflectResult {
    pub agent_id: String,
    pub condensed: bool,
    pub reason: String,
    pub old_bytes: Option<u64>,
    pub new_bytes: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KnowledgeStatus {
    pub enabled: bool,
    pub document_count: u64,
    pub chunk_count: u64,
    pub by_modality: BTreeMap<String, u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KnowledgeDocument {
    pub id: String,
    pub title: String,
    pub source: String,
    pub modality: String,
    pub mime: Option<String>,
    pub original_extension: String,
    pub bytes: u64,
    pub tags: Vec<String>,
    pub caption: Option<String>,
    pub chunk_count: u64,
    pub added_at: String,
    pub extractor: String,
    pub truncated: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KnowledgeDetail {
    pub document: KnowledgeDocument,
    pub text: String,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct MemoryGraphSnapshot {
    pub nodes: Vec<MemoryGraphNode>,
    pub edges: Vec<MemoryGraphEdge>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MemoryGraphNode {
    pub id: String,
    pub label: String,
    pub modality: String,
    pub weight: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MemoryGraphEdge {
    pub source: String,
    pub target: String,
    pub relation: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct KnowledgeHit {
    pub document_id: String,
    pub title: String,
    pub source: String,
    pub modality: String,
    pub chunk_index: u64,
    pub score: f64,
    pub snippet: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KnowledgeFileResult {
    pub ok: bool,
    pub source_name: String,
    pub document_id: Option<String>,
    pub chunk_count: Option<u64>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KnowledgeIngestResponse {
    pub ok: bool,
    pub results: Vec<KnowledgeFileResult>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KnowledgeUploadRequest {
    pub source_name: String,
    pub title: Option<String>,
    pub tags: Vec<String>,
    pub caption: Option<String>,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillProvider {
    Claude,
    OpenCode,
    Codex,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillScope {
    User,
    Project,
    Bundled,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LocalSkill {
    pub id: String,
    pub name: String,
    pub description: String,
    pub provider: SkillProvider,
    pub scope: SkillScope,
    pub managed_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CatalogSkill {
    pub name: String,
    pub description: String,
    pub url: String,
    pub category: String,
    pub owner: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SkillCatalogResponse {
    pub skills: Vec<CatalogSkill>,
    pub fetched_at_ms: i64,
    pub stale: bool,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SkillActionResponse {
    pub ok: bool,
    pub managed_id: Option<String>,
    pub error: Option<String>,
    pub unsupported: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ActivityEntry {
    pub timestamp_ms: i64,
    pub kind: String,
    pub summary: String,
    pub details: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct AgentUsageSample {
    pub agent_id: String,
    pub session_id: String,
    pub timestamp_ms: i64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    pub model: String,
    pub usd: f64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ToolSpan {
    pub agent_id: String,
    pub session_id: String,
    pub timestamp_ms: i64,
    pub tool: String,
    pub success: bool,
    pub duration_ms: u64,
    pub decision: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderUsageKind {
    Claude,
    Codex,
    Gemini,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageCounterMode {
    Cumulative,
    Delta,
}

/// Sanitized provider event. Raw transcripts and provider credentials never
/// cross this boundary.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ProviderUsageEvent {
    pub event_id: String,
    pub provider: ProviderUsageKind,
    pub counter_mode: UsageCounterMode,
    pub usage: AgentUsageSample,
    pub context_window_tokens: Option<u64>,
    pub tool_spans: Vec<ToolSpan>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ProviderUsageReceipt {
    pub inserted: bool,
    pub event_id: String,
    pub usd: f64,
    pub context_pct: Option<u8>,
    pub tool_spans_recorded: u32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct TelemetrySnapshot {
    pub usage: Vec<AgentUsageSample>,
    pub spans: BTreeMap<String, Vec<ToolSpan>>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ToolWaterfall {
    pub origin_ms: i64,
    pub duration_ms: u64,
    pub rows: Vec<ToolWaterfallRow>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ToolWaterfallRow {
    pub agent_id: String,
    pub tool: String,
    pub offset_ms: u64,
    pub duration_ms: u64,
    pub success: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CommandHistoryEntry {
    pub id: String,
    pub agent_id: String,
    pub cwd: Option<String>,
    pub text: String,
    pub timestamp_ms: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HistoryQuery {
    pub agent_id: Option<String>,
    pub query: Option<String>,
    pub limit: u16,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct AgentCostTotal {
    pub agent_id: String,
    pub usd: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct MemorySkillsSnapshot {
    pub memory: MemoryStatus,
    pub knowledge: KnowledgeStatus,
    pub documents: Vec<KnowledgeDocument>,
    pub local_skills: Vec<LocalSkill>,
    pub catalog: SkillCatalogResponse,
    pub activities: Vec<ActivityEntry>,
    pub telemetry: TelemetrySnapshot,
    pub history: Vec<CommandHistoryEntry>,
    pub costs: Vec<AgentCostTotal>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MemorySkillsEventKind {
    ActivityChanged,
    TelemetryChanged,
    MemoryChanged,
    KnowledgeChanged,
    SkillsChanged,
    HistoryChanged,
    CostChanged,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MemorySkillsEvent {
    pub sequence: u64,
    pub timestamp_ms: i64,
    pub kind: MemorySkillsEventKind,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryNamespaceResetPhase {
    Prepared,
    Reinitialized,
    Aborted,
}

/// Receipt for the two-phase memory runtime side of a namespace reset.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MemoryNamespaceResetReceipt {
    pub generation: u64,
    pub phase: MemoryNamespaceResetPhase,
    pub drained: bool,
    pub active_processes: u32,
    pub projections_cleared: bool,
    pub event_journal_cleared: bool,
    pub caches_invalidated: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillCompatibility {
    CodexPlugin,
    ClaudePlugin,
    SharedAgentSkill,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BaseSkillSourceKind {
    OpenAiProjectSkillsApi,
    OpenAiPluginMarketplace,
    AnthropicAgentSkills,
    AnthropicPluginMarketplace,
    GitHubRepository,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BaseSkillSourceView {
    pub id: String,
    pub name: String,
    pub kind: BaseSkillSourceKind,
    pub repository: String,
    pub reference: String,
    pub official: bool,
    pub authentication_configured: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BaseSkillCatalogEntry {
    pub id: String,
    pub source_id: String,
    pub name: String,
    pub description: String,
    pub relative_path: String,
    pub provenance: String,
    pub version: String,
    pub license: Option<String>,
    pub compatibility: Vec<SkillCompatibility>,
    pub installed: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BaseSkillCatalogSnapshot {
    pub sources: Vec<BaseSkillSourceView>,
    pub skills: Vec<BaseSkillCatalogEntry>,
    pub cached_at_ms: i64,
    pub stale: bool,
    pub error: Option<String>,
}

impl Default for BaseSkillCatalogSnapshot {
    fn default() -> Self {
        Self {
            sources: Vec::new(),
            skills: Vec::new(),
            cached_at_ms: 0,
            stale: true,
            error: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BaseSkillSelectionRequest {
    pub skill_ids: Vec<String>,
    pub confirmed: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SoftwareTeamRole {
    Orchestrator,
    Implementer,
    Verifier,
    DomainSpecialist,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RoleSkillAssignment {
    pub agent_id: String,
    pub display_name: String,
    pub role: SoftwareTeamRole,
    pub skill_ids: Vec<String>,
    pub task_condition: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TeamSkillAssignments {
    pub version: u32,
    pub assignments: Vec<RoleSkillAssignment>,
    pub specialists_on_demand: bool,
    pub updated_at_ms: i64,
}

impl TeamSkillAssignments {
    pub fn minimum_software_team() -> Self {
        Self {
            version: 1,
            assignments: vec![
                RoleSkillAssignment {
                    agent_id: String::from("aria"),
                    display_name: String::from("Aria"),
                    role: SoftwareTeamRole::Orchestrator,
                    skill_ids: vec![
                        String::from("aria-orchestration"),
                        String::from("graph-engineering"),
                        String::from("project-documentation"),
                    ],
                    task_condition: None,
                },
                RoleSkillAssignment {
                    agent_id: String::from("implementer"),
                    display_name: String::from("Implementer"),
                    role: SoftwareTeamRole::Implementer,
                    skill_ids: vec![
                        String::from("local-development"),
                        String::from("web-project-standards"),
                    ],
                    task_condition: None,
                },
                RoleSkillAssignment {
                    agent_id: String::from("verifier"),
                    display_name: String::from("Verifier"),
                    role: SoftwareTeamRole::Verifier,
                    skill_ids: vec![String::from("perfectionist-reviewer")],
                    task_condition: None,
                },
            ],
            specialists_on_demand: true,
            updated_at_ms: 0,
        }
    }
}

impl Default for TeamSkillAssignments {
    fn default() -> Self {
        Self::minimum_software_team()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        EmbeddingModel, HistoryQuery, MemorySearchRequest, SkillProvider, SkillScope,
        SoftwareTeamRole, TeamSkillAssignments,
    };

    #[test]
    fn memory_search_preserves_wing_scope() {
        let request = MemorySearchRequest {
            query: String::from("decision"),
            wing: Some(String::from("agent-1")),
            results: 5,
        };

        assert_eq!(request.wing.as_deref(), Some("agent-1"));
    }

    #[test]
    fn history_query_supports_global_listing() {
        let query = HistoryQuery {
            agent_id: None,
            query: None,
            limit: 100,
        };

        assert_eq!(query.limit, 100);
    }

    #[test]
    fn provider_and_scope_are_copy_values() {
        let provider = SkillProvider::Codex;
        let scope = SkillScope::Project;

        assert_eq!(
            (provider, scope),
            (SkillProvider::Codex, SkillScope::Project)
        );
    }

    #[test]
    fn embedding_model_has_multilingual_variant() {
        assert_eq!(
            EmbeddingModel::EmbeddingGemma,
            EmbeddingModel::EmbeddingGemma
        );
    }

    #[test]
    fn minimum_team_separates_implementation_and_verification() {
        let template = TeamSkillAssignments::minimum_software_team();
        assert!(template.specialists_on_demand);
        assert_eq!(template.assignments.len(), 3);
        assert!(template.assignments.iter().any(|assignment| {
            assignment.role == SoftwareTeamRole::Orchestrator
                && assignment
                    .skill_ids
                    .contains(&String::from("aria-orchestration"))
        }));
    }
}
