# Voice / Realtime integration handoff

The voice domain is mounted as `VoiceRealtimeDomain`. Long-lived credentials remain
inside the server process; WebAssembly receives only presence flags and a short-lived
OpenAI Realtime client secret.

## Integrated exports and route

- Contracts: `md_web_contracts::domains::voice_realtime::*`.
- Services: `md_web_services::domains::voice_realtime::*`.
- UI: `crate::components::domains::voice_realtime::VoiceRealtimeDomain`.
- Route: `/voice` renders `VoiceRealtimeDomain {}`.
- CSS: `assets/app.css` imports `domains/voice_realtime.css`.
- Browser island: `realtime_bridge.js`, loaded by `VoiceBridgeScript`.

## Callable Server Functions

`web/app/src/server_fns/voice.rs` exports:

- `voice_bootstrap()` — config, key-presence booleans, HTTPS status, model, timeout.
- `voice_set_freeflow_config(patch)` — enablement and model update; no secret field.
- `voice_write_provider_key(VoiceProviderKeyWrite)` / `voice_clear_provider_key` —
  write-only Groq/OpenAI updates through the shared server-only `SecretProvider`.
- `voice_transcribe(metadata, audio)` — CBOR upload and server-side validation.
- `voice_mint_realtime_token(request)` — server-side OpenAI key to ephemeral secret.
- `voice_action`, `voice_confirm_action`, `voice_cancel_action` — every mutation
  crosses `ActionPolicy` first.
- `voice_set_realtime_cost_cap` / `voice_record_realtime_usage` — server-owned live
  audio metering and hard-cap state.
- `voice_set_session_live` and `voice_events` — bounded completion, floor, and
  renderer-queue event delivery.

Real adapters cover dispatch/ping/steer, task create/assign/update/delete/dependency,
spawn, kill, pause/halt/resume/auto-delivery, archive/unarchive, clear-context,
tool gates, schedule create/edit, and allowlisted setting updates. Spawn, schedules,
tool gates, and settings use typed request fields; every mutation returns an explicit
result and remains subject to the voice confirmation policy.

## OpenAI Realtime browser flow

The bridge obtains microphone media, connects to `/v1/realtime/calls` with only the
ephemeral token, sends `session.update` with the three function tools, forwards
`response.function_call_arguments.done` to Rust, and returns the server result as
`function_call_output` followed by `response.create`.

- `munder:voice-freeflow-event`
- `munder:voice-realtime-event`

Free Flow uses `MediaRecorder`; its `ArrayBuffer` becomes bytes for the CBOR Server
Function without base64. Realtime uses `RTCPeerConnection`, `setSinkId` when
supported, and `mediaDevices.devicechange`. Input/output selections persist only in
browser local storage. The capture-phase Option/Alt shortcut arms after a 320 ms solo
hold, aborts for composition/Alt combinations, and never calls `preventDefault`.
Realtime `response.done` usage is priced on the server; completion/floor events are
returned to the live conversation as explicitly labelled non-command notifications.

## Environment / credential inputs

- `OPENAI_API_KEY`: server-only ephemeral token minting.
- `GROQ_API_KEY`: server-only file transcription.
- `MD_FREEFLOW_ENABLED`, `MD_FREEFLOW_MODEL`, `MD_REALTIME_IDLE_DISCONNECT_MS`.
- `MD_WEB_HTTPS`, `MD_WEB_TLS_CERT_PATH`, `MD_WEB_TLS_KEY_PATH`, `MD_GOD_AGENT_ID`.
- No live upstream calls were made by this implementation worker.

## Cross-domain inputs

- Agent roster and PTY queue/kill APIs.
- Hive task and agent-control APIs.
- Completion and floor changes are derived by the server-side watchers during
  `voice_events`; Realtime enqueue events stay in the same bounded event state.
- Free Flow produces an editable draft and requires an explicit click before
  `pty_queue`; it never auto-sends.

## HTTPS gate

Remote LAN browsers require a secure context for microphone APIs. Bootstrap reports
HTTPS enablement, certificate/key presence, and whether both configured files exist;
the browser's `globalThis.isSecureContext` result remains the authoritative mic gate.
Loopback localhost is browser-exempt.

The shared HTTP startup owner must re-export and call
`voice_tls_paths() -> Result<Option<VoiceTlsPaths>, ServerFnError>` before binding:
`None` means HTTP mode, `Some` contains server-only certificate/private-key paths,
and an error means HTTPS was requested but startup must stop because either path/file
is absent. The paths must never be included in a browser response or log. This voice
change defines the startup contract but does not modify the shared listener itself.

## Verification after integration

- `node --check web/app/src/components/domains/voice_realtime/realtime_bridge.js`
- `cargo check -p md-web-app --no-default-features --features web --offline --locked`
- `cargo check -p md-web-app --no-default-features --features server --offline --locked`
- `cargo test -p md-web-services voice_realtime --lib --offline --locked`
- `cargo test -p md-web-contracts voice_realtime --lib --offline --locked`
- `web/e2e/voice_realtime.spec.ts` against `/voice`.
