/* Narrow browser adapter: Rust owns process/transport state; this file owns xterm only. */
const terminals = new Map();
let observer = null;
const JAPANESE_MONO_FONT = '"JetBrains Mono", "BIZ UDGothic", "Noto Sans Mono CJK JP", "Noto Sans CJK JP", "Yu Gothic", Meiryo, ui-monospace, "SFMono-Regular", Menlo, monospace';

function terminalConstructors(options = {}) {
  return {
    TerminalCtor: options.TerminalCtor || globalThis.Terminal,
    FitAddonCtor: options.FitAddonCtor || globalThis.FitAddon?.FitAddon,
    Unicode11AddonCtor: options.Unicode11AddonCtor || globalThis.Unicode11Addon?.Unicode11Addon,
  };
}

export function mountTerminal(element, ptyId, options = {}) {
  if (!element || !ptyId) return false;
  const { TerminalCtor, FitAddonCtor, Unicode11AddonCtor } = terminalConstructors(options);
  if (typeof TerminalCtor !== "function") {
    element.dataset.xtermState = "fallback";
    return false;
  }

  unmountTerminal(ptyId);
  const terminal = new TerminalCtor({
    cursorBlink: true,
    convertEol: false,
    scrollback: 100000,
    allowProposedApi: typeof Unicode11AddonCtor === "function",
    fontFamily: options.fontFamily || JAPANESE_MONO_FONT,
    fontSize: options.fontSize || 14,
    theme: options.theme || {},
  });
  const fitAddon = typeof FitAddonCtor === "function" ? new FitAddonCtor() : null;
  const unicode11Addon = typeof Unicode11AddonCtor === "function" ? new Unicode11AddonCtor() : null;
  if (fitAddon) terminal.loadAddon(fitAddon);
  if (unicode11Addon) {
    terminal.loadAddon(unicode11Addon);
    if (terminal.unicode?.versions?.includes("11")) terminal.unicode.activeVersion = "11";
  }
  terminal.open(element);
  fitAddon?.fit();
  let lastCols = terminal.cols;
  let lastRows = terminal.rows;

  const inputDisposable = terminal.onData((data) => {
    element.dispatchEvent(new CustomEvent("md-terminal-input", {
      bubbles: true,
      detail: { ptyId, data },
    }));
  });
  const emitPresence = (presence) => element.dispatchEvent(new CustomEvent("md-terminal-presence", {
    bubbles: true,
    detail: { ptyId, ...presence, lastActivityAtMs: Date.now() },
  }));
  const compositionStart = () => emitPresence({ composing: true, draftNonempty: true, pickerOpen: false });
  const compositionEnd = () => emitPresence({ composing: false, draftNonempty: false, pickerOpen: false });
  const paste = () => emitPresence({ composing: false, draftNonempty: false, pickerOpen: false });
  element.addEventListener("compositionstart", compositionStart);
  element.addEventListener("compositionend", compositionEnd);
  element.addEventListener("paste", paste);
  const resizeObserver = new ResizeObserver(() => {
    fitAddon?.fit();
    const cols = terminal.cols;
    const rows = terminal.rows;
    if (Number.isInteger(cols) && Number.isInteger(rows) && cols > 0 && rows > 0
      && (cols !== lastCols || rows !== lastRows)) {
      lastCols = cols;
      lastRows = rows;
      element.dispatchEvent(new CustomEvent("md-terminal-resize", {
        bubbles: true,
        detail: { ptyId, cols, rows },
      }));
    }
  });
  resizeObserver.observe(element);

  terminals.set(ptyId, {
    terminal,
    fitAddon,
    inputDisposable,
    resizeObserver,
    presenceListeners: { compositionStart, compositionEnd, paste },
    element,
    generation: 0,
    seq: 0,
  });
  element.dataset.xtermState = "mounted";
  return true;
}

export function applyServerFrame(frame) {
  if (!frame || typeof frame !== "object") return false;
  const type = frame.type;
  const data = frame.data || {};
  const ptyId = data.pty_id || data.pty?.id;
  const state = terminals.get(ptyId);
  if (!state) return false;

  if (type === "attached") {
    if (Number.isInteger(data.generation) && data.generation >= state.generation) {
      const newGeneration = data.generation > state.generation;
      if (newGeneration || data.truncated) state.terminal.reset();
      state.generation = data.generation;
      if (newGeneration || data.truncated) {
        state.seq = Math.max(0, (data.oldest_seq || 1) - 1);
      }
      state.fitAddon?.fit();
      return true;
    }
    return false;
  }
  if (type === "error") {
    if (typeof data.message_ja === "string") {
      state.terminal.writeln(`\r\n\x1b[31m${data.message_ja}\x1b[0m`);
      return true;
    }
    return false;
  }
  if (!Number.isInteger(data.generation) || data.generation !== state.generation) return false;
  if (Number.isInteger(data.seq) && data.seq <= state.seq) return false;
  if (Number.isInteger(data.seq)) state.seq = data.seq;

  if (type === "output" && typeof data.data === "string") {
    state.terminal.write(data.data);
    return true;
  }
  if (type === "exited") {
    const code = data.exit?.exit_code;
    state.terminal.writeln(`\r\n\x1b[90m─ プロセス終了${Number.isInteger(code) ? ` (code ${code})` : ""} ─\x1b[0m`);
    return true;
  }
  if (type === "relaunching") {
    state.terminal.writeln("\r\n\x1b[90m─ 再起動中 ─\x1b[0m");
    return true;
  }
  return false;
}

export function resizeTerminal(ptyId, cols, rows) {
  const state = terminals.get(ptyId);
  if (!state || !Number.isInteger(cols) || !Number.isInteger(rows) || cols < 1 || rows < 1) {
    return false;
  }
  state.terminal.resize(cols, rows);
  return true;
}

export function resetTerminal(ptyId) {
  const state = terminals.get(ptyId);
  if (!state) return false;
  state.terminal.reset();
  state.seq = 0;
  return true;
}

export function unmountTerminal(ptyId) {
  const state = terminals.get(ptyId);
  if (!state) return false;
  state.resizeObserver.disconnect();
  state.element.removeEventListener("compositionstart", state.presenceListeners.compositionStart);
  state.element.removeEventListener("compositionend", state.presenceListeners.compositionEnd);
  state.element.removeEventListener("paste", state.presenceListeners.paste);
  state.inputDisposable.dispose();
  state.terminal.dispose();
  terminals.delete(ptyId);
  return true;
}

export function startTerminalBridge(root = document, options = {}) {
  const mountAll = () => {
    root.querySelectorAll(".pty-terminal__xterm[data-pty-id]").forEach((element) => {
      const ptyId = element.dataset.ptyId;
      if (ptyId && terminals.get(ptyId)?.element !== element) {
        mountTerminal(element, ptyId, options);
      }
    });
    for (const ptyId of terminals.keys()) {
      if (!root.querySelector(`.pty-terminal__xterm[data-pty-id="${CSS.escape(ptyId)}"]`)) {
        unmountTerminal(ptyId);
      }
    }
  };
  mountAll();
  observer?.disconnect();
  observer = new MutationObserver(mountAll);
  observer.observe(root === document ? document.body : root, { childList: true, subtree: true });
  return () => {
    observer?.disconnect();
    observer = null;
    for (const ptyId of [...terminals.keys()]) unmountTerminal(ptyId);
  };
}
