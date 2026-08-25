# Office UI domain handoff

## Owned artifacts

- Contracts: `web/crates/contracts/src/domains/office_ui/`
- Pure service reducers: `web/crates/services/src/domains/office_ui/`
- Dioxus components and browser island: `web/app/src/components/domains/office_ui/`
- Styles: `web/app/assets/domains/office_ui.css`
- Browser contract tests: `web/e2e/office_ui.spec.ts`

## Integration dependencies

The integration owner must add the module chain in the existing shared files:

```rust
// md-web-contracts/src/lib.rs
pub mod domains;

// md-web-contracts/src/domains/mod.rs
pub mod office_ui;

// md-web-services/src/lib.rs
pub mod domains;

// md-web-services/src/domains/mod.rs
pub mod office_ui;

// md-web-app/src/components/mod.rs
pub(crate) mod domains;

// md-web-app/src/components/domains/mod.rs
pub(crate) mod office_ui;
```

The shared shell should render `OfficeUi` instead of the placeholder dashboard.
The domain server-function adapter is `web/app/src/server_fns/office.rs`; the
integration owner must declare and re-export it from the shared `server_fns`
module.

PixiJS 8.18.1 is vendored from the repository's locked `pixi.js` installation
as `assets/pixi.min.js`. The island module loads portrait art, the distinct
Darryl extension, and Pixi sequentially from Dioxus `asset!` URLs before it
mounts the TMJ scene. No CDN, npm install, manifest edit, or runtime network
fetch is required.

The original `office.tmj`, `brooklyn99.tmj`, and their three PNG tilesets are
copied byte-for-byte into `assets/{maps,tilesets}`. They are each declared with
`asset!` in `floor_island.rs`, and their emitted public URLs are passed to the
browser island as DTO attributes. Dioxus therefore fingerprints and serves all
map resources as public/static assets without a shared pipeline change.

The island ports the original Tiled layer renderer (including Tiled flip bits),
collision parsing, walkable spawn overrides, BFS pathfinding, ordered seat pool,
camera fitting, status animation, click anchors, selection, and animated hive
handoff envelopes. Canvas2D remains only as a degraded fallback if WebGL/Pixi
initialisation fails. Original attribution and the LimeZu/Pixi licences are
kept beside the assets.

## Server/API inputs

`OfficeUi` needs:

- `OfficeSnapshot` from a server function or ordered event projection;
- `Vec<CompletionNotice>` from the shared event stream;
- app version and auto-mode display state;
- a `detail_panel: Element` supplied by the selected-agent domain;
- controlled `add_agent_open`, `add_agent_spawning`, and `focus_mode` state.

Callbacks map to other domain owners:

| Callback | Consumer |
| --- | --- |
| `on_add_agent` / `on_close_add_agent` | controlled add-agent modal visibility |
| `on_spawn_agent(OfficeAgentSpawnRequest)` | exact PTY spawn request plus office character/accent/project/goal |
| `on_select` | selected-agent projection |
| `on_reorder` | durable roster ordering |
| `on_rename` | roster/hive registry mutation |
| `on_note` | durable private note mutation |
| `on_open_task` | task detail route/overlay |
| `on_open_tasks` | canonical Tasks tab/route from floor boards |
| `on_open_human_questions` | canonical Ask tab/route from the floor board |
| `on_request_close` | web app close/shutdown confirmation; never selected-agent kill |
| `on_restore_all` / `on_dismiss_restore` | agent restore lifecycle |
| `on_theme` | durable UI preference + terminal theme notification |
| `on_open_settings` | settings overlay |
| `on_toggle_focus` | terminal/focus domain |
| `on_dismiss_notice` | completion toast reducer |
| `on_open_ide` / `on_open_terminal` / `on_close_agent` | selected-agent lifecycle actions |

`OfficeAgentSpawnRequest` is lossless. The modal sends every editable process
field; the server generates the same unique process ID behavior as the original
when the hidden ID is empty. Do not replace it with a unit callback or defaults.

The selected-agent detail consumer must provide exactly these ten focus tabs,
in this order, to match the canonical video: `Terminal`, `Monitor`, `Tasks`,
`Ask`, `Triggers`, `Memory`, `Graph`, `Activity`, `Commands`, `Workers`.
`AgentDetailHost` owns only the portrait/title/status and IDE/open/close chrome;
the tab content stays with those domain owners.

Each tab needs its own compact content hook: Terminal → selected PTY, Monitor →
agent telemetry, Tasks → selected-agent task board, Ask → selected-agent human
questions, Triggers → trigger manager, Memory → selected-agent memory, Graph →
memory graph, Activity → activity/history, Commands → selected-agent command
history, and Workers → selected-agent workers. In particular, Commands must not
reuse the Terminal component; in the shared `OfficeCommandCenter` match, index
`0` maps to `SelectedAgents` and index `8` maps to `CommandHistoryCompact`.

The detail consumer receives the resolved `selected_agent_id` exposed as
`data-selected-agent-id`; it must match
`AgentDetailHost[data-agent-detail-id]`. Do not feed the tabs a stale raw
selection when the office has fallen back to Aria or the first live agent.
The resolved agent is selected in this exact order: a live match for the
snapshot ID, then the live `is_god`/Aria agent, then the first live agent. The
shared route must use the same resolved ID for every selected-agent context
input (terminal, monitor, tasks, questions, triggers, memory, graph, activity,
commands, and workers), not just for the visible tab header.

## Server-function exports

Re-export these functions from shared `server_fns.rs`:

```text
office_snapshot
office_spawn
office_close_agent
office_restore_all
office_dismiss_restore
office_select
office_reorder
office_rename
office_note
office_theme
office_theme_preference
office_pause
office_focus
office_auto_mode
office_toast
office_dismiss_toast
office_live_update
office_live_poll
```

`office_snapshot` joins the live PTY registry with the server-owned
`OfficeProjection`; the route must not construct `OfficeSnapshot::default()`.
Spawn delegates to `pty_spawn`, close delegates to `pty_kill`, and restore-all
delegates to `pty_restore`. The route should refresh `office_snapshot` after a
successful mutation. `office_dismiss_restore` durably filters one restore card
through the PostgreSQL-backed office projection; the route must not report that
the action is unsupported.

The floor island emits one bubbling `office-ui-action` `CustomEvent` with DTOs:

```text
{ type: "select_agent", data: { agent_id } }
{ type: "open_tasks" }
{ type: "open_human_questions" }
{ type: "request_close" }
```

The integration adapter should translate these into the same callbacks. Hive
handoffs enter the island as `office-handoff` events containing
`{ event_id, sequence, from, targets, act, needs_human }`.

## Live projection fan-in

After the initial `office_snapshot`, the shared event owner must translate its
durable streams into `OfficeLiveUpdate` and call `office_live_update`:

- PTY output/hooks → `AgentState` with the complete status/action/prompt,
  progress, context, carrying and draft state;
- Hive ledger snapshot → `ReplaceTasks` using browser-safe `OfficeTask` values;
- `HiveDomainEvent::MessageRouted` → `Handoff` using the durable message ID and
  event sequence; the island de-duplicates by sequence;
- memory telemetry plus the durable cost ledger → `Telemetry`; convert USD to
  integer millionths once at the adapter boundary;
- route selection → `SelectAgent`, or continue using `office_select`; both
  increment the same Office revision.

The producer remains the durable owner. Live ingestion intentionally avoids a
PostgreSQL write per token/tool sample. Later Office preference/note writes also
strip tasks, handoffs, telemetry and live agent fields from the Office record,
so a restart cannot revive stale source-owned state. When SSE is unavailable, poll
`office_live_poll(Some(last_revision))`; `None` means unchanged and a returned
snapshot includes the synchronized selected agent. Reconnect by taking a fresh
source snapshot, then resume ordered events without inventing defaults.

The shared `OfficeCommandCenter` must key/remount its selected-agent content by
`selected_agent_id` or make every resource effect explicitly depend on that
prop. The required A→B-dependent children are Terminal, Monitor, Tasks, Ask,
Memory, Commands and Workers. Trigger, Graph and global Activity are not
agent-scoped. The Office-owned detail boundary changes both its Dioxus key and
`data-agent-content-id` on A→B for the browser acceptance test.

## Public/static hookup

No shared code is needed for the asset hookup. A production build must retain
the URLs generated by these declarations:

- `PORTRAIT_ART_JS`, `DARRYL_ART_JS`, `PIXI_JS`, `OFFICE_ISLAND_JS`;
- `OFFICE_MAP`, `BROOKLYN99_MAP`;
- `OFFICE_TILESET`, `OFFICE_FLOORS_WALLS`, `OFFICE_INTERIORS`.

The app's existing Dioxus static-asset collector handles them. Do not replace
the local Pixi script or image URLs with a CDN.

Runtime acceptance markers are `data-pixi-state="ready"`,
`data-renderer="pixi"`, `data-runtime-marker="pixi-tmj-ready"`, and a concrete
`data-map-loaded`. Degraded fallback exposes the exact failure in
`data-load-error` and retains its visible status message.

The floor host is keyboard-operable: Enter/Space selects the resolved floor
agent, `T` opens Tasks, `A` opens Ask, and `Q` requests app close. Focus mode
exits on Escape. The add-agent dialog remounts to reset its draft, initially
focuses Name, traps Tab/Shift+Tab, and closes on Escape.

## Boundaries

- The browser island never reads `window.cth`, Electron IPC, filesystem paths,
  credentials, secrets, or PostgreSQL.
- Process lifecycle, PTY teardown, persistence, and theme-switch confirmation
  remain server-side.
- The server adapter persists office-only note/order/theme/focus and dismissed
  restore IDs through the PostgreSQL projection record; no browser fallback or
  SQLite path is allowed.
- Electron updater/restart and OS-window close interception are not browser UI
  responsibilities. A web release notice can feed `CompletionNotice` or a
  dedicated update domain.
- `OfficeProjection` validates roster identity, progress, names, and notes before
  publishing a new revision.

## Verification after wiring

```bash
cargo fmt --manifest-path web/Cargo.toml --all -- --check
cargo test --manifest-path web/Cargo.toml --workspace
cargo clippy --manifest-path web/Cargo.toml --workspace --all-targets --all-features -- -D warnings
npx playwright test web/e2e/office_ui.spec.ts
```

The Playwright spec expects the shared shell wiring and therefore should run at
the fan-in stage, not against the earlier placeholder shell.
