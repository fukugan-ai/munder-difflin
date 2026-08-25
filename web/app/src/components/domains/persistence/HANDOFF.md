# PostgreSQL Web parity persistence handoff

## Delivered scope

- Forward-only migration: `db/migrations/002_web_parity.sql`
- Browser-safe contracts: `web/crates/contracts/src/domains/persistence/**`
- SQLx repository: `web/crates/services/src/domains/persistence/**`

Migration 002 is additive. It keeps migration 001's `kv`, `command_history`, and
`cost_ledger` intact and adds:

- `web_app_config`: one browser-safe config document per namespace, updated by CAS.
- `web_durable_records`: lossless versioned JSONB records for `tasks`, `hive`,
  `connections`, `triggers`, and `floors`.
- `web_event_stream_heads` + `web_event_replay`: durable, namespace-local,
  per-stream sequence allocation and idempotent replay.
- `web_trigger_history`: append-only trigger history, newest-first, pruned to 500
  rows by the repository in the insert transaction.

No plaintext secret column is present. Connection/provider/webhook/Slack secrets
must remain in the server's chosen encrypted secret provider. `payload_json` is
for browser-safe metadata only; the repository cannot infer whether arbitrary
JSON supplied by an integration caller contains a secret.

## Integration status and remaining changes

This slice did not edit manifests or root module registries. The current
worktree now registers both persistence domain modules through integration-owned
changes.

Completed in the shared DB runtime:

- `web/crates/services/src/persistence.rs` and `src/main/db.ts` now require schema
  version 2.
- `tools/db-migrate.cjs` discovers contiguous numbered migrations, validates
  transaction boundaries, and applies 001 then 002 under one global session lock.
- `src/main/db.ts::resetNamespace()` drains writes and deletes migration 001 and
  002 namespace data in one transaction on the client that owns the process
  namespace lock; deleting stream heads cascades replay rows.
- `web/crates/contracts/src/domains/mod.rs` and
  `web/crates/services/src/domains/mod.rs` expose their persistence modules.

The following application wiring remains outside this slice:

1. Re-export persistence types from shared crate roots only if the application
   requires shorter import paths.
2. Wire one process-lifetime `PgPool` and validated
   `md_web_contracts::domains::persistence::Namespace` into
   `PgPersistenceRepository`.
3. Replace the duplicate history/cost SQL in
   `services/src/domains/memory_skills/history.rs` with delegation to this
   repository, or keep exactly one implementation as canonical. Do not run both
   producers for one logical event.
4. Implement `config_onboarding::ConfigRepository` as a thin adapter over
   `load_app_config` / `write_app_config`: serialize the typed `PublicConfig`,
   remove or ignore its derived `SecretPresence` on write, and rehydrate secret
   presence from the server-only secret provider on read.
5. Replace `hive_tasks::event_hub::EventHub` as the sequence authority, or change
   it to broadcast an event only after `append_replay_event` returns. PostgreSQL's
   returned sequence must be the `HiveEventEnvelope.seq`; two independent
   allocators would break reconnect replay.

No additional crate dependency is required by this domain. JSON is bound as text
and cast to `jsonb` by PostgreSQL. Integration layers may use `serde_json` to
serialize typed domain contracts before calling the repository.

Keep migration and runtime roles separate. The migration role needs the provider-
specific ability to create/alter the `munder_difflin` schema objects. The runtime
role needs `USAGE` on the schema, `SELECT/INSERT/UPDATE/DELETE` on runtime tables,
and sequence privileges for migration 001 identity columns; it does not need DDL
or cross-namespace access by application contract. Migration 002 does not add
row-level security, so a compromised runtime role can technically query every
namespace in the same tables. This is acceptable only for the selected local/
self-hosted, single-trust-boundary deployment; a hostile multi-tenant service
requires RLS or an API boundary before release. Actual grants are deployment-owned
and **NOT VERIFIED**.

The selected topology is direct SQLx access from the trusted Web server process,
never from browser/WASM code. It keeps ownership in the existing service crate
and reuses the configured pool with low coupling. A separate persistence API is
preferable for a distributed or hostile multi-tenant deployment because clients
do not receive database credentials, but it adds an independently operated
service and protocol now. Keeping SQLite was rejected because the requested
replacement and shared Web/Electron durability require PostgreSQL.

The current workspace pins SQLx with `tls-none`. That is suitable only for the
already-enforced local PostgreSQL mode. A remotely hosted database is not release
ready until an integration owner selects a SQLx TLS feature, validates the server
certificate against an operator-provided CA, and verifies secret-safe child
process inheritance. Never embed a production connection string or password in
the Web/WASM bundle.

The current worktree's integration-owned PTY and filesystem command builders
remove `MD_PG_PASSWORD`, `MD_PG_HOST`, `MD_PG_PORT`, `MD_PG_DATABASE`,
`MD_PG_USER`, `MD_PG_NAMESPACE`, and `MD_PG_TLS_CA` from every child command.
Preserve those focused inheritance tests; do not replace this with mutation of
process-global environment after worker threads start.

`PgPersistenceRepository` owns only a reference-counted `PgPool` clone. The
server bootstrap owns the pool and connection policy. Shutdown must stop request
admission, await in-flight repository futures, then call `PgPool::close`; dropping
one repository must not close a pool shared by other domains. Configure finite
connect/acquire/statement/lock timeouts at bootstrap. Do not add a transaction
advisory lock for reset on a second pooled connection while the Electron runtime
already holds the same namespace session lock; first quiesce the owning runtime.

## Record mapping

Use one stable `(domain, kind, record_id)` per logical object:

| Domain | Suggested kind | Record id | Payload |
|---|---|---|---|
| `tasks` | `card` | task id | lossless `HiveTask`, including unknown fields |
| `hive` | `registry`, `message`, `control` | stable object/message/agent id | durable coordination snapshot |
| `connections` | `slack_public`, `integration` | singleton or integration id | non-secret metadata only |
| `triggers` | `webhook`, `organisation`, `context`, `mission` | configured id | non-secret trigger/schedule metadata |
| `floors` | `session` | `floor-<digits>` | label, routing and lifecycle metadata |

`web_durable_records` is not an invitation to mix write authorities. During
transition, nominate one producer for each kind. A filesystem-to-PostgreSQL
backfill must preserve complete task JSON and stop before authority flip if any
unknown field is lost. Migration 002 does not delete or rewrite filesystem data.

## PTY durable facade integration

Create exactly one `PgPersistenceRuntime::from_environment()` during server
bootstrap and retain it in process-lifetime server state. Obtain cheap
`runtime.repository()` handles for adapters. Missing/invalid/unreachable/schema
errors are explicit and contain no connection values. Stop request admission and
await repository futures before `runtime.close().await`; never open a pool per
Server Function.

The typed facade uses `RecordDomain::Floors` with kind `agent` or
`terminal_queue` and stable record id `<floor_id>:<agent_id>`:

- Startup: `list_floor_agents(floor_id, limit)`, then
  `load_terminal_queue(floor_id, agent_id)` for each hydrated agent. Partition
  active/archive/restorable indexes from the persisted `AgentStatus`.
- Spawn/restore: start and validate the process first, then
  `upsert_floor_agent(FloorAgentWrite)`. If persistence fails, terminate the new
  process so no untracked live agent is reported. Revision zero creates; later
  calls use the last returned revision.
- Administrative transition without a live process:
  `archive_floor_agent(FloorAgentRevision)` or
  `mark_floor_agent_restorable(FloorAgentRevision)`. Restorable rejects
  orchestrator/assistant roles and every transition clears stale `pty_id`.
- Enqueue: call `enqueue_terminal_message(TerminalQueueEnqueue)` with a stable
  server-generated message id. An identical-id retry returns the committed queue;
  id reuse with different content conflicts.
- Delivery: write message/instruction and carriage return to the PTY first, then
  call `acknowledge_terminal_message(TerminalQueueHeadMutation)`. On a failed
  write pair call `record_terminal_failure`; its third failure returns the dropped
  message. When a delivery-time precondition is false call
  `drop_terminal_message`. On revision conflict reload the queue and re-decide;
  never blindly retry a stale mutation.
- Explicit kill and natural exit: after process teardown call
  `persist_agent_exit(NaturalExitWrite)` for both paths. It locks the floor event
  stream, checks the stable UUID, CAS-transitions the agent, clears the complete
  terminal queue, appends the ordered replay event, and advances the stream head
  in one transaction. Retry an ambiguous result with the same UUID and identical
  request to receive the existing `NaturalExitReceipt`; publish using its
  `event_sequence`. Do not separately call archive/queue clear/replay for that
  exit.

Terminal delivery is necessarily at-least-once across a crash between successful
PTY writes and durable acknowledgement. After an acknowledgement database error,
stop that agent's automatic delivery loop and surface recovery instead of
immediately sending the same input again.

## Query and transaction contract

- Every query is schema-qualified and binds `namespace` as its first ownership
  predicate.
- Config and durable record writes use revision zero for create, then exact
  compare-and-swap. A stale or ambiguous caller receives `RepositoryError::Conflict`.
- History and cost writes require a caller-generated stable UUID. `ON CONFLICT`
  makes retry after an ambiguous commit idempotent.
- Lifetime cost reconstruction orders by ledger `id`, matching append order; a
  USD decrease greater than `1e-9` starts a new process-reset segment.
- Replay append locks one `(namespace, stream)` head row, checks `event_id` while
  holding the lock, then inserts the event and advances the head in one transaction.
  A retry returns the existing event without consuming another sequence.
- Trigger-history append takes a namespace transaction advisory lock, inserts
  idempotently, prunes rows older than the newest 500, and commits once. Clear
  takes the same lock so it cannot race an append in the same namespace.
- JSONB and UUID casts are database-enforced. Invalid values return the generic
  repository database error; browser responses must not expose its source detail.

Default `READ COMMITTED` is sufficient for single-statement CAS and the explicit
row/advisory-locked transactions above. Retry only whole idempotent operations for
SQLSTATE `40001` or `40P01`, with a finite backoff. Never retry an operation with
a newly generated event id.

## Migration and rollback boundary

The intended operator command is:

```text
node tools/db-migrate.cjs
```

The migration role supplies `MD_PG_*` through the existing secret-handling path.
The command is proposed only; it was **NOT RUN** by this worker.

Existing SQLite history/KV import remains an explicit, one-shot operator action
after schema migration:

```text
node tools/db-import.cjs --source-id <immutable-id> --sqlite </absolute/read-only/source.db>
```

Use `--cost-jsonl` instead of `--sqlite` for a legacy cost ledger. The importer
preflights the source without modifying it, fingerprints the content, uses stable
event ids, records progress in `legacy_imports`, and safely reconsiders already
written rows after interruption. It does not delete the source or overwrite a
different fingerprint. It imports migration 001 KV/history (or cost) only; it
does **not** infer `web_app_config` from arbitrary KV or import filesystem Hive
records into migration 002. Those promotions require a separate typed, explicit
backfill with record-count/hash comparison before authority flip.

Migration 002 uses one transaction and a transaction-scoped advisory migration
lock. Re-running it is idempotent through `IF NOT EXISTS` and the version receipt.
Before version 2 is applied, rollback is simply application rollback. After Web
parity writes begin, schema rollback is not supported: stop writers and fix
forward. Do not drop tables or delete namespace data automatically.

## Verification and unknowns

Verified locally without a database connection:

```text
node --test test/db-postgres.test.cjs
npm run typecheck:node
cargo test --manifest-path web/Cargo.toml -p md-web-contracts -p md-web-services --offline --locked
cargo clippy --manifest-path web/Cargo.toml -p md-web-contracts -p md-web-services --all-targets --offline --locked -- -D warnings
```

The focused Node suite passed 11 tests. After the PTY facade landed, Rust passed
58 contract tests and 225 service tests; a final persistence-only run passed all
31 persistence tests. Clippy passed for both crates and all targets with warnings
denied. Cargo used `--offline --locked` and task-private build/temp output.

An authorized disposable PostgreSQL integration test should apply 001 and 002
twice, assert schema version 2, exercise CAS conflict, concurrent replay append,
ambiguous-event retry, trigger retention at 500, namespace isolation, history
literal wildcard search, nullable cost model, and restart-segment lifetime totals.

`NOT VERIFIED`: live PostgreSQL version/provider, migration lock behavior on the
deployment target, TLS/roles/grants, row counts and growth, backup/PITR, RPO/RTO,
autovacuum, replay retention policy, and restore time. No DB connection, DDL, DML,
restore, external access, or production mutation was performed.

## Bounded pre-mortem

- Secret enters JSONB: prevent with typed browser-safe serializers and a server
  allowlist; test serialized connection/config documents for forbidden secret
  fields; stop the write on a match; rotate the credential and delete the scoped
  contaminated record through an approved recovery operation.
- Two authorities overwrite a task: prevent with one owner per record kind and
  CAS; observe revision conflicts; stop authority flip on conflict; recover from
  the retained source plus PostgreSQL record comparison.
- Replay sequence diverges after retry: prevent with stable event ids and the
  locked-head transaction; test concurrent same-id append; stop if more than one
  sequence is consumed; repair only in an offline approved namespace migration.
- Migration partially applies: prevent with the migration transaction and
  advisory lock; verify version receipt plus table/index catalog; stop on first
  false predicate; transaction rollback is the recovery.
