use dioxus::prelude::*;
use md_web_contracts::domains::memory_skills::{
    ActivityEntry, AgentCostTotal, AgentUsageSample, BaseSkillCatalogSnapshot,
    BaseSkillSelectionRequest, CatalogSkill, CommandHistoryEntry, CommandOutcome, HistoryQuery,
    KnowledgeDetail, KnowledgeDocument, KnowledgeHit, KnowledgeIngestResponse, KnowledgeStatus,
    KnowledgeUploadRequest, LocalSkill, MemoryGraphSnapshot, MemorySearchRequest,
    MemorySkillsEvent, MemorySkillsEventKind, MemorySkillsSnapshot, MemoryStatus, ReflectResult,
    SkillActionResponse, SkillCatalogResponse, TeamSkillAssignments, TelemetrySnapshot,
    TextSearchResponse, ToolSpan, ToolWaterfall,
};
#[cfg(feature = "server")]
use md_web_contracts::domains::memory_skills::{
    MemoryNamespaceResetPhase, MemoryNamespaceResetReceipt, ProviderUsageEvent,
    ProviderUsageReceipt,
};
#[cfg(feature = "server")]
use md_web_contracts::domains::persistence::{CostAppend, HistoryAppend};
#[cfg(feature = "server")]
use md_web_services::domains::connections::{ProviderSecretId, SecretId, SecretProvider};

#[cfg(feature = "server")]
use dioxus::server::axum::{
    Json,
    extract::Multipart,
    http::StatusCode,
    response::{IntoResponse, Response},
};

#[cfg(feature = "server")]
pub(crate) async fn knowledge_upload_multipart(mut multipart: Multipart) -> Response {
    let host = match host().await {
        Ok(host) => host,
        Err(error) => return upload_error(StatusCode::SERVICE_UNAVAILABLE, server_error(error)),
    };
    let mut source_name = None;
    let mut title = None;
    let mut caption = None;
    let mut tags = Vec::new();
    let mut staging = None;
    loop {
        let field = match multipart.next_field().await {
            Ok(Some(field)) => field,
            Ok(None) => break,
            Err(_) => return upload_error(StatusCode::BAD_REQUEST, "invalid multipart upload"),
        };
        let name = field.name().unwrap_or_default().to_owned();
        if name == "file" {
            let Some(file_name) = field.file_name().map(str::to_owned) else {
                return upload_error(StatusCode::BAD_REQUEST, "upload filename is required");
            };
            let mut target = match host.begin_upload() {
                Ok(target) => target,
                Err(error) => {
                    return upload_error(StatusCode::INTERNAL_SERVER_ERROR, server_error(error));
                }
            };
            let mut field = field;
            loop {
                match field.chunk().await {
                    Ok(Some(chunk)) => {
                        if let Err(error) = target.write_chunk(&chunk) {
                            return upload_error(
                                StatusCode::PAYLOAD_TOO_LARGE,
                                server_error(error),
                            );
                        }
                    }
                    Ok(None) => break,
                    Err(_) => {
                        return upload_error(StatusCode::BAD_REQUEST, "upload stream failed");
                    }
                }
            }
            source_name = Some(file_name);
            staging = Some(target);
        } else {
            let value = match bounded_text_field(field).await {
                Ok(value) => value,
                Err(response) => return *response,
            };
            match name.as_str() {
                "title" => title = non_empty(value),
                "caption" => caption = non_empty(value),
                "tags" => {
                    tags = value
                        .split(',')
                        .map(str::trim)
                        .filter(|tag| !tag.is_empty())
                        .take(32)
                        .map(str::to_owned)
                        .collect();
                }
                _ => {}
            }
        }
    }
    let Some(staging) = staging else {
        return upload_error(StatusCode::BAD_REQUEST, "file field is required");
    };
    let Some(source_name) = source_name else {
        return upload_error(StatusCode::BAD_REQUEST, "upload filename is required");
    };
    match staging.finish(
        &host,
        &source_name,
        title.as_deref(),
        &tags,
        caption.as_deref(),
    ) {
        Ok(result) => {
            publish(MemorySkillsEventKind::KnowledgeChanged);
            (StatusCode::OK, Json(result)).into_response()
        }
        Err(error) => upload_error(StatusCode::BAD_REQUEST, server_error(error)),
    }
}

#[cfg(feature = "server")]
async fn bounded_text_field(
    mut field: dioxus::server::axum::extract::multipart::Field<'_>,
) -> Result<String, Box<Response>> {
    const MAX_METADATA_BYTES: usize = 8 * 1024;
    let mut bytes = Vec::new();
    loop {
        match field.chunk().await {
            Ok(Some(chunk)) => {
                if bytes.len().saturating_add(chunk.len()) > MAX_METADATA_BYTES {
                    return Err(Box::new(upload_error(
                        StatusCode::PAYLOAD_TOO_LARGE,
                        "upload metadata is too large",
                    )));
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok(None) => break,
            Err(_) => {
                return Err(Box::new(upload_error(
                    StatusCode::BAD_REQUEST,
                    "invalid upload field",
                )));
            }
        }
    }
    String::from_utf8(bytes).map_err(|_| {
        Box::new(upload_error(
            StatusCode::BAD_REQUEST,
            "upload metadata must be UTF-8",
        ))
    })
}

#[cfg(feature = "server")]
fn non_empty(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}

#[cfg(feature = "server")]
fn upload_error(status: StatusCode, message: impl ToString) -> Response {
    (
        status,
        Json(serde_json::json!({ "ok": false, "error": message.to_string() })),
    )
        .into_response()
}

#[get("/api/memory-skills/snapshot")]
pub(crate) async fn memory_skills_snapshot() -> Result<MemorySkillsSnapshot, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let host = host().await.map_err(server_error)?;
        let (memory, knowledge, documents, local_skills, catalog, mut activities, telemetry) =
            tokio::task::spawn_blocking(move || {
                Ok::<_, md_web_services::domains::memory_skills::DomainError>((
                    host.memory.status(),
                    host.knowledge.status()?,
                    host.knowledge.list()?,
                    host.skills.list_local(),
                    host.catalog(false),
                    host.activity.tail(200)?,
                    telemetry_store().snapshot(),
                ))
            })
            .await
            .map_err(|_| ServerFnError::new("memory snapshot task failed"))?
            .map_err(server_error)?;
        activities.extend(runtime_activity_tail(200));
        activities.sort_by_key(|entry| entry.timestamp_ms);
        if activities.len() > 200 {
            activities.drain(..activities.len() - 200);
        }
        let (history, costs) = match super::persistence_repository().await {
            Ok(repository) => (
                repository
                    .query_history(&HistoryQuery {
                        agent_id: None,
                        query: None,
                        limit: 100,
                    })
                    .await
                    .unwrap_or_default(),
                repository.lifetime_cost_totals().await.unwrap_or_default(),
            ),
            Err(_) => (Vec::new(), Vec::new()),
        };
        Ok(MemorySkillsSnapshot {
            memory,
            knowledge,
            documents,
            local_skills,
            catalog,
            activities,
            telemetry,
            history,
            costs,
        })
    }
    #[cfg(not(feature = "server"))]
    Err(server_only())
}

#[get("/api/memory-skills/memory/status")]
pub(crate) async fn memory_status() -> Result<MemoryStatus, ServerFnError> {
    blocking(|host| Ok(host.memory.status())).await
}

#[get("/api/memory-skills/memory/text/:query")]
pub(crate) async fn memory_text_search(query: String) -> Result<TextSearchResponse, ServerFnError> {
    blocking(move |host| host.memory.text_search(&query)).await
}

#[post("/api/memory-skills/memory/semantic")]
pub(crate) async fn memory_semantic_search(
    request: MemorySearchRequest,
) -> Result<CommandOutcome, ServerFnError> {
    blocking(move |host| host.memory.semantic_search(&request)).await
}

#[post("/api/memory-skills/memory/mine")]
pub(crate) async fn memory_mine(agent_id: String) -> Result<CommandOutcome, ServerFnError> {
    let result = blocking(move |host| host.memory.mine_agent(&agent_id)).await?;
    publish(MemorySkillsEventKind::MemoryChanged);
    Ok(result)
}

#[post("/api/memory-skills/memory/wake-up")]
pub(crate) async fn memory_wake_up(
    agent_id: Option<String>,
) -> Result<CommandOutcome, ServerFnError> {
    blocking(move |host| host.memory.wake_up(agent_id.as_deref())).await
}

#[post("/api/memory-skills/memory/reflect")]
pub(crate) async fn memory_reflect(agent_id: String) -> Result<ReflectResult, ServerFnError> {
    let result = blocking(move |host| host.reflect(&agent_id)).await?;
    if result.condensed {
        publish(MemorySkillsEventKind::MemoryChanged);
    }
    Ok(result)
}

#[get("/api/memory-skills/knowledge/status")]
pub(crate) async fn knowledge_status() -> Result<KnowledgeStatus, ServerFnError> {
    blocking(|host| host.knowledge.status()).await
}

#[get("/api/memory-skills/knowledge/list")]
pub(crate) async fn knowledge_list() -> Result<Vec<KnowledgeDocument>, ServerFnError> {
    blocking(|host| host.knowledge.list()).await
}

#[get("/api/memory-skills/knowledge/get/:document_id")]
pub(crate) async fn knowledge_get(
    document_id: String,
) -> Result<Option<KnowledgeDetail>, ServerFnError> {
    blocking(move |host| host.knowledge.get_detail(&document_id)).await
}

#[get("/api/memory-skills/knowledge/graph")]
pub(crate) async fn memory_graph() -> Result<MemoryGraphSnapshot, ServerFnError> {
    blocking(|host| host.knowledge.graph()).await
}

#[get("/api/memory-skills/knowledge/search/:query/:limit")]
pub(crate) async fn knowledge_search(
    query: String,
    limit: usize,
) -> Result<Vec<KnowledgeHit>, ServerFnError> {
    blocking(move |host| host.knowledge.search(&query, limit)).await
}

#[post("/api/memory-skills/knowledge/upload")]
pub(crate) async fn knowledge_upload(
    request: KnowledgeUploadRequest,
) -> Result<KnowledgeIngestResponse, ServerFnError> {
    let result = blocking(move |host| host.ingest_upload(&request)).await?;
    publish(MemorySkillsEventKind::KnowledgeChanged);
    Ok(result)
}

#[post("/api/memory-skills/knowledge/remove")]
pub(crate) async fn knowledge_remove(document_id: String) -> Result<bool, ServerFnError> {
    let removed = blocking(move |host| host.knowledge.remove(&document_id)).await?;
    if removed {
        publish(MemorySkillsEventKind::KnowledgeChanged);
    }
    Ok(removed)
}

#[get("/api/memory-skills/skills/local")]
pub(crate) async fn skills_local() -> Result<Vec<LocalSkill>, ServerFnError> {
    blocking(|host| Ok(host.skills.list_local())).await
}

#[get("/api/memory-skills/skills/catalog/:force")]
pub(crate) async fn skills_catalog(force: bool) -> Result<SkillCatalogResponse, ServerFnError> {
    blocking(move |host| Ok(host.catalog(force))).await
}

#[post("/api/memory-skills/skills/install")]
pub(crate) async fn skills_install(
    entry: CatalogSkill,
) -> Result<SkillActionResponse, ServerFnError> {
    let result = blocking(move |host| host.install_catalog_skill(&entry)).await?;
    if result.ok {
        publish(MemorySkillsEventKind::SkillsChanged);
    }
    Ok(result)
}

#[post("/api/memory-skills/skills/uninstall")]
pub(crate) async fn skills_uninstall(
    managed_id: String,
) -> Result<SkillActionResponse, ServerFnError> {
    let result = blocking(move |host| host.skills.uninstall(&managed_id)).await?;
    if result.ok {
        publish(MemorySkillsEventKind::SkillsChanged);
    }
    Ok(result)
}

#[get("/api/memory-skills/base-skills/catalog/:refresh")]
pub(crate) async fn base_skills_catalog(
    refresh: bool,
) -> Result<BaseSkillCatalogSnapshot, ServerFnError> {
    if !refresh {
        return blocking(|host| Ok(host.base_skills()?.cached_catalog())).await;
    }
    #[cfg(feature = "server")]
    {
        let provider = super::connections::hydrated_secret_provider().await?;
        let openai_key = provider
            .get_secret(&SecretId::Provider(ProviderSecretId::OpenAi))
            .map_err(|_| ServerFnError::new("OpenAI skill credentials are unavailable"))?;
        let (service, github_catalog) = blocking(|host| {
            let service = host.base_skills()?;
            let catalog = service.refresh_catalog()?;
            Ok((service, catalog))
        })
        .await?;
        if !service.openai_project_skills_enabled() {
            return Ok(github_catalog);
        }
        let key = openai_key.ok_or_else(|| {
            ServerFnError::new("OpenAI project API key is required to refresh official skills")
        })?;
        return service
            .refresh_openai_project_skills(key.expose_for_server())
            .await
            .map_err(server_error);
    }
    #[cfg(not(feature = "server"))]
    Err(server_only())
}

#[post("/api/memory-skills/base-skills/install")]
pub(crate) async fn base_skills_install(
    request: BaseSkillSelectionRequest,
) -> Result<Vec<md_web_contracts::domains::memory_skills::BaseSkillCatalogEntry>, ServerFnError> {
    let installed = blocking(move |host| host.base_skills()?.install_selection(&request)).await?;
    if !installed.is_empty() {
        publish(MemorySkillsEventKind::SkillsChanged);
    }
    Ok(installed)
}

#[get("/api/memory-skills/base-skills/assignments")]
pub(crate) async fn base_skill_assignments() -> Result<TeamSkillAssignments, ServerFnError> {
    blocking(|host| Ok(host.base_skills()?.load_assignments())).await
}

#[post("/api/memory-skills/base-skills/assignments")]
pub(crate) async fn save_base_skill_assignments(
    assignments: TeamSkillAssignments,
) -> Result<TeamSkillAssignments, ServerFnError> {
    let saved = blocking(move |host| host.base_skills()?.save_assignments(assignments)).await?;
    publish(MemorySkillsEventKind::SkillsChanged);
    Ok(saved)
}

#[cfg(feature = "server")]
#[allow(
    dead_code,
    reason = "called by shared team spawn integration after onboarding wiring"
)]
pub(crate) async fn assigned_skill_injection(
    agent_id: &str,
    task_domains: &[String],
) -> Result<md_web_services::domains::memory_skills::AgentSkillInjection, ServerFnError> {
    host()
        .await
        .map_err(server_error)?
        .base_skills()
        .map_err(server_error)?
        .injection_for(agent_id, task_domains)
        .map_err(server_error)
}

#[get("/api/memory-skills/activity/:limit")]
pub(crate) async fn activity_tail(limit: usize) -> Result<Vec<ActivityEntry>, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let mut activities = blocking(move |host| host.activity.tail(limit)).await?;
        activities.extend(runtime_activity_tail(limit));
        activities.sort_by_key(|entry| entry.timestamp_ms);
        let limit = limit.clamp(1, 1_000);
        if activities.len() > limit {
            activities.drain(..activities.len() - limit);
        }
        Ok(activities)
    }
    #[cfg(not(feature = "server"))]
    Err(server_only())
}

#[get("/api/memory-skills/telemetry")]
pub(crate) async fn telemetry_snapshot() -> Result<TelemetrySnapshot, ServerFnError> {
    #[cfg(feature = "server")]
    return Ok(telemetry_store().snapshot());
    #[cfg(not(feature = "server"))]
    Err(server_only())
}

#[get("/api/memory-skills/telemetry/spans/:agent_id")]
pub(crate) async fn telemetry_spans(agent_id: String) -> Result<Vec<ToolSpan>, ServerFnError> {
    #[cfg(feature = "server")]
    return Ok(telemetry_store().spans(&agent_id));
    #[cfg(not(feature = "server"))]
    Err(server_only())
}

#[post("/api/memory-skills/telemetry/waterfall")]
pub(crate) async fn telemetry_waterfall(
    agent_id: Option<String>,
) -> Result<ToolWaterfall, ServerFnError> {
    #[cfg(feature = "server")]
    return Ok(telemetry_store().waterfall(agent_id.as_deref()));
    #[cfg(not(feature = "server"))]
    Err(server_only())
}

#[post("/api/memory-skills/telemetry/usage")]
pub(crate) async fn telemetry_record_usage(sample: AgentUsageSample) -> Result<(), ServerFnError> {
    #[cfg(feature = "server")]
    {
        record_cli_usage(sample).await
    }
    #[cfg(not(feature = "server"))]
    Err(server_only())
}

#[post("/api/memory-skills/telemetry/span")]
pub(crate) async fn telemetry_record_span(span: ToolSpan) -> Result<(), ServerFnError> {
    #[cfg(feature = "server")]
    {
        record_tool_span(span)
    }
    #[cfg(not(feature = "server"))]
    Err(server_only())
}

#[post("/api/memory-skills/history/add")]
pub(crate) async fn history_add(
    agent_id: String,
    cwd: Option<String>,
    text: String,
) -> Result<bool, ServerFnError> {
    #[cfg(feature = "server")]
    {
        record_prompt_accepted(HistoryAppend {
            event_id: durable_event_id(),
            agent_id,
            cwd,
            text,
            occurred_at_ms: epoch_millis(),
        })
        .await
    }
    #[cfg(not(feature = "server"))]
    Err(server_only())
}

#[cfg(feature = "server")]
#[allow(dead_code, reason = "called by PTY/composer producer integration")]
pub(crate) async fn record_prompt_accepted(event: HistoryAppend) -> Result<bool, ServerFnError> {
    let inserted = super::persistence_repository()
        .await?
        .append_history(&event)
        .await
        .map_err(|_| ServerFnError::new("PostgreSQL operation failed"))?;
    if inserted {
        publish(MemorySkillsEventKind::HistoryChanged);
    }
    Ok(inserted)
}

#[cfg(feature = "server")]
#[allow(dead_code, reason = "called by CLI telemetry producer integration")]
pub(crate) async fn record_cli_usage(sample: AgentUsageSample) -> Result<(), ServerFnError> {
    telemetry_store().record_usage(sample.clone());
    super::persistence_repository()
        .await?
        .append_cost(&CostAppend {
            event_id: durable_event_id(),
            agent_id: sample.agent_id.clone(),
            session_id: sample.session_id.clone(),
            occurred_at_ms: sample.timestamp_ms,
            input_tokens: sample.input_tokens,
            output_tokens: sample.output_tokens,
            cache_read_tokens: sample.cache_read_tokens,
            cache_creation_tokens: sample.cache_creation_tokens,
            model: Some(sample.model.clone()),
            usd: sample.usd,
        })
        .await
        .map_err(|_| ServerFnError::new("PostgreSQL operation failed"))?;
    publish(MemorySkillsEventKind::TelemetryChanged);
    publish(MemorySkillsEventKind::CostChanged);
    super::pty::enforce_agent_token_cap(&sample).await?;
    Ok(())
}

#[cfg(feature = "server")]
#[allow(
    dead_code,
    reason = "called by Hive/provider transcript and exit producers"
)]
pub(crate) async fn record_provider_usage_event(
    event: ProviderUsageEvent,
) -> Result<ProviderUsageReceipt, ServerFnError> {
    let event = memory_runtime().usage_accumulator.accumulate(&event);
    let context_pct = md_web_services::domains::memory_skills::context_percentage(&event);
    let inserted = super::persistence_repository()
        .await?
        .append_cost(&CostAppend {
            event_id: event.event_id.clone(),
            agent_id: event.usage.agent_id.clone(),
            session_id: event.usage.session_id.clone(),
            occurred_at_ms: event.usage.timestamp_ms,
            input_tokens: event.usage.input_tokens,
            output_tokens: event.usage.output_tokens,
            cache_read_tokens: event.usage.cache_read_tokens,
            cache_creation_tokens: event.usage.cache_creation_tokens,
            model: Some(event.usage.model.clone()),
            usd: event.usage.usd,
        })
        .await
        .map_err(|_| ServerFnError::new("PostgreSQL operation failed"))?;
    let tool_spans_recorded = telemetry_store().record_provider_event(&event, inserted);
    if inserted {
        if let Some(context_pct) = context_pct
            && let Ok(mut context) = memory_runtime().context.write()
        {
            context.insert(
                event.usage.agent_id.clone(),
                md_web_services::domains::connections::ContextUsageSample {
                    agent_id: event.usage.agent_id.clone(),
                    context_pct,
                    large_window: event
                        .context_window_tokens
                        .is_some_and(|window| window > 200_000),
                },
            );
        }
        publish(MemorySkillsEventKind::TelemetryChanged);
        publish(MemorySkillsEventKind::CostChanged);
        record_activity_event(provider_activity(&event));
        let last_tool = event.tool_spans.last();
        let cost_usd_micros = (event.usage.usd * 1_000_000.0)
            .round()
            .clamp(0.0, u64::MAX as f64) as u64;
        super::office::office_live_update(
            md_web_contracts::domains::office_ui::OfficeLiveUpdate::Telemetry(
                md_web_contracts::domains::office_ui::OfficeAgentTelemetry {
                    agent_id: event.usage.agent_id.clone(),
                    input_tokens: event.usage.input_tokens,
                    output_tokens: event.usage.output_tokens,
                    cache_read_tokens: event.usage.cache_read_tokens,
                    cache_creation_tokens: event.usage.cache_creation_tokens,
                    cost_usd_micros,
                    last_tool: last_tool.map(|span| span.tool.clone()),
                    last_tool_duration_ms: last_tool.map(|span| span.duration_ms),
                    observed_at_ms: event.usage.timestamp_ms,
                },
            ),
        )
        .await?;
        super::pty::enforce_agent_token_cap(&event.usage).await?;
    }
    Ok(ProviderUsageReceipt {
        inserted,
        event_id: event.event_id,
        usd: event.usage.usd,
        context_pct,
        tool_spans_recorded: if inserted {
            u32::try_from(tool_spans_recorded).unwrap_or(u32::MAX)
        } else {
            0
        },
    })
}

#[cfg(feature = "server")]
#[allow(
    dead_code,
    reason = "called by server-only provider transcript adapters"
)]
pub(crate) async fn record_provider_transcript(
    provider: md_web_contracts::domains::memory_skills::ProviderUsageKind,
    source_event_id: &str,
    agent_id: &str,
    session_id: &str,
    timestamp_ms: i64,
    payload_json: &str,
) -> Result<ProviderUsageReceipt, ServerFnError> {
    let event = md_web_services::domains::memory_skills::sanitize_provider_transcript(
        &md_web_services::domains::memory_skills::ProviderTranscriptEvent {
            provider,
            source_event_id,
            agent_id,
            session_id,
            timestamp_ms,
            payload_json,
        },
    )
    .map_err(server_error)?;
    record_provider_usage_event(event).await
}

#[cfg(feature = "server")]
#[allow(dead_code, reason = "called by shared server startup")]
pub(crate) fn install_memory_context_usage_provider() -> Result<(), ServerFnError> {
    md_web_services::domains::connections::install_context_usage_provider(std::sync::Arc::new(
        MemoryContextUsageProvider,
    ))
    .map_err(|_| ServerFnError::new("context usage provider is already installed"))
}

#[cfg(feature = "server")]
#[allow(dead_code, reason = "called by CLI tool-span producer integration")]
pub(crate) fn record_tool_span(span: ToolSpan) -> Result<(), ServerFnError> {
    telemetry_store().record_span(span);
    publish(MemorySkillsEventKind::TelemetryChanged);
    Ok(())
}

#[cfg(feature = "server")]
#[allow(dead_code, reason = "called after typed activity log producers commit")]
pub(crate) fn record_activity_event(mut entry: ActivityEntry) {
    for forbidden in [
        "body", "message", "prompt", "content", "text", "token", "secret",
    ] {
        entry.details.remove(forbidden);
    }
    entry.summary = entry.summary.chars().take(256).collect();
    entry.details = entry
        .details
        .into_iter()
        .take(32)
        .map(|(key, value)| (key, value.chars().take(256).collect()))
        .collect();
    if let Ok(mut activities) = memory_runtime().activities.lock() {
        if activities.len() == 512 {
            activities.pop_front();
        }
        activities.push_back(entry);
    }
    publish(MemorySkillsEventKind::ActivityChanged);
}

#[post("/api/memory-skills/history/query")]
pub(crate) async fn history_query(
    request: HistoryQuery,
) -> Result<Vec<CommandHistoryEntry>, ServerFnError> {
    #[cfg(feature = "server")]
    return super::persistence_repository()
        .await?
        .query_history(&request)
        .await
        .map_err(|_| ServerFnError::new("PostgreSQL operation failed"));
    #[cfg(not(feature = "server"))]
    Err(server_only())
}

#[get("/api/memory-skills/costs")]
pub(crate) async fn cost_totals() -> Result<Vec<AgentCostTotal>, ServerFnError> {
    #[cfg(feature = "server")]
    return super::persistence_repository()
        .await?
        .lifetime_cost_totals()
        .await
        .map_err(|_| ServerFnError::new("PostgreSQL operation failed"));
    #[cfg(not(feature = "server"))]
    Err(server_only())
}

#[get("/api/memory-skills/events/:after")]
pub(crate) async fn memory_events(after: u64) -> Result<Vec<MemorySkillsEvent>, ServerFnError> {
    #[cfg(feature = "server")]
    return Ok(event_journal()
        .lock()
        .map(|events| {
            events
                .iter()
                .filter(|event| event.sequence > after)
                .cloned()
                .collect()
        })
        .unwrap_or_default());
    #[cfg(not(feature = "server"))]
    Err(server_only())
}

#[cfg(feature = "server")]
async fn blocking<T, F>(operation: F) -> Result<T, ServerFnError>
where
    T: Send + 'static,
    F: FnOnce(
            md_web_services::domains::memory_skills::MemorySkillsHost,
        ) -> Result<T, md_web_services::domains::memory_skills::DomainError>
        + Send
        + 'static,
{
    let host = host().await.map_err(server_error)?;
    tokio::task::spawn_blocking(move || operation(host))
        .await
        .map_err(|_| ServerFnError::new("memory service task failed"))?
        .map_err(server_error)
}

#[cfg(not(feature = "server"))]
#[allow(
    dead_code,
    reason = "Dioxus replaces server-function bodies with client stubs on web builds"
)]
async fn blocking<T, F>(_operation: F) -> Result<T, ServerFnError> {
    Err(server_only())
}

#[cfg(feature = "server")]
async fn host() -> Result<
    md_web_services::domains::memory_skills::MemorySkillsHost,
    md_web_services::domains::memory_skills::DomainError,
> {
    let repository = super::persistence_repository().await.map_err(|_| {
        md_web_services::domains::memory_skills::DomainError::Unavailable(
            "shared PostgreSQL runtime is unavailable",
        )
    })?;
    let config = md_web_services::domains::config_onboarding::load_config(&repository)
        .await
        .map_err(|_| {
            md_web_services::domains::memory_skills::DomainError::Unavailable(
                "runtime configuration is unavailable",
            )
        })?;
    md_web_services::domains::memory_skills::MemorySkillsHost::from_public_config_with_process(
        &config,
        process_control()?,
    )
}

#[cfg(feature = "server")]
fn process_control() -> Result<
    md_web_services::domains::memory_skills::ProcessControl,
    md_web_services::domains::memory_skills::DomainError,
> {
    let process = memory_runtime().process.lock().map_err(|_| {
        md_web_services::domains::memory_skills::DomainError::Unavailable(
            "memory runtime is unavailable",
        )
    })?;
    if process.reset_prepared {
        return Err(
            md_web_services::domains::memory_skills::DomainError::Unavailable(
                "memory namespace reset is in progress",
            ),
        );
    }
    Ok(process.control.clone())
}

#[cfg(feature = "server")]
#[allow(dead_code, reason = "called by the shared graceful shutdown owner")]
pub(crate) fn cancel_memory_processes() {
    if let Ok(process) = memory_runtime().process.lock() {
        process.control.cancel();
    }
}

#[cfg(feature = "server")]
#[allow(
    dead_code,
    reason = "called before the shared namespace reset transaction"
)]
pub(crate) async fn prepare_memory_namespace_reset()
-> Result<MemoryNamespaceResetReceipt, ServerFnError> {
    let (control, generation) = {
        let mut process = memory_runtime()
            .process
            .lock()
            .map_err(|_| ServerFnError::new("memory runtime is unavailable"))?;
        if process.reset_prepared {
            return Ok(reset_receipt(
                process.generation,
                MemoryNamespaceResetPhase::Prepared,
                true,
                0,
                false,
            ));
        }
        process.reset_prepared = true;
        (process.control.clone(), process.generation)
    };
    let drain = tokio::task::spawn_blocking(move || {
        control.cancel_and_wait(std::time::Duration::from_secs(5))
    })
    .await
    .map_err(|_| ServerFnError::new("memory process drain task failed"))?;
    Ok(reset_receipt(
        generation,
        MemoryNamespaceResetPhase::Prepared,
        drain.drained,
        u32::try_from(drain.active_after).unwrap_or(u32::MAX),
        false,
    ))
}

#[cfg(feature = "server")]
#[allow(
    dead_code,
    reason = "called after the shared namespace reset transaction"
)]
pub(crate) fn finish_memory_namespace_reset(
    committed: bool,
) -> Result<MemoryNamespaceResetReceipt, ServerFnError> {
    let runtime = memory_runtime();
    let mut process = runtime
        .process
        .lock()
        .map_err(|_| ServerFnError::new("memory runtime is unavailable"))?;
    if !process.reset_prepared {
        return Err(ServerFnError::new(
            "memory namespace reset was not prepared",
        ));
    }
    process.generation = process.generation.saturating_add(1);
    process.control = md_web_services::domains::memory_skills::ProcessControl::default();
    process.reset_prepared = false;
    if committed {
        runtime.telemetry.clear();
        runtime.usage_accumulator.clear();
        runtime
            .context
            .write()
            .map_err(|_| ServerFnError::new("memory context projection is unavailable"))?
            .clear();
        runtime
            .activities
            .lock()
            .map_err(|_| ServerFnError::new("memory activity projection is unavailable"))?
            .clear();
        runtime
            .events
            .lock()
            .map_err(|_| ServerFnError::new("memory event journal is unavailable"))?
            .clear();
    }
    Ok(reset_receipt(
        process.generation,
        if committed {
            MemoryNamespaceResetPhase::Reinitialized
        } else {
            MemoryNamespaceResetPhase::Aborted
        },
        true,
        0,
        committed,
    ))
}

#[cfg(feature = "server")]
fn telemetry_store() -> &'static md_web_services::domains::memory_skills::TelemetryStore {
    &memory_runtime().telemetry
}

#[cfg(feature = "server")]
fn event_journal() -> &'static std::sync::Mutex<std::collections::VecDeque<MemorySkillsEvent>> {
    &memory_runtime().events
}

#[cfg(feature = "server")]
fn publish(kind: MemorySkillsEventKind) {
    use std::sync::atomic::Ordering;
    let event = MemorySkillsEvent {
        sequence: memory_runtime()
            .sequence
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1),
        timestamp_ms: epoch_millis(),
        kind,
    };
    if let Ok(mut events) = event_journal().lock() {
        if events.len() == 512 {
            events.pop_front();
        }
        events.push_back(event);
    }
}

#[cfg(feature = "server")]
struct MemoryRuntime {
    process: std::sync::Mutex<MemoryProcessGeneration>,
    telemetry: md_web_services::domains::memory_skills::TelemetryStore,
    usage_accumulator: md_web_services::domains::memory_skills::ProviderUsageAccumulator,
    events: std::sync::Mutex<std::collections::VecDeque<MemorySkillsEvent>>,
    activities: std::sync::Mutex<std::collections::VecDeque<ActivityEntry>>,
    sequence: std::sync::atomic::AtomicU64,
    context: std::sync::RwLock<
        std::collections::BTreeMap<
            String,
            md_web_services::domains::connections::ContextUsageSample,
        >,
    >,
}

#[cfg(feature = "server")]
struct MemoryProcessGeneration {
    control: md_web_services::domains::memory_skills::ProcessControl,
    generation: u64,
    reset_prepared: bool,
}

#[cfg(feature = "server")]
fn memory_runtime() -> &'static MemoryRuntime {
    use std::sync::OnceLock;
    static RUNTIME: OnceLock<MemoryRuntime> = OnceLock::new();
    RUNTIME.get_or_init(|| MemoryRuntime {
        process: std::sync::Mutex::new(MemoryProcessGeneration {
            control: md_web_services::domains::memory_skills::ProcessControl::default(),
            generation: 1,
            reset_prepared: false,
        }),
        telemetry: md_web_services::domains::memory_skills::TelemetryStore::default(),
        usage_accumulator:
            md_web_services::domains::memory_skills::ProviderUsageAccumulator::default(),
        events: std::sync::Mutex::new(std::collections::VecDeque::with_capacity(512)),
        activities: std::sync::Mutex::new(std::collections::VecDeque::with_capacity(512)),
        sequence: std::sync::atomic::AtomicU64::new(0),
        context: std::sync::RwLock::new(std::collections::BTreeMap::new()),
    })
}

#[cfg(feature = "server")]
fn runtime_activity_tail(limit: usize) -> Vec<ActivityEntry> {
    let limit = limit.clamp(1, 1_000);
    memory_runtime()
        .activities
        .lock()
        .map(|activities| {
            activities
                .iter()
                .rev()
                .take(limit)
                .cloned()
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(feature = "server")]
fn provider_activity(event: &ProviderUsageEvent) -> ActivityEntry {
    let mut details = std::collections::BTreeMap::new();
    details.insert(String::from("provider"), format!("{:?}", event.provider));
    details.insert(String::from("model"), event.usage.model.clone());
    details.insert(
        String::from("tokens"),
        event
            .usage
            .input_tokens
            .saturating_add(event.usage.output_tokens)
            .to_string(),
    );
    details.insert(String::from("usd"), format!("{:.6}", event.usage.usd));
    ActivityEntry {
        timestamp_ms: event.usage.timestamp_ms,
        kind: String::from("provider_usage"),
        summary: format!("{} usageを記録", event.usage.agent_id),
        details,
    }
}

#[cfg(feature = "server")]
struct MemoryContextUsageProvider;

#[cfg(feature = "server")]
impl md_web_services::domains::connections::ContextUsageProvider for MemoryContextUsageProvider {
    fn samples(&self) -> Vec<md_web_services::domains::connections::ContextUsageSample> {
        memory_runtime()
            .context
            .read()
            .map(|context| context.values().cloned().collect())
            .unwrap_or_default()
    }
}

#[cfg(feature = "server")]
fn reset_receipt(
    generation: u64,
    phase: MemoryNamespaceResetPhase,
    drained: bool,
    active_processes: u32,
    cleared: bool,
) -> MemoryNamespaceResetReceipt {
    MemoryNamespaceResetReceipt {
        generation,
        phase,
        drained,
        active_processes,
        projections_cleared: cleared,
        event_journal_cleared: cleared,
        caches_invalidated: cleared,
    }
}

#[cfg(not(feature = "server"))]
#[allow(
    dead_code,
    reason = "Dioxus replaces server-function bodies with client stubs on web builds"
)]
fn publish(_kind: MemorySkillsEventKind) {}

#[cfg(feature = "server")]
fn epoch_millis() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or(0)
}

#[cfg(feature = "server")]
fn durable_event_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0_u128, |duration| duration.as_nanos());
    let hex = format!(
        "{:032x}",
        nanos ^ u128::from(SEQUENCE.fetch_add(1, Ordering::Relaxed))
    );
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

#[cfg(feature = "server")]
fn server_error(error: md_web_services::domains::memory_skills::DomainError) -> ServerFnError {
    use md_web_services::domains::memory_skills::DomainError;
    match error {
        DomainError::InvalidInput(message) => ServerFnError::new(message),
        DomainError::Unavailable(message) => ServerFnError::new(message),
        DomainError::NotFound => ServerFnError::new("requested item was not found"),
        DomainError::OutsideManagedRoot => ServerFnError::new("managed item was refused"),
        DomainError::Io(_) => ServerFnError::new("local storage operation failed"),
        DomainError::Database(_) => ServerFnError::new("PostgreSQL operation failed"),
        DomainError::Serialization(_) => ServerFnError::new("stored data could not be decoded"),
    }
}

#[cfg(not(feature = "server"))]
#[allow(
    dead_code,
    reason = "Dioxus replaces server-function bodies with client stubs on web builds"
)]
fn server_only() -> ServerFnError {
    ServerFnError::new("memory service is server-only")
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use md_web_contracts::domains::memory_skills::{
        MemoryNamespaceResetPhase, MemorySkillsEventKind, ToolSpan,
    };

    use super::{
        event_journal, finish_memory_namespace_reset, prepare_memory_namespace_reset,
        process_control, publish, telemetry_store,
    };

    #[tokio::test]
    async fn namespace_reset_reinitializes_process_and_ephemeral_projections() {
        telemetry_store().record_span(ToolSpan {
            agent_id: String::from("agent"),
            session_id: String::from("session"),
            timestamp_ms: 1,
            tool: String::from("test"),
            success: true,
            duration_ms: 1,
            decision: None,
            error: None,
        });
        publish(MemorySkillsEventKind::TelemetryChanged);

        let Ok(prepared) = prepare_memory_namespace_reset().await else {
            panic!("prepare memory reset failed");
        };
        assert!(prepared.drained);
        let Ok(completed) = finish_memory_namespace_reset(true) else {
            panic!("finish memory reset failed");
        };

        assert_eq!(completed.phase, MemoryNamespaceResetPhase::Reinitialized);
        assert!(completed.projections_cleared);
        assert!(telemetry_store().snapshot().spans.is_empty());
        let Ok(events) = event_journal().lock() else {
            panic!("event journal lock failed");
        };
        assert!(events.is_empty());
        assert!(process_control().is_ok());
    }
}
