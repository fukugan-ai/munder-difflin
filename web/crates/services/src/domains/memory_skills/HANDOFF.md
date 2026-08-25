# Memory / Knowledge / Skills / Activity / Telemetry / History handoff

This domain is intentionally isolated under `domains/memory_skills`. The integration owner must wire it into the shared module tree, application state, server functions, event stream, and route. No browser/WASM code may receive PostgreSQL credentials, executable paths, or unrestricted filesystem paths.

## Manifest dependencies

Add these workspace dependencies before compiling the domain:

```toml
serde.workspace = true
serde_json = "1"
```

Enable both only in `md-web-services`; contracts already use workspace `serde`. No JavaScript state library is needed.

If server-side file upload uses streaming multipart, choose the Dioxus/Axum-compatible multipart adapter in the integration layer. Do not buffer an unbounded upload. The service rejects staged files over 64 MiB.

## Module wiring

Contracts:

```rust
pub mod domains {
    pub mod memory_skills;
}
```

Services:

```rust
pub mod domains {
    pub mod memory_skills;
}
```

App components:

```rust
pub(crate) mod domains {
    pub(crate) mod memory_skills;
}
```

Import `assets/domains/memory_skills.css` after the existing token and application styles. Render `MemorySkillsWorkspace` from the existing dashboard or a dedicated local route; do not replace the shell.

## Required server functions / routes

These adapters now exist in `app/src/server_fns/memory.rs`; do not replace them
with route-local defaults. Re-export its callable functions from
`app/src/server_fns.rs`. The streamed upload also needs this explicit Axum route:

```rust
use dioxus::server::axum::routing::{get, post};

Router::new()
    .route("/ws/terminal", get(server_fns::terminal_socket))
    .route(
        "/api/memory-skills/knowledge/upload-multipart",
        post(server_fns::knowledge_upload_multipart),
    )
```

`Memory` must load `memory_skills_snapshot` with `use_resource`, populate every
`MemorySkillsWorkspace` field from it, and use the following action calls before
restarting the snapshot resource:

- search: `memory_semantic_search(MemorySearchRequest { query, wing: None, results: 20 })`;
- mine/reflect: `memory_mine(selected_agent_id)` / `memory_reflect(selected_agent_id)`;
- knowledge: `knowledge_search(query, 100)`, `knowledge_upload(request)` (or the
  multipart route for streamed uploads), `knowledge_remove(document_id)`;
- skills: `skills_catalog(true)`, `skills_install(entry)`, `skills_uninstall(managed_id)`;
- activity/history: `activity_tail(200)`, `history_query(HistoryQuery { ... })`;
- events: poll `memory_events(last_sequence)` and restart only affected resources,
  or forward these ordered envelopes into the shared event hub.

The route currently supplies static disabled statuses, empty vectors and no-op
callbacks for every one of these actions. Until that block is replaced, all
domain services remain unreachable despite the callable adapters being present.

Use Dioxus Server Functions for commands and snapshots. Suggested logical surface:

- `memory_status()`
- `memory_read(agent_id)`
- `memory_text_search(query)`
- `memory_semantic_search(request)`
- `memory_wake_up(wing)`
- `memory_mine(agent_id)`
- `memory_reflect(agent_id)`
- `knowledge_status()` / `knowledge_list()` / `knowledge_get(id)`
- `knowledge_search(query, limit)`
- `knowledge_upload(upload_token, metadata)`
- `knowledge_remove(id)`
- `skills_local()` / `skills_catalog(force)`
- `skills_install(catalog_entry)` / `skills_uninstall(managed_skill_id)`
- `activity_tail(limit)`
- `telemetry_snapshot()` / `telemetry_spans(agent_id)`
- `history_add(entry)` / `history_query(request)`
- `cost_totals()`

The browser must send the opaque `managed_id` returned by `SkillService::list_local`, not a free-form path. `SkillService::uninstall` resolves and canonicalizes that ID server-side and refuses bundled roots.

Knowledge upload must stream browser bytes into an owner-private server staging directory, then pass the staged path plus the sanitized browser filename to `KnowledgeService::ingest_uploaded_file`. Always remove the staging file after success or failure. Never interpret a client-supplied absolute path.

## Live event integration

Extend the shared ordered `EventEnvelope` stream with domain variants or a generic domain payload:

- activity appended;
- telemetry usage changed;
- telemetry tool span appended;
- memory index changed;
- knowledge corpus changed;
- history row appended.

Sequence and reconnect ownership remain in the shared event hub. This domain only owns snapshot/state production. Activity events are metadata-only; never include message body, prompt, content, credentials, or raw environment values.

## PostgreSQL schema requirements

Reuse migration `db/migrations/001_initial.sql` unchanged:

- `munder_difflin.command_history`
  - unique `(namespace,event_id)` for retry idempotency;
  - indexes `(namespace,agent_id,occurred_at DESC,id DESC)` and `(namespace,occurred_at DESC,id DESC)`;
  - server writes trimmed non-empty prompt text;
  - `ILIKE` search escapes `\\`, `%`, and `_` and clamps limit to 1000.
- `munder_difflin.cost_ledger`
  - append cumulative usage samples only;
  - non-negative token/cost checks;
  - lifetime totals segment a session when cumulative USD resets;
  - tool spans remain in the bounded RAM ring and are not inserted here.

Construct `HistoryRepository` from the already verified application `PgPool` and `MD_PG_NAMESPACE`. Do not create a second pool or read connection secrets in WASM. Existing startup must verify schema version before exposing write functions.

## Server-only filesystem / CLI inputs

Construct services from canonical server-owned paths:

- `MemoryService.hive_root`: configured harness home, containing `hive/` and `palace/`.
- `MemoryService.semantic_cli`: resolved absolute `mempalace` executable or `None`.
- reflect summarizer: resolved absolute Claude CLI.
- `KnowledgeService.root`: configured knowledge root or `<app-data>/knowledge`.
- `ActivityService.log_path`: `<harness-home>/hive/log.jsonl`.
- `SkillService.roots`: explicit user/project/bundled roots.
- `SkillService.install_root`: canonical user Claude skills root.

Run blocking filesystem and `std::process::Command` methods behind the server runtime's bounded blocking executor. Add a 120-second process timeout at that integration boundary and terminate the child on expiry. Never run CLI/process functions on the WASM target.

Environment values consumed by the shared bootstrap remain:

- `MD_PG_HOST`, `MD_PG_PORT`, `MD_PG_DATABASE`, `MD_PG_USER`, `MD_PG_PASSWORD`, `MD_PG_NAMESPACE`, optional `MD_PG_TLS_CA`;
- configured harness home / knowledge root from the server's local config;
- resolved CLI paths, not browser input.

`MemorySkillsHost::from_public_config` is the canonical server constructor.
`app/src/server_fns/memory.rs::host()` loads that `PublicConfig` through the
shared process-lifetime `PgPersistenceRuntime`; do not restore env-only
`MD_HARNESS_HOME` resolution in request handlers. The env constructor remains a
compatibility/test boundary only. History and cost calls use
`super::persistence_repository()` and must never create a per-request pool.

PTY/composer and CLI producers call the typed server hooks after their owning
write succeeds:

- `record_prompt_accepted(HistoryAppend)`;
- `record_cli_usage(AgentUsageSample)` (RAM snapshot plus durable `CostAppend`);
- `record_tool_span(ToolSpan)`;
- `record_activity_event(ActivityEntry)` after the metadata-only JSONL append.
- `record_provider_transcript(provider, source_event_id, agent_id, session_id,
  timestamp_ms, payload_json)` from server-only Claude/Codex/Gemini hook,
  transcript or exit producers; or `record_provider_usage_event` when the
  producer already owns a sanitized typed event.

Re-export those server-only hooks for their producer modules. They publish a
post-commit domain event; consumers must not call browser-facing endpoints to
simulate producer state.

Provider transcript normalization drops prompt/content fields, caps raw event
size at 2 MiB, accepts only token/cost/model/context/tool metadata, and derives a
stable UUID from provider + session + source event ID. The adapter inserts that
UUID into `cost_ledger` first. Only a newly inserted row updates RAM telemetry,
tool waterfall, metadata-only activity and context pressure, so retries do not
double-count cost or spans. Producers must reuse their canonical provider event
ID on transcript/exit retries; never substitute a fresh UUID. Claude
`modelUsage`, Codex `usage`, and Gemini `usageMetadata` shapes are supported.
Claude model totals are treated as cumulative; Codex/Gemini turn deltas pass
through the bounded retry-aware accumulator before ledger insertion so the
existing cumulative-ledger lifetime query remains correct. A process restart
starts a new lower cumulative segment, which the ledger query already treats as
a reset segment.
Call `install_memory_context_usage_provider()` once during server startup so
Connections context rules consume the live projection.

Both accepted prompt paths must call `record_prompt_accepted`: durable PTY queue
already supplies its message UUID, while direct PTY input must supply the
accepted input event UUID, agent ID, cwd, text and timestamp. Do not record raw
keystrokes before the PTY admission/control gate accepts them.

The process owner must re-export and invoke `cancel_memory_processes()` at the
start of graceful shutdown, before waiting for in-flight requests. MemPalace,
reflect summarizer and both skill clone paths share that cancellation handle and
a 120-second deadline; timeout/shutdown kills the child and refuses new starts.

Namespace reset is two-phase. Before the shared PostgreSQL transaction, call
`prepare_memory_namespace_reset().await`, require `drained == true` and
`active_processes == 0`, and do not start the transaction otherwise. After the
transaction attempt, always call `finish_memory_namespace_reset(committed)`:
`true` clears telemetry projections and the memory event journal and installs a
fresh process-control generation; `false` reopens the runtime without claiming
the projections were cleared. This prevents the irreversible shutdown control
from breaking onboarding in the same server process. Config and skill
assignments are loaded per call, and history/cost operations use the shared
repository, so there are no domain-owned PG pools or cache handles to retain.

Knowledge ingestion/removal uses one process-wide writer lock. It prepares and
fsyncs a replacement index, renames the document/quarantine, then atomically
commits the index; a failed index commit rolls the document rename back. Do not
write `index.jsonl` anywhere else.

`memory_graph()` projects real document/tag nodes and edges from the knowledge
corpus. `telemetry_waterfall(agent_id)` projects real timestamp offsets and
durations instead of duration-only bars. Route integration should populate the
defaultable `memory_graph` workspace prop and refresh it after
`KnowledgeChanged`; tool waterfalls refresh after `TelemetryChanged`.
`knowledge_get(id)` populates `knowledge_detail` and `on_knowledge_get` in the
workspace. `memory_wake_up(agent_id)` is distinct from semantic search and mine;
wire `on_memory_wake_up` to it.

The concrete host adapter additionally reads `MD_HARNESS_HOME` (required,
absolute), and optional absolute `MD_APP_DATA_ROOT`, `MD_KNOWLEDGE_ROOT`,
`MD_PROJECT_ROOT`, `MD_MEMPALACE_BIN`, `MD_SUMMARIZER_BIN`. Feature flags are
`MD_MEMORY_ENABLED`, `MD_KNOWLEDGE_ENABLED`, and `MD_MEMORY_MODEL` (`minilm` or
`embeddinggemma`).

## Cross-domain inputs

- Agent lifecycle provides agent IDs and the canonical harness root.
- PTY/composer calls `history_add` after a prompt is accepted.
- Telemetry ingestion calls `TelemetryStore::record_usage` and `HistoryRepository::append_cost` for usage; it calls only `TelemetryStore::record_span` for tool results.
- Voice enabled/model/cost-cap durability remains owned by the Voice/config and
  Connections secret seams; memory telemetry must consume their post-commit
  typed usage event rather than duplicating voice configuration.
- The shared `PERSISTENCE` runtime must not retain an initial `Err` forever.
  Retry opening it on a later health/config request and refresh health from the
  current runtime state; this domain continues to use only
  `persistence_repository()` and does not create a fallback pool.
- Shared event hub publishes post-commit events. Do not announce a history/cost write before PostgreSQL confirms it.
- Config domain supplies semantic-memory enabled/model and knowledge enabled/root values.
- Upload adapter supplies staged server paths; browser paths are never accepted.
- Shell/router supplies the workspace mount point and current agent selection.

## Base skills, onboarding, and role assignment

The old `openai/skills` repository is deprecated and is not used. When
`MD_ENABLE_OFFICIAL_SKILL_SOURCES=true`, explicit catalog refresh uses the
project-scoped OpenAI Skills API (`GET /skills`, version listing, and bounded
version/content download) through the hydrated server-only
`SecretProvider::OpenAi` key. It caches only format-compatible `SKILL.md`
instructions plus API provenance, resolved version, license and compatibility;
the key is never serialized or returned. No API call occurs on initial page
load. The separate `https://github.com/openai/plugins` source remains labeled as
the OpenAI GitHub plugin marketplace and uses its
`.agents/plugins/marketplace.json` layout. The official shared Agent Skills
repository
`https://github.com/anthropics/skills` and its
`.claude-plugin/marketplace.json`. Optional
`MD_ENABLE_ANTHROPIC_PLUGIN_MARKETPLACE=true` adds
`anthropics/claude-plugins-official`. Third-party Claude repositories are never
defaults; add them explicitly through `MD_BASE_SKILL_SOURCES_JSON`.

Configured source objects contain `id`, `name`, `kind`, HTTPS GitHub
`repository`, `reference`, `official`, and optional `token_env`. Private GitHub
tokens are read only from the named server environment variable and passed to
Git through an environment-backed HTTP header; tokens never enter DTOs, argv,
cache metadata, logs, or WASM. Catalog refresh only scans standard `SKILL.md`
instructions and marketplace-declared trees. It does not execute vendor scripts.
Provenance stores repository, commit SHA, relative path, license filename and
Codex/Claude/shared-Agent-Skills compatibility.

The shared refresh adapter must hydrate Connections only after the user presses
refresh, call synchronous Git refresh on the bounded blocking executor, then
call `refresh_openai_project_skills` with the server-only secret when the API
source is enabled. Tests use mock HTTP only; never make a live Skills API call.

Mount `BaseSkillsOnboardingPanel` in the existing onboarding flow. Exact calls:

1. `base_skills_catalog(false)` displays cached sources and provenance.
2. The user explicitly checks skill rows and the confirmation checkbox.
3. Refresh is only `base_skills_catalog(true)`; tests and initial page load must
   not perform network calls.
4. Install is only
   `base_skills_install(BaseSkillSelectionRequest { skill_ids, confirmed: true })`.
5. Load `base_skill_assignments()`; seed with
   `TeamSkillAssignments::minimum_software_team()` and persist through
   `save_base_skill_assignments`.

The canonical minimum template is Aria as orchestrator with
`aria-orchestration`, `graph-engineering`, `project-documentation`; one
implementer with `local-development`, `web-project-standards`; and one verifier
with `perfectionist-reviewer`. `specialists_on_demand=true`; a
`DomainSpecialist` assignment must include `task_condition` and is injected only
when the spawn task domains contain that condition. Our local-development and
web-project-standards copies remain authoritative; external catalogs cannot
replace them. Compatible optional development bases include Anthropic's
`webapp-testing`, `frontend-design`, and `skill-creator`/skill-development
instructions after explicit selection.

Before each team spawn, the server owner must call
`assigned_skill_injection(agent_id, task_domains)`. Pass only the returned
canonical paths and instruction texts into that agent's prompt/environment.
Never inject the entire installed directory, another role's skills, unselected
catalog instructions, or vendor scripts. The assignment JSON is written
atomically below `<MD_APP_DATA_ROOT>/base-skills/assignments.json`; installed
skills and provenance remain below the same server-owned root.

## Remaining adapter work

`SkillService::install_from_staging` intentionally owns only validation and atomic installation. The integration adapter must fetch a selected catalog GitHub source into a private bounded staging tree, without following symlinks/submodules, then call it. Keep the current catalog limits: at most 60 files, 2 MiB total, depth 5.

`KnowledgeService` currently indexes native text/code/sheet files and caption text for binary/image/PDF artifacts. If parity requires PDF/image extraction, add it as a server-side extractor adapter before `ingest_uploaded_file`; preserve the original artifact and the 5 MiB indexed-text cap.

## Verification

After integration wiring:

```bash
cargo fmt --all -- --check
cargo test --workspace --offline --locked
cargo clippy --workspace --all-targets --all-features --offline --locked -- -D warnings
cargo check -p md-web-app --target wasm32-unknown-unknown --no-default-features --features web --offline --locked
```

Then run `web/e2e/memory_skills.spec.ts` against the fullstack server at the configured 5000-series port.
