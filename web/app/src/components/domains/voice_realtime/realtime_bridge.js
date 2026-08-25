/* Narrow browser adapter: owns media/WebRTC only. Long-lived credentials never enter this module. */

const EVENT_NAME = "munder:voice-realtime-event";
const FREEFLOW_EVENT_NAME = "munder:voice-freeflow-event";

let recorder = null;
let recorderStream = null;
let recorderChunks = [];
let recorderCompletion = null;
let recorderResolve = null;
let peer = null;
let realtimeStream = null;
let audioElement = null;
let dataChannel = null;
let idleTimer = null;
let idleDisconnectMs = 180000;
let shortcutEnabled = false;
let shortcutTargetAgentId = null;
let shortcutInputDeviceId = null;
let optionDown = false;
let optionArmTimer = null;
let optionDisqualified = false;
let shortcutRecording = false;
const OPTION_ARM_MS = 320;
const INPUT_DEVICE_STORAGE_KEY = "munder.voice.inputDeviceId";
const OUTPUT_DEVICE_STORAGE_KEY = "munder.voice.outputDeviceId";

const ACTION_VERBS = [
  "ping", "dispatch", "steer", "create_task", "assign_task", "update_task",
  "delete_task", "wait_for", "spawn", "kill", "pause", "halt", "resume",
  "auto_delivery", "gate_tool", "archive", "unarchive", "clear_context",
  "edit_schedule", "create_schedule", "update_setting",
];

const REALTIME_TOOLS = [
  {
    type: "function",
    name: "munder_action",
    description: "Operate one local Munder Difflin agent, task, schedule, or setting. Never target all agents.",
    parameters: {
      type: "object",
      properties: {
        verb: { type: "string", enum: ACTION_VERBS },
        agentId: { type: "string" },
        taskId: { type: "string" },
        text: { type: "string" },
        title: { type: "string" },
        objective: { type: "string" },
        provider: { type: "string" },
        settingKey: {
          type: "string",
          enum: ["notifications", "freeflowEnabled", "strongKeepalive", "autoUpdate", "autoMode", "semanticMemory", "terminalTheme", "realtimeIdleDisconnectMs", "defaultModel", "godModel", "godProvider"],
        },
        settingValue: { type: "string" },
        toolName: { type: "string" },
        enabled: { type: "boolean" },
        spawnRequest: {
          type: "object",
          properties: {
            id: { type: "string" },
            name: { type: "string" },
            provider: { type: "string", enum: ["claude", "codex", "grok", "kimi", "gemini", "antigravity", "qwen", "open_code", "crush", "pi", "copilot", "cursor", "custom"] },
            role: {
              type: "object",
              properties: { orchestrator: { type: "boolean" }, assistant: { type: "boolean" } },
              required: ["orchestrator", "assistant"],
            },
            description: { type: "string" },
            cwd: { type: "string" },
            command: { type: "string" },
            args: { type: "array", items: { type: "string" } },
            model: { type: "string" },
            cols: { type: "integer", minimum: 20, maximum: 500 },
            rows: { type: "integer", minimum: 5, maximum: 200 },
            isolate: { type: "boolean" },
            resume: { type: "boolean" },
            require_resume: { type: "boolean" },
            resume_session_id: { type: "string" },
          },
          required: ["id", "name", "provider", "role", "description", "cwd", "command", "args", "cols", "rows", "isolate", "resume", "require_resume"],
        },
        mission: {
          type: "object",
          properties: {
            id: { type: "string" }, label: { type: "string" },
            interval_ms: { type: "integer", minimum: 0 },
            weekly: {
              type: "object",
              properties: {
                days: { type: "array", items: { type: "integer", minimum: 0, maximum: 6 } },
                minute: { type: "integer", minimum: 0, maximum: 1439 },
              },
              required: ["days", "minute"],
            },
            to: { type: "string" }, body: { type: "string" }, enabled: { type: "boolean" },
            last_fired_at_ms: { type: "integer" },
            kind: { type: "string", enum: ["Dispatch", "Heartbeat", "Compact"] },
            quiet_threshold_ms: { type: "integer" },
          },
          required: ["id", "label", "interval_ms", "to", "body", "enabled", "kind"],
        },
      },
      required: ["verb"],
    },
  },
  {
    type: "function",
    name: "confirm_action",
    description: "Confirm the single pending Munder Difflin action after the user says the requested confirmation phrase.",
    parameters: {
      type: "object",
      properties: {
        pendingId: { type: "string" },
        phrase: { type: "string" },
      },
      required: ["pendingId", "phrase"],
    },
  },
  {
    type: "function",
    name: "cancel_action",
    description: "Cancel the pending Munder Difflin action.",
    parameters: { type: "object", properties: {} },
  },
];

function emit(name, detail) {
  globalThis.dispatchEvent(new CustomEvent(name, { detail }));
}

function stopTracks(stream) {
  for (const track of stream?.getTracks?.() ?? []) track.stop();
}

function pickMimeType() {
  for (const mime of ["audio/webm;codecs=opus", "audio/webm", "audio/ogg;codecs=opus"]) {
    if (globalThis.MediaRecorder?.isTypeSupported?.(mime)) return mime;
  }
  return "";
}

async function enumerateDevices() {
  const devices = await navigator.mediaDevices?.enumerateDevices?.();
  return (devices ?? [])
    .filter((device) => device.kind === "audioinput" || device.kind === "audiooutput")
    .map((device, index) => ({
      id: device.deviceId,
      kind: device.kind === "audioinput" ? "input" : "output",
      label: device.label || `${device.kind === "audioinput" ? "Microphone" : "Speaker"} ${index + 1}`,
    }));
}

function storedDeviceId(key) {
  try { return globalThis.localStorage?.getItem(key) || null; } catch { return null; }
}

function persistDeviceId(key, deviceId) {
  try {
    if (deviceId) globalThis.localStorage?.setItem(key, deviceId);
    else globalThis.localStorage?.removeItem(key);
  } catch { /* browser storage may be disabled */ }
}

async function publishCapabilities() {
  emit(EVENT_NAME, {
    type: "capabilities",
    secureContext: globalThis.isSecureContext === true,
    devices: await enumerateDevices().catch(() => []),
    inputDeviceId: storedDeviceId(INPUT_DEVICE_STORAGE_KEY),
    outputDeviceId: storedDeviceId(OUTPUT_DEVICE_STORAGE_KEY),
  });
}

function isOptionKey(event) {
  return event.code === "AltLeft" || event.code === "AltRight" || event.key === "Alt";
}

function clearOptionArm() {
  clearTimeout(optionArmTimer);
  optionArmTimer = null;
}

function resetOptionShortcut() {
  clearOptionArm();
  optionDown = false;
  optionDisqualified = false;
  if (shortcutRecording && recorder?.state === "recording") {
    recorder.stop();
    emit(FREEFLOW_EVENT_NAME, { type: "shortcut-stopped" });
  }
  shortcutRecording = false;
}

function configureFreeflowShortcut({ enabled, targetAgentId = null, inputDeviceId = null }) {
  shortcutEnabled = enabled === true;
  shortcutTargetAgentId = targetAgentId || null;
  shortcutInputDeviceId = inputDeviceId || null;
  if (!shortcutEnabled) resetOptionShortcut();
}

function onShortcutKeyDown(event) {
  if (!shortcutEnabled) return;
  if (event.isComposing || event.key === "Process" || event.key === "AltGraph") {
    if (optionDown && !shortcutRecording) {
      optionDisqualified = true;
      clearOptionArm();
    }
    return;
  }
  if (!isOptionKey(event)) {
    if (optionDown && !shortcutRecording) {
      optionDisqualified = true;
      clearOptionArm();
    }
    return;
  }
  if (event.repeat || optionDown) return;
  optionDown = true;
  optionDisqualified = recorder !== null || !shortcutTargetAgentId;
  if (optionDisqualified) return;
  optionArmTimer = setTimeout(async () => {
    optionArmTimer = null;
    if (!optionDown || optionDisqualified || !shortcutTargetAgentId) return;
    try {
      await startFreeflow(shortcutTargetAgentId, shortcutInputDeviceId);
      shortcutRecording = true;
      emit(FREEFLOW_EVENT_NAME, { type: "shortcut-started", targetAgentId: shortcutTargetAgentId });
      if (!optionDown && recorder?.state === "recording") recorder.stop();
    } catch {
      shortcutRecording = false;
      emit(FREEFLOW_EVENT_NAME, { type: "shortcut-error", message: "マイクを開始できませんでした" });
    }
  }, OPTION_ARM_MS);
}

function onShortcutKeyUp(event) {
  if (!isOptionKey(event)) return;
  clearOptionArm();
  optionDown = false;
  optionDisqualified = false;
  if (shortcutRecording && recorder?.state === "recording") {
    recorder.stop();
    emit(FREEFLOW_EVENT_NAME, { type: "shortcut-stopped" });
  }
  shortcutRecording = false;
}

async function startFreeflow(targetAgentId, inputDeviceId = null) {
  if (!globalThis.isSecureContext) throw new Error("HTTPS_REQUIRED");
  if (recorder) return;
  const audio = {
    echoCancellation: true,
    noiseSuppression: true,
    autoGainControl: true,
  };
  if (inputDeviceId) audio.deviceId = { exact: inputDeviceId };
  recorderStream = await navigator.mediaDevices.getUserMedia({ audio });
  recorderChunks = [];
  recorderCompletion = new Promise((resolve) => {
    recorderResolve = resolve;
  });
  const mimeType = pickMimeType();
  recorder = mimeType
    ? new MediaRecorder(recorderStream, { mimeType })
    : new MediaRecorder(recorderStream);
  recorder.addEventListener("dataavailable", (event) => {
    if (event.data?.size) recorderChunks.push(event.data);
  });
  recorder.addEventListener("stop", async () => {
    const type = recorder?.mimeType || "audio/webm";
    const blob = new Blob(recorderChunks, { type });
    recorderChunks = [];
    recorder = null;
    stopTracks(recorderStream);
    recorderStream = null;
    const result = {
      type: "recording-ready",
      targetAgentId,
      audio: await blob.arrayBuffer(),
      mimeType: type.split(";")[0],
      filename: type.includes("ogg") ? "dictation.ogg" : "dictation.webm",
    };
    recorderResolve?.(result);
    recorderResolve = null;
    emit(FREEFLOW_EVENT_NAME, result);
  }, { once: true });
  recorder.start();
  emit(FREEFLOW_EVENT_NAME, { type: "recording-started", targetAgentId });
}

async function stopFreeflow() {
  if (recorder?.state === "recording") recorder.stop();
  const completion = recorderCompletion;
  const result = completion ? await completion : null;
  recorderCompletion = null;
  return result;
}

function resetIdleTimer() {
  clearTimeout(idleTimer);
  idleTimer = setTimeout(() => {
    disconnectRealtime();
    emit(EVENT_NAME, { type: "idle-disconnect" });
  }, idleDisconnectMs);
}

async function connectRealtime({ ephemeralToken, inputDeviceId = null, outputDeviceId = null, idleMs = 180000 }) {
  if (!globalThis.isSecureContext) throw new Error("HTTPS_REQUIRED");
  if (peer) return;
  idleDisconnectMs = Math.max(30000, Math.min(Number(idleMs) || 180000, 3600000));
  const audio = {
    echoCancellation: true,
    noiseSuppression: true,
    autoGainControl: true,
  };
  if (inputDeviceId) audio.deviceId = { exact: inputDeviceId };
  realtimeStream = await navigator.mediaDevices.getUserMedia({ audio });
  peer = new RTCPeerConnection();
  for (const track of realtimeStream.getTracks()) peer.addTrack(track, realtimeStream);

  audioElement = new Audio();
  audioElement.autoplay = true;
  if (outputDeviceId && typeof audioElement.setSinkId === "function") {
    await audioElement.setSinkId(outputDeviceId).catch(() => undefined);
  }
  peer.ontrack = (event) => {
    audioElement.srcObject = event.streams[0] ?? new MediaStream([event.track]);
  };
  peer.onconnectionstatechange = () => emit(EVENT_NAME, {
    type: "connection-state",
    state: peer?.connectionState ?? "closed",
  });
  dataChannel = peer.createDataChannel("oai-events");
  dataChannel.addEventListener("open", () => {
    sendRealtimeEvent({
      type: "session.update",
      session: {
        type: "realtime",
        instructions: "You are Michael, a concise Japanese voice operator for this local Munder Difflin workspace. Use tools for every mutation. Never claim success before the tool result. Ask for one exact target when ambiguous.",
        tools: REALTIME_TOOLS,
        tool_choice: "auto",
      },
    });
  }, { once: true });
  dataChannel.onmessage = (event) => {
    resetIdleTimer();
    try {
      const parsed = JSON.parse(event.data);
      emit(EVENT_NAME, { type: "server-event", event: parsed });
      if (parsed?.type === "response.done") {
        const usage = parsed.response?.usage ?? {};
        emit(EVENT_NAME, {
          type: "usage",
          inputTokens: Number(usage.input_tokens) || 0,
          outputTokens: Number(usage.output_tokens) || 0,
        });
      }
    } catch {
      emit(EVENT_NAME, { type: "protocol-error", message: "invalid realtime event" });
    }
  };
  const offer = await peer.createOffer();
  await peer.setLocalDescription(offer);
  const response = await fetch("https://api.openai.com/v1/realtime/calls", {
    method: "POST",
    headers: {
      Authorization: `Bearer ${ephemeralToken}`,
      "Content-Type": "application/sdp",
    },
    body: offer.sdp,
  });
  if (!response.ok) throw new Error(`REALTIME_CONNECT_${response.status}`);
  await peer.setRemoteDescription({ type: "answer", sdp: await response.text() });
  resetIdleTimer();
}

function sendRealtimeEvent(event) {
  if (dataChannel?.readyState !== "open") return false;
  dataChannel.send(JSON.stringify(event));
  return true;
}

function sendRealtimeToolResult(callId, output) {
  if (!sendRealtimeEvent({
    type: "conversation.item.create",
    item: { type: "function_call_output", call_id: callId, output },
  })) return false;
  return sendRealtimeEvent({ type: "response.create" });
}

function setRealtimeMuted(muted) {
  for (const track of realtimeStream?.getAudioTracks?.() ?? []) track.enabled = !muted;
}

async function setOutputDevice(deviceId) {
  persistDeviceId(OUTPUT_DEVICE_STORAGE_KEY, deviceId);
  if (audioElement && typeof audioElement.setSinkId === "function") {
    await audioElement.setSinkId(deviceId || "");
  }
}

function setInputDevice(deviceId) {
  persistDeviceId(INPUT_DEVICE_STORAGE_KEY, deviceId);
}

function disconnectRealtime() {
  clearTimeout(idleTimer);
  idleTimer = null;
  dataChannel?.close();
  dataChannel = null;
  peer?.close();
  peer = null;
  stopTracks(realtimeStream);
  realtimeStream = null;
  if (audioElement) {
    audioElement.pause();
    audioElement.srcObject = null;
  }
  audioElement = null;
  emit(EVENT_NAME, { type: "connection-state", state: "closed" });
}

globalThis.munderVoiceBridge = Object.freeze({
  isSecureContext: () => globalThis.isSecureContext === true,
  enumerateDevices,
  configureFreeflowShortcut,
  startFreeflow,
  stopFreeflow,
  connectRealtime,
  sendRealtimeEvent,
  sendRealtimeToolResult,
  setRealtimeMuted,
  setInputDevice,
  setOutputDevice,
  disconnectRealtime,
});

globalThis.addEventListener?.("keydown", onShortcutKeyDown, true);
globalThis.addEventListener?.("keyup", onShortcutKeyUp, true);
globalThis.addEventListener?.("blur", resetOptionShortcut);
globalThis.navigator?.mediaDevices?.addEventListener?.("devicechange", publishCapabilities);
queueMicrotask(publishCapabilities);

export { isOptionKey };
