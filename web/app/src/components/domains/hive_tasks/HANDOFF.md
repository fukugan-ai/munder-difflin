# Hive Tasks domain handoff

## Owned artifacts

- Contracts: `web/crates/contracts/src/domains/hive_tasks/**`
- Server services: `web/crates/services/src/domains/hive_tasks/**`
- Dioxus UI: `web/app/src/components/domains/hive_tasks/**`
- Styles: `web/app/assets/domains/hive_tasks.css`
- Browser checks: `web/e2e/hive_tasks.spec.ts`

## Integration dependencies

The integration owner must make these shared-file changes; this worker intentionally did not edit them.

1. Add the lock-compatible `serde_json = "=1.0.151"` to `web/Cargo.toml` workspace dependencies and enable it in both `md-web-contracts` and `md-web-services`.
2. Export `domains::hive_tasks` from `web/crates/contracts/src/lib.rs` and `web/crates/services/src/lib.rs`.
3. Export `components::domains::hive_tasks` from the app component module tree.
4. Import `assets/domains/hive_tasks.css` after the shared tokens in the app stylesheet.
5. Mount `HiveTasksDomain` on `/hive`; the E2E spec uses that canonical route.

## Server-function/API mapping

The callable adapter is implemented in `web/crates/services/src/domains/hive_tasks/service.rs`
and the Dioxus boundary is implemented in `web/app/src/server_fns/hive.rs`. The shared
`server_fns.rs` owner must add:

```rust
mod hive;
#[cfg(feature = "server")]
pub(crate) use hive::hive_event_stream;
pub(crate) use hive::{
    hive_add_task, hive_answer_question, hive_control_auto_delivery, hive_control_gate,
    hive_control_halt, hive_control_pause, hive_control_resume, hive_control_steer, hive_delete_task,
    hive_create_task, hive_dismiss_question, hive_events_replay, hive_inbox, hive_move_task,
    hive_new_thread, hive_patch_role, hive_patch_task, hive_reply, hive_send, hive_set_hold,
    hive_snapshot, hive_stop_worker, hive_workers,
};

#[cfg(feature = "server")]
pub(crate) use hive::{
    hive_agent_hook_event, hive_control_hook_decision, hive_register_worker_projection,
    hive_reinitialize_harness_home, hive_scheduler_enqueue_message,
    hive_scheduler_enqueue_task,
};
```

The shared server launcher must mount the replaying SSE endpoint:

```rust
.route("/api/hive/events/stream", get(server_fns::hive_event_stream))
```

The implemented Server Functions delegate to `HiveTasksService`, which owns one
`HiveStore`, `HiveRouter`, `ControlRegistry`, `WorkerRegistry`, and `EventHub`:

- `hive_registry`, `hive_tasks`, `hive_inbox`, `hive_messages`
- `hive_send`, `hive_add_task`, `hive_patch_task`, `hive_delete_task`
- `control_pause`, `control_auto_delivery`, `control_resume`, `control_steer`, `control_halt`, `control_snapshot`
- `workers_list`, `workers_stop`
- `hive_create_task`, `hive_new_thread`, `hive_patch_role`, `hive_set_hold`
- `hive_events_replay` and `/api/hive/events/stream`, both over the PostgreSQL
  replay ledger and its durable stream sequence

The service root is resolved only from the latest durable common config's absolute
`harness_home`, with `hive` appended. After committing a home change, call
`hive_reinitialize_harness_home().await`; the next request loads the new durable config
and constructs a service keyed to the new root. No root is accepted from a browser request.

## Route callback mapping

The shared `/hive` route must call `hive_snapshot(selected_agent_id)` for initial and
manual refreshes, convert the returned `HiveSnapshot` directly into
`HiveTasksViewModel`, and wire callbacks as follows:

- `TaskAction::Create` -> `hive_create_task`
- `TaskAction::Move` -> `hive_move_task`
- `TaskAction::Delete` -> `hive_delete_task`
- `TaskAction::Answer` -> `hive_answer_question`
- `TaskAction::DismissQuestion` -> `hive_dismiss_question`
- `MessageAction::Reply` -> `hive_reply`
- `MessageAction::NewThread` -> `hive_new_thread`
- `ControlAction::{Pause, AutoDelivery, Resume, Steer, Halt}` -> matching
  `hive_control_*` function
- `ControlAction::PatchRole` -> `hive_patch_role`
- `ControlAction::SetHold` -> `hive_set_hold`
- `on_stop_worker(worker_id)` -> `hive_stop_worker`
- `on_refresh(())` -> restart the `hive_snapshot` resource

After every successful mutation, restart the snapshot resource until the shared route
applies `HiveEventEnvelope` deltas locally. On SSE event `hive-reset` or a replay result
with `gap == true`, discard local deltas and fetch `hive_snapshot` before continuing.

Use one keyed process-lifetime `Arc` per current durable harness home. Local mutations first
publish to the bounded `EventHub`; the adapter flushes those envelopes to the PostgreSQL
`hive` replay stream with stable retry IDs. Browser replay and SSE read PostgreSQL sequences,
so `Last-Event-ID` survives process restart. On `gap == true`, refresh the snapshot before
applying replayed deltas.

## Mutation invariants

- Task patch requests must be a `serde_json::Map<String, Value>` and must call `HiveStore::patch_task`; never serialize a display-normalized collection back over `tasks.json`.
- Unknown task fields (`scope`, `origin`, `commit`, Slack/webhook metadata, and future fields) are preserved.
- `id` in a patch is ignored; card identity cannot be replaced.
- Human answers patch only the newest open `humanQA` entry, then send an `inform` message to god. Dismissal adds `dismissedAt` and leaves the task blocked.
- Message bodies and filesystem paths remain server-side except for the explicit thread/inbox read surfaces.

## Cross-domain handoffs

- PTY/process owner: call `hive_control_hook_decision(agent_id, tool)` at the actual
  command/tool boundary and enforce `paused`, `halted`, `tool_gated`, and
  `auto_delivery_paused`; deliver the returned one-shot `steer` before continuing.
  After spawn, call `hive_register_worker_projection(worker)`. Browser
  `hive_stop_worker` marks releasing, kills the canonical PTY, preserves the resolved
  worktree path, removes the live projection, publishes teardown, and returns
  `WorkerTeardownReceipt`.
  Current shared PTY queue and direct-input consumers call the hook with `tool = None`,
  so pause/halt/auto-delivery/steer are connected. A named tool dispatcher must call it
  with `Some(tool_name)` before execution for `gate_tool` to have an execution effect.
  The authenticated `/internal/hive-hook` endpoint must instead call
  `hive_agent_hook_event(&request)` exactly once after capability verification and use
  its returned decision. It deduplicates by `(agent_id, event_id)`, replays the same
  one-shot steering decision on retry, and publishes only agent ID, event ID, typed event,
  and normalized tool name. It never persists or logs `capability` or arbitrary `payload`.
- Voice owner: export and call `hive_control_gate(agent_id, tool, on)` for the
  voice `gate_tool` action. It writes the same process-lifetime `ControlRegistry`
  used by pause/resume and publishes `ControlChanged`; do not keep a voice-only
  gate map. Hook consumers continue reading `HiveTasksService::control_snapshot`.
- Git owner: after successful Hive filesystem mutations, enqueue the existing single-committer Hive Git commit. This domain deliberately does not shell out to Git.
- Scheduler owner: enqueue through `hive_scheduler_enqueue_task` and
  `hive_scheduler_enqueue_message`; do not write task/message files directly. These
  adapters are implemented but remain consumer-free until the shared scheduler dispatch
  boundary calls them.
- Shell/router owner: feed all snapshot fields (`board`, `log_tail`, `selected_memory`
  included), translate UI actions to server functions, and reconnect the event stream
  with `Last-Event-ID`. Mount `HiveInitialTab::Tasks` for Tasks navigation and
  `HiveInitialTab::AskMe` for human questions.
- Notifications/Slack owner: after a task reaches `done`, use its preserved Slack metadata for the existing one-shot thread summary.
- PostgreSQL owner: Hive coordination artifacts remain the existing local filesystem
  protocol, while event replay is durably appended to the existing PostgreSQL replay
  repository under stream `hive`.

## Environment and route assumptions

- Canonical browser route: `/hive`
- Canonical hive root: server-resolved `<harnessHome>/hive`; never accepted as a browser request parameter.
- LAN binding and port remain owned by the shared server launcher.
- The browser never receives credentials, capability tokens, or arbitrary absolute-path access.
