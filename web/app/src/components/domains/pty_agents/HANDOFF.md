# PTY / agent domain integration handoff

## Root module wiring

- Export `contracts/src/domains/pty_agents` from `contracts/src/domains/mod.rs` and re-export the public DTOs from the canonical contracts root if flat imports are required.
- Export `services/src/domains/pty_agents` from `services/src/domains/mod.rs`.
- Export `app/src/components/domains/pty_agents` from the app domains module.
- Import `/assets/domains/pty_agents.css` from the route or the canonical application stylesheet.

## Required dependencies

- Add `portable-pty = "=0.9.0"`. The owned service already uses its native PTY, controlling terminal, resize/ioctl, PID, and PTY process-group identity APIs.
- On Unix add `nix = { version = "=0.30.1", features = ["process", "signal"] }`; explicit teardown sends SIGHUP then SIGKILL to the PTY process group so descendants are not orphaned.
- Provide xterm and its CSS to the browser (`@xterm/xterm`). Load `@xterm/addon-fit` before the bridge as `globalThis.FitAddon.FitAddon`.
- For Japanese/CJK width parity, copy `node_modules/@xterm/addon-unicode11/lib/addon-unicode11.js` into the same server-owned vendor asset directory and load its route asset **after xterm but before `xterm_bridge.js`**. Its UMD export must be available as `globalThis.Unicode11Addon.Unicode11Addon`; keep its version compatible with xterm. The bridge activates Unicode version 11 when that constructor is present and otherwise retains xterm's built-in Unicode provider. WebGL remains optional.
- `xterm_bridge.js` intentionally expects the terminal/addon constructors through those globals and owns no application state. Do not silently rename the globals in the asset integration.
- Import `xterm_bridge.js`, call `startTerminalBridge()` once, feed every decoded socket frame to `applyServerFrame`, and translate bubbling `md-terminal-input` / `md-terminal-resize` events into `TerminalClientFrame`. The bridge drops duplicate sequence numbers and stale generations.
- Use Dioxus/Axum's typed WebSocket extraction already selected by the integration owner; do not expose a second untyped REST stream.

## Server state and endpoints

Keep one process-lifetime `Arc<PtyRegistry>` in the fullstack server state. Required Server Functions:

- `list_agents() -> Vec<AgentRecord>`
- `spawn_agent(SpawnAgentRequest) -> SpawnAgentResult`
- `kill_agent(pty_id)`; on success call `AgentLifecycle::archive` and clear its `TerminalQueue`
- `restart_agent(RestartAgentRequest)`; build and validate the replacement with `restart_spawn_request` **before** killing the current process
- `restore_agent(RestoreAgentRequest)` and `restore_all()` via `restore_spawn_request`
- `enqueue_terminal_input(QueuedTerminalMessage)`; map the UI's `QueueMessage { agent_id, text }` action here, assign its durable ID/timestamp server-side, then let the delivery loop write text and `\r` separately. Acknowledge only after both writes succeed, and call `record_failure` after a failed pair. `Input { pty_id, data }` is reserved for direct xterm keystrokes.
- `resize_pty`, `redraw_pty`, and `list_ptys`

WebSocket endpoint: `/ws/terminal`. Accept `TerminalClientFrame`; emit `TerminalServerFrame`. Preserve `{pty_id, generation, seq}` and ignore stale-generation output. Reconnect with `Attach { after_seq }` and return the retained output window before live frames.

## Cross-domain inputs

- Config/onboarding: harness home, default provider/model, auto mode, max turns, registered repositories, terminal theme.
- Hive/tasks: agent provisioning, role/session ID updates, hook status events, inbox count, control pause/halt/auto-delivery flags, route-to-terminal handoffs.
- FS/git: canonical cwd validation, optional isolated worktree creation, worktree existence check during restore, safe teardown/preservation.
- Persistence: durable active/archive/restorable recipes and terminal queues in PostgreSQL. Do not persist terminal output or secret environment.
- Connections/secrets: materialize BYOK only inside the server spawn boundary. Never add it to `SpawnAgentRequest`, `AgentRecord`, WebSocket frames, logs, or browser state.
- Office UI: project `AgentRecord` into the shared agent strip and select the same stable agent ID.

## Environment

No new browser-visible environment variables. Provider credentials remain server-only. Native process startup needs the server's resolved login-shell `PATH`; define that in the integration layer rather than accepting arbitrary environment entries from the browser.

`NativePtyBackend` always removes `MD_PG_PASSWORD`, `MD_PG_HOST`, `MD_PG_PORT`, `MD_PG_DATABASE`, `MD_PG_USER`, `MD_PG_NAMESPACE`, and `MD_PG_TLS_CA` from the child command without mutating the server's global environment. Keep that boundary intact when adding provider-specific environment materialization.

## Lifecycle invariants

- An ID is active, archived, or restorable—never more than one.
- Explicit kill and natural exit run the same teardown.
- `require_resume` failure never kills or replaces the current process.
- Worktree and hive provisioning failures remain explicit integration decisions; do not silently claim isolation/resume.
- Queue automation waits through boot, active output, user drafts/pickers, auto-delivery pause, and HITL. A constrained agent may receive steering only after 12 s of measured silence.
- Agent output is scoped by PTY ID. Do not broadcast one agent's terminal stream into another workspace or browser session.

## Process-owned exit monitor

Natural-exit persistence must not depend on `send_frames`, `drain_frames`, or any connected browser. Call `PtyRegistry::start_exit_monitor` exactly once on the process-lifetime `Arc<PtyRegistry>`. The callback runs on its monitor thread, so it should only send each typed `PtyExitEvent` into a Tokio `mpsc::UnboundedSender`; one server task then performs the async durable transition.

For each event, first confirm `is_current_generation(event.pty_id, event.generation) == Ok(true)`, then retry the existing transactional `persist_agent_exit` until a terminal receipt (success, stale generation, or canonical conflict resolved by reload). That repository operation clears the durable terminal queue in the same transaction. After its receipt, update the in-memory agent revision and archive/preserve its worktree capability. WebSocket `Exited` frames are presentation-only and must never call persistence again.

## Queue delivery closure

- Generate each durable queue message ID with server-side `Uuid::new_v4()` (or a database-generated UUID). Never use a process-local counter such as `web-1`; it collides after restart and across server processes.
- Build `DeliveryGate` from current facts at delivery time: durable agent status, measured quiet time from `PtySummary.last_output_at_ms`, automation/HITL pause, boot grace, cooldown, active browser draft/picker ownership, and inbox precondition. Do not hardcode `quiet_ms = u64::MAX` or every safety flag to ready.
- After a `Send` decision, call blocking `PtyRegistry::deliver_queued_message` through `tokio::task::spawn_blocking`. It serializes deliveries per PTY generation, writes the complete single-line payload or multiline bracketed paste, waits `QUEUE_ENTER_DELAY` (140 ms), then writes Enter. A stable ID reused by another generation cannot receive the delayed Enter.
- Acknowledge the durable head only after `deliver_queued_message` succeeds. Record one failure after an error; retain the existing three-attempt drop contract. The delivery worker, not the request handler, must keep revisiting persisted waiting queues when readiness facts change.
- Use `DELIVERY_BOOT_GRACE_MS` (35,000), `DELIVERY_QUIET_MS` (4,500), and `DELIVERY_COOLDOWN_MS` (4,500) through `evaluate_terminal_readiness`; never replace these with immediate/maximum-ready facts. Delivery requires the first real PTY output, completed boot grace, quiet and cooldown windows, and no browser draft, picker, or IME composition ownership.
- Route `TerminalClientFrame::Presence { pty_id, presence }` from both `PtyAgentsAction::Presence` and the bridge's bubbling `md-terminal-presence` event into `TerminalFrameRouter`. Project `TerminalReadiness.status == Busy` as the per-agent busy/working display while recent activity is inside the quiet/cooldown window. Server timestamps are canonical; clamp or replace browser `last_activity_at_ms` rather than trusting a future value.
- Do not call Hive `take_steer`/one-shot `hook_decision` merely because a hook arrived. First select a nonempty durable queue head (`TerminalQueue::selected_head_id(agent_id).is_some()`); only that selected delivery may consume a steer. An empty queue leaves the Hive steer queued for the next eligible selected message.

## Restart CAS rollback

After replacement spawn, capture `registry.current_generation(pty_id)`. Do not update the in-memory durable agent before PostgreSQL CAS succeeds. If `upsert_floor_agent` fails, immediately call `registry.kill_generation(pty_id, replacement_generation)` so only that replacement is stopped, leave the prior durable recipe/revision untouched, and reload the canonical durable record into memory. This avoids both an orphan replacement and accidentally killing a newer concurrent generation. Explicitly handle cleanup errors rather than returning early through `?`.

## Named-tool Hive hook seam

Keep hook credentials server-only; do **not** add them to `SpawnAgentRequest`, `AgentRecord`, browser state, persistence, or logs. For each spawn/restart/restore, generate a cryptographically random URL-safe capability of 32–512 ASCII bytes (a fresh UUID v4 string satisfies this contract), then construct:

```rust
let hook = AgentHookLaunch::new(
    "http://127.0.0.1:5000/internal/hive-hook",
    canonical_agent_id,
    generated_capability,
    canonical_persisted_harness_home.join("runtime"),
)?;
let spawned = registry.spawn_with_hook(spawn_request, Some(hook))?;
```

The endpoint URL must be an absolute `http`/`https` URL with a host and without userinfo, query, or fragment. The runtime root must be an existing absolute private directory derived from the canonical persisted harness configuration; never use `/tmp`, `temp_dir`, cwd, or a browser-supplied path in production. `spawn_with_hook` requires its normalized agent ID to equal the spawn ID. The native child receives only `MD_HIVE_HOOK_URL`, `MD_HIVE_AGENT_ID`, and `MD_HIVE_HOOK_CAPABILITY`; inherited values for these and the internal helper/header variables are removed first. The existing `MD_PG_*` deny-list remains enforced and cannot be reintroduced through this typed seam.

Mount one integration-owned POST endpoint that deserializes `AgentHookRequest`. Before any Hive decision or event mutation, call `registry.verify_hook_request(&request)` and return an unauthenticated/forbidden response unless it is exactly `Ok(true)`. Never include the request capability in an error, tracing field, metric label, persistence payload, or `AgentHookDecision`. After verification, dispatch `request.agent_id`, `request.event_id`, typed `request.event`, optional `request.tool_name`, and `request.payload` to Hive, then map the full one-shot Hive result to `AgentHookDecision { allow: hive.allow, reason_ja: hive.reason_ja, steer: hive.steer }`. Preserve pause/halt/tool-gate denial in `allow`/`reason_ja`; do not convert it into steering. Return `steer` only for the verified agent/event and consume it once in the Hive producer. `AgentHookDecision` redacts steering content from `Debug`, but the endpoint must also avoid logging the serialized response body.

Provider hook provisioning is automatic inside `NativePtyBackend` for Claude, Codex, and Gemini. Claude receives an isolated `--settings` hook file; Codex preserves the canonical inherited `CODEX_HOME` (auth, config, packages, and sessions) and receives validated `-c hooks.<Event>=...` overlays plus the installed flag `--dangerously-bypass-hook-trust`; Gemini receives its native `hooksConfig`/BeforeTool/AfterTool settings through `GEMINI_CLI_SYSTEM_SETTINGS_PATH`. Antigravity and all other providers return an explicit unsupported-hook error rather than silently spawning without boundaries or mutating a global user hook file. The installed Codex 0.149.1 parser was smoke-tested with `--strict-config` against the same inline hook table shape; keep the spawn-time help capability check so older CLIs fail explicitly.

The generated provider command invokes a mode-0700 local relay and keeps identity/capability in a mode-0600 header file. Mount raw routes `POST /internal/hive-hook/{provider}` (`provider` is `claude`, `codex`, or `gemini`) that accept the provider's original JSON body plus `X-MD-Agent-ID` and `X-MD-Hook-Capability`. Normalize event/tool/payload into `AgentHookRequest`, verify it, call Hive once, then use `render_claude_hook_response(event_name, &decision)` for Claude/Codex or `render_gemini_hook_response(event_name, &decision)` for Gemini. `parse_agent_hook_decision` is available for normalized relay consumers. Do not log raw request/response bodies. The relay requires the packaged runtime to provide `curl`; startup/preflight must report hook provisioning unavailable if it is absent.

Capability lifecycle is generation-bound: every successful `spawn_with_hook` rotates the agent entry; the old capability fails immediately. Explicit kill, generation-specific rollback kill, and natural exit remove only the matching lease, so a delayed old-generation cleanup cannot revoke a newer rotation. A failed native spawn never installs the capability. Keep the process-lifetime `PtyRegistry`/exit monitor across ordinary route reloads; a namespace reset must kill all live PTYs before clearing integration agent/worktree projections, otherwise child processes survive without canonical records.

Hook absence is a control limitation, not a spawn failure. `SpawnAgentResult.hook_supported` is true only for an active Claude/Codex/Gemini bridge. Every other `AgentProvider` must still spawn normally with `hook_supported == false`; surface that limitation in the agent UI and use queue/status observation rather than claiming pause/gate/steer enforcement.
