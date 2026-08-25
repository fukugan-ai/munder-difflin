# Canonical video parity

Acceptance oracle: `/home/fukugan/hero-demo.mp4` (35.30 s, 2300×1440, 60 fps).
Representative frames were extracted to task-private temporary storage only.

| Timestamp | Canonical behavior | Office-domain resolution |
| --- | --- | --- |
| 00:00–09.30 | Pixel-art office occupies roughly two thirds of the workspace; command center occupies the right third; horizontal agent cards sit along the bottom. | `OfficeUi` preserves the three-region macro-layout with a 1.72:1 workspace split and bottom `AgentStrip`. |
| 00:00–10.66 | Distinct cast sprites walk through the mapped room and show outlined speech/action labels. | The Pixi island loads the original TMJ/PNG floor, original procedural cast portraits, pathfinding, seat assignments, idle errands, status animation, bubbles, selection, and handoffs. |
| 00:00 | Original wordmark, macOS traffic lights, version, auto mode, theme/settings/focus controls. | The original repository logo replaces the invented badge; titlebar hierarchy and controls are present. |
| 09.30–16.50 | The office dims and a centered four-section Add Agent dialog advances through Identity, Workspace, Engine, and Briefing. | Controlled modal matches the four sections, 940 px canonical width, cast portrait grid, accent/provider selection, all spawn fields, and loading state. |
| 16.50–17.60 | Spawn shows a pending state, then the modal closes and Andy appears. | `add_agent_spawning` is controlled by the route; the typed request reaches real `pty_spawn`; snapshot refresh makes the new live agent visible. |
| 17.60–23.60 | Selecting Andy replaces the right panel with the selected-agent detail and lifecycle actions. | `AgentDetailHost` renders the selected portrait/title/status/project and IDE/open/close callbacks around the supplied domain detail content. |
| 23.60–35.30 | Focus mode replaces the floor with a project-grouped vertical agent rail and full detail surface. | `focus_mode` renders a responsive grouped rail and expanded detail host with an enter transition. |
| 24.00–35.30 | Detail tabs appear in the order Terminal, Monitor, Tasks, Ask, Triggers, Memory, Graph, Activity, Commands, Workers. | The host accepts the real detail element. Exact tab order/content is a shared integration requirement because those panels are outside office ownership. |

The canonical orchestrator display name/persona is **Aria**. Legacy `god` and
`michael` process identifiers are accepted only at the integration boundary and
resolve to Aria-facing copy; they are not shown as the current persona.

## Shared acceptance hooks

- `OfficeUi` must receive `OfficeUiState` from `office_snapshot`; no default or
  fabricated snapshot is acceptable.
- Modal visibility, spawning, focus, selection, theme, and notices are
  controlled route state backed by the office server functions.
- The detail consumer supplies the ten tabs in the canonical order above.
- Agent input ordering must be project-grouped before focus rendering so the
  rail headings match the video.

## Known external dependency

The browser island is asset-complete and makes no Electron calls. Actual PTY
spawn still requires the host's registered workspace and harness configuration;
live browser-to-PTY behavior is not established by the video-only audit.
