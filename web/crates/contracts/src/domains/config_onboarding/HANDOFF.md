# Config / onboarding domain handoff

This directory owns browser-safe contracts only. The integration owner must wire
the sibling service and component modules into their existing root `mod.rs`, route,
and Server Function graph.

## Required module wiring

- `md-web-contracts`: add `pub mod domains` if absent, and expose
  `domains::config_onboarding`.
- `md-web-services`: expose `domains::config_onboarding` only under the server
  build. No service item is needed in WASM.
- `md-web-app`: register
  `components::domains::config_onboarding::ConfigOnboardingPanel` and serve it at
  `/onboarding` when `PublicConfig.onboarding_complete == false`, otherwise at
  `/settings`.
- The component imports `/assets/domains/config_onboarding.css`; keep the existing
  `tokens.css` loaded before it.

## PostgreSQL adapter contract

`ConfigRepository` is the adapter interface. `PgPersistenceRepository` now
implements it through `load_app_config` / `write_app_config`, and
`connect_from_environment()` uses the same PostgreSQL namespace authority as the
existing durable store. Until a CA-backed SQLx configuration is added, the
connector rejects non-loopback hosts instead of silently using plaintext TCP.

Recommended logical row (migration owned elsewhere):

- one configuration row per `MD_PG_NAMESPACE`;
- `namespace text primary key`;
- `revision bigint not null` used for compare-and-swap;
- public configuration fields as typed columns or one validated `jsonb` payload;
- secret values in the existing server-only secret store, never in the public
  payload; `SecretPresence` is derived with `EXISTS`/non-null checks only;
- `updated_at timestamptz not null` for operator diagnosis.

`save_if_revision(expected, config)` must be one transaction with a statement
equivalent to `UPDATE ... SET ..., revision = revision + 1 WHERE namespace = $1
AND revision = $2 RETURNING revision`. Zero returned rows maps to `Conflict`.
Do not retry a stale full snapshot. Use a qualified schema or constrained
`search_path`, finite statement/lock timeouts, and the normal application role.

No migration or database mutation is included in this slice.

## Server Functions / events

Implemented in `web/app/src/server_fns/config.rs` (the integration owner must add
`mod config;` and the corresponding `pub(crate) use config::{...};`):

- `config_bootstrap`, `config_get`, `config_patch`, `onboarding_finish`,
  `onboarding_confirm_team`, `onboarding_probe_paths`
- `config_set_agent_token_cap`, `config_change_home`, `config_write_provider_key`
- `tools_status`, `update_current`, `update_check`
- `app_capabilities`, `config_app_info`
- `floor_create`, `shutdown`, `reset_all`

`update_check` performs one unauthenticated, read-only GitHub Releases API call
against `MD_RELEASE_REPO`; it has no GitHub write or install path. `shutdown`
validates the observed PTY count and delegates accepted shutdown to the shared
application lifecycle owner. `reset_all` delegates to the bounded PostgreSQL
namespace reset below.

Onboarding is a durable saga. `PublicConfig.onboarding_phase` is `Draft`,
`TeamStarting`, `Complete`, or `RepairRequired`; `team_initialized` is persisted.
`onboarding_complete` is true only with `Complete`, `team_initialized=true`, and
exactly one persisted assignment for Aria, Implementer and Verifier. All route,
Office and Settings gates must call `config.onboarding_ready()` /
`config.requires_onboarding()` instead of reading the legacy boolean alone.

`onboarding_finish` returns a team-start `FinishOnboardingResult`. The shared
route/team adapter must:

1. set `finish_pending=true` before awaiting it and pass that state plus any
   typed receipt/error back to `ConfigOnboardingPanel`;
2. leave the wizard on Reliability when the call fails (do not restart the
   bootstrap resource and do not show Done);
3. accept only a persisted `TeamStarting` receipt matching the current PG
   snapshot; do not require `onboarding_complete` at this phase;
4. spawn/idempotently observe all three roles using the canonical selected
   `workspace_cwd` from `result.aria.cwd` (never `harness_home`);
5. after all three agents and role assignments are observed, call
   `onboarding_confirm_team(ConfirmTeamInitializedRequest { expected_revision:
   result.config.revision, initialized_roles: [Aria, Implementer, Verifier] })`;
6. navigate to Office only from `ConfirmTeamInitializedResult`. A spawn or
   confirm failure remains on onboarding with the cached team-start receipt so
   retry does not repeat the configuration CAS.

`harness_home` owns application/harness storage. `workspace_cwd` is a separate,
required path and must equal one of the canonical `registered_repos`; the server
path probe validates all three together. A legacy snapshot claiming complete
without confirmed team/assignments normalizes in memory to `RepairRequired`,
renders onboarding (starting at Team), and the next successful configuration
write returns `OnboardingRepairReceipt`. It must never route directly to Settings.

The route must load `skills_local() -> Vec<memory_skills::LocalSkill>` and pass
that public DTO list as `base_skills` to `ConfigOnboardingPanel`. The chosen
`managed_id` is submitted; the server resolves it again against the current
`skills_local()` result and returns three `RoleSkillAssignment` values:

- Aria: `aria-orchestration`, `graph-engineering`, `project-documentation`;
- Implementer: `local-development`, `web-project-standards`;
- Verifier: `perfectionist-reviewer`.

The selected base skill is additionally assigned to all three. Missing mandatory
standards or a stale/unresolved selected `managed_id` blocks completion.
The resolved assignments are stored in `PublicConfig.onboarding_role_skills`
through the same PostgreSQL CAS write and echoed in `FinishOnboardingResult`;
only public `LocalSkill` metadata is persisted.
`database-specialist` is not a default fourth role; task/project scanning may add
it only when database work exists. Do not duplicate memory_skills catalog parsing
inside config_onboarding.

## Runtime settings, reset, BYOK, floors

- `config_set_agent_token_cap` and `config_change_home` perform a PostgreSQL CAS
  and return `ConfigRuntimeReceipt`. The shared runtime owner must consume
  `RuntimeReinitialize::AgentBudgets` / `HarnessHome` and reinitialize its cached
  budget/home projections before reporting the UI action complete.
- Exact component hooks are `on_set_agent_token_cap`, `on_change_home`, and
  `on_provider_key`. Route these respectively to `config_set_agent_token_cap`,
  `config_change_home`, and `config_write_provider_key`. Do not show a runtime
  settings write as complete until its `RuntimeReinitialize` consumer succeeds.
- `reset_all(ResetNamespaceRequest)` requires the exact phrase
  `RESET <MD_PG_NAMESPACE>` and deletes only that namespace across all owned
  tables in one transaction. Authority comes from the existing
  `PgPersistenceRepository`: `reset_bound_namespace()` reuses its shared pool
  and bound namespace under a transaction advisory lock; it does not reread PG
  credentials, open a second pool, or accept a database namespace from the
  caller. The server adapter validates before draining, rejects new requests,
  prepares Memory/Connections, shuts down PTYs, commits the reset, then clears
  PTY/Office projections and reinitializes Memory/Connections/Hive before
  returning the receipt. The route navigates to `/onboarding` only from a
  successful receipt.
- BYOK uses `ProviderKeyWrite` / `WriteOnlyProviderKey`; Debug is redacted and
  the UI clears the password field after emitting it.
  `config_write_provider_key(ProviderKeyWrite) -> SecretPresence` is the exact
  server-function export. It accepts typed `openai` / `groq` identities, writes
  through the hydrated shared `SecretProvider`, seals the broker state with
  `MD_CONNECTIONS_MASTER_KEY`, and acknowledges only after persistence. A
  persistence failure restores the previous in-memory value. Plaintext never
  enters `PublicConfig`, PostgreSQL config JSON, or a response; bootstrap/get
  overlay only presence booleans and provider IDs.
- Multi-floor is intentionally reported `NotApplicable` and `multi_floor`
  defaults false. `floor_create` returns unavailable until a real `/floor/:id`
  route and floor-scoped runtime/PG identity are connected; do not present the
  old process-local allocator as available.

The server expands `~` against its own `HOME`, canonicalizes existing
directories, and confines them to `MD_ONBOARDING_ROOTS` (path-list) or `HOME`
when unset. Registered paths must contain `.git`. The browser never sends a
client-PC path capability.

Publish config/update/lifecycle changes through the shared ordered
`EventEnvelope`; do not create a second event transport. The app component uses
event handlers so integration can call those Server Functions without adding a
JavaScript state store.

## Environment

- `MD_RELEASE_REPO` optionally overrides the release source in `owner/name`
  form. Default: `fukugan-ai/munder-difflin`.
- `chaitanyagiri/munder-difflin` is explicitly rejected by
  `resolve_release_repository` so this Web branch cannot accidentally treat the
  original project as its update source.
- Existing `MD_PG_*` variables and `MD_PG_NAMESPACE` remain authoritative.

## Cross-domain inputs

- Memory domain supplies the resolved MemPalace executable path to
  `probe_host_tools`; passing `None` reports it missing.
- Agent/PTY domain supplies the current running-terminal count to
  `shutdown_decision` and performs the actual graceful teardown after acceptance.
- Filesystem domain validates/creates/moves `harness_home` under its existing
  server-side path boundary before `onboarding_finish` or a home-changing patch is
  persisted.
- Event-hub domain broadcasts successful writes and update/lifecycle transitions.
- Secret/integration domain converts server-only secret records to
  `SecretPresence`; plaintext must never enter `PublicConfig`.

## Explicit Web equivalents / N/A

- window bounds, Electron titlebar/menu, OS settings deep-links, login-item API,
  native installer/self-update, and display-sleep blocking are N/A;
- login-time server start is external setup (for example systemd);
- notifications are browser-restricted and may require a secure context;
- multi-window is a server floor ID plus URL opened in another tab/PC;
- browser disconnect never kills a PTY; only the explicit shutdown protocol may
  stop server work;
- folder selection refers to the server host, not the client PC.

## Verification after wiring

Run formatter, contracts/services/app host tests, WASM check, Clippy with warnings
denied, then `web/e2e/config_onboarding.spec.ts` against the live `/settings`
route. The E2E covers secret redaction, explicit N/A copy, focus visibility, and
320/375/414/768 px overflow.
