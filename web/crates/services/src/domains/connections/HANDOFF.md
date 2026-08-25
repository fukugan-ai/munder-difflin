# Connections domain handoff

## Integration points

- Export `contracts/src/domains/connections/mod.rs` as
  `md_web_contracts::domains::connections`.
- Export `services/src/domains/connections/mod.rs` as
  `md_web_services::domains::connections`.
- Export `app/src/components/domains/connections/mod.rs`, import
  `assets/domains/connections.css`, and mount `ConnectionsPanel` at
  `/connections` or the existing Connections tab.
- `ConnectionsPanel` is presentation-only. Its `ConnectionUiAction` callback
  must call Server Functions and refresh `ConnectionsSnapshot` after a write.
- Forward domain `ConnectionEvent` values through the shared SSE/event hub.

### Live integration correction

`web/app/src/server_fns/connections.rs` now owns the complete callable Server
Function surface. The integration owner must:

1. add `mod connections;` to `web/app/src/server_fns.rs`;
2. `pub(crate) use connections::{ ... };` for the functions used by routes;
3. replace the old `connections_snapshot()` local `OnceLock` implementation
   with `connections_domain_snapshot()`; the old static is a different store;
4. map every `ConnectionUiAction` in `/connections` instead of matching only
   `Refresh`.

Export these names from `server_fns.rs`:

- `connections_domain_snapshot`
- `connections_update_slack`, `connections_write_slack_secret`,
  `connections_clear_slack_secret`, `connections_start_slack`,
  `connections_stop_slack`
- `connections_upsert_integration`, `connections_add_integration_template`,
  `connections_write_integration_secret`, `connections_remove_integration`,
  `connections_probe_integration`
- `connections_create_webhook`, `connections_create_default_webhook`,
  `connections_upsert_webhook`,
  `connections_write_webhook_secret`, `connections_rotate_webhook_secret`,
  `connections_remove_webhook`, `connections_start_webhooks`,
  `connections_stop_webhooks`
- `connections_set_context`, `connections_set_context_enabled`
- `connections_set_organisation`, `connections_write_organisation_key`
- `connections_decide_history`, `connections_clear_history`
- `connections_replace_missions`, `connections_set_mission_enabled`,
  `connections_remove_mission`, `connections_upsert_mission`
- `connections_start_broker`, `connections_stop_broker`

Start/stop/probe functions now execute the real runtime only after the explicit
operator action. Slack and webhook listeners bind loopback, validate inbound
credentials, and spawn the Tunnelmole child. Integration probes perform the real
authenticated HTTP request. The broker binds `127.0.0.1` only and forwards with
server-held credentials. A missing Tunnelmole binary/network is reported as
typed `TransportUnavailable`; the local signed listener remains available.

## Server Functions

Recommended names and return shapes:

- `connections_snapshot() -> ConnectionsSnapshot`
- `connections_update_slack(SlackConfigPatch) -> SlackConfigView`
- `connections_write_slack_secret(SlackSecretWrite) -> SlackConfigView`
- `connections_start_slack() / connections_stop_slack() -> ListenerStatus`
- `connections_upsert_integration(IntegrationUpsert) -> IntegrationView`
- `connections_write_integration_secret(id, WriteOnlySecret) -> IntegrationView`
- `connections_probe_integration(id, path) -> ProbeResult`
- `connections_remove_integration(id)`
- `connections_upsert_webhook(WebhookUpsert) -> WebhookView`
- `connections_rotate_webhook_secret(id) -> OneTimeSecret`
- `connections_start_webhooks() / connections_stop_webhooks() -> ListenerStatus`
- `connections_set_context(ContextTriggerConfig)`
- `connections_set_organisation(enabled, mode)` and a separate write-only key call
- `connections_decide_history(id, decision)` / `connections_clear_history(source)`
- `connections_replace_missions(Vec<ScheduledMission>)`

Do not serialize the private secret store or add a secret-read Server Function.
`OneTimeSecret` is only for a newly generated/rotated webhook secret.

## Runtime implementation

- `services/.../connections/runtime.rs` owns bounded HTTP parsing, Slack
  HMAC-SHA256/replay validation, constant-time secret comparison, listener
  lifecycle, and the Tunnelmole child lifecycle.
- `MD_TUNNELMOLE_BIN` can point at an exact executable. Otherwise the runtime
  uses `node_modules/.bin/tunnelmole`, then `tmole` from `PATH`.
- Tunnelmole's random public URL is the free mode. The runtime never configures
  a paid reserved subdomain.
- The loopback broker mints a one-time capability at explicit Start and never
  returns integration credentials. The route must put
  `BrokerStartResult.capability` into the existing one-time-secret UI signal.
- `connections_decide_history` creates the Hive task and sends the God request
  before flipping a pending item to Approved. Failure leaves it Pending.
- Listener callbacks create Hive task/God requests through the injected
  server-side adapters; no secret enters a task, history row, or browser DTO.

## PostgreSQL durability and secrets

The adapter stores one CAS-protected record at
`RecordDomain::Connections / state / main` through the shared PostgreSQL
repository. Its metadata document includes settings, integrations, webhooks,
history (already capped to 500), missions and context scheduler timestamps.
Every successful mutation persists before its Server Function returns.

Namespace reset must call the server-only lifecycle seams around the shared PG
transaction:

1. `prepare_connections_namespace_reset().await` rejects new connection work,
   joins the scheduler, stops all listeners/broker and clears process metadata,
   capabilities and plaintext secrets.
2. Commit (or roll back) the shared PostgreSQL namespace reset.
3. `reinitialize_connections_after_reset().await` releases the reset gate,
   reloads the committed namespace and starts a fresh scheduler in the same
   server process.

The hydration state is deliberately restartable and is not a sticky OnceCell.

Secrets are a separate authenticated encrypted envelope inside that atomic
record. `MD_CONNECTIONS_MASTER_KEY` (minimum 32 bytes) is mandatory before a
secret write and at restart when sealed values exist. Plaintext values never
enter `ConnectionsSnapshot`, serde DTOs, errors, or Debug output.

Server-only consumers such as Voice use:

- `SecretId::Provider(ProviderSecretId::{OpenAi, Groq})`
- `SecretProvider::{get_secret,set_secret,clear_secret,has_secret}`
- `ServerSecret::expose_for_server()` only at the outbound provider call

There is deliberately no secret-read Server Function and these types have no
Serde implementation.

## Environment and routes

- Existing PG environment: `MD_PG_HOST`, `MD_PG_PORT`, `MD_PG_DATABASE`,
  `MD_PG_USER`, `MD_PG_PASSWORD`, `MD_PG_NAMESPACE`, optional `MD_PG_TLS_CA`.
- Slack port is configured in the UI (default 3847). Webhook is 3849 and the
  loopback broker is 3851.
- Public webhook contract: `POST /hooks/<id>` with
  `x-md-webhook-secret`; token-scoped status at `GET /hooks/<id>` with
  `x-md-webhook-token`.
- Slack Events uses the tunnel root POST URL shown by the Slack card.
- The worker integration broker must bind to `127.0.0.1` only and must never be
  exposed through the LAN listener or public tunnel.

## Cross-domain inputs

- Hive/tasks: approved or auto-allowed webhook messages require one idempotent
  task-create + God-request operation and return its task id.
- Agent lifecycle installs one `Arc<dyn ContextUsageProvider>` with
  `install_context_usage_provider`. The 15-second executor reads server-only
  samples via `context_usage_samples`, stamps cadence before dispatch for
  idempotency, and sends due compact/clear messages through Hive/God.
- Event hub: status, history, mission, integration, and Slack-incoming updates.
- Files: Slack attachments must be downloaded server-side and exposed only as
  sandboxed local attachment handles, not arbitrary paths.
- Config/DB: first domain access hydrates PostgreSQL, then restarts desired
  Slack/webhook listeners. Broker capability is intentionally ephemeral, so
  broker desired-state hydration restarts the loopback listener with a fresh
  server-only capability. Capabilities are never persisted or returned during
  restart; the worker orchestrator uses the server-only capability grant API
  when it needs a separately scoped token.

Voice non-secret preferences share this record through
`VoiceDurableSettings` plus `voice_durable_settings()` and
`update_voice_durable_settings()`. The latter persists by CAS and restores the
previous in-memory value on failure. Voice stores USD caps as integer micro-USD
and converts only at its server adapter boundary.

The production `MemoryContextUsageProvider` updates from accepted provider
usage events using their reported context-window size. The scheduler reads its
current server-only samples on every poll; no telemetry sample is serialized
into the Connections DTO.

## Verification status

- Contract secret/validation, restart hydration, encrypted tamper rejection,
  secret absence, mission once-only firing, and runtime SHA/HMAC known-vector
  tests are present. No test starts a listener, tunnel, broker, external probe,
  or PostgreSQL mutation.
- `connections.spec.ts` covers write-only fields, Slack channel/port, custom REST
  editing, broker controls, context editing, organisation key, mission Add/Edit,
  keyboard tabs, and 320px overflow.
- No external network, tunnel, Slack, GitHub, filesystem, or DB mutation was run
  during verification.
