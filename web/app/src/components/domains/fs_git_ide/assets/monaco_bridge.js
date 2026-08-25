const mounted = new WeakMap();

function mount(host) {
  const surface = host.querySelector('[data-monaco-surface]');
  const source = host.querySelector('[data-monaco-source]');
  if (!(surface instanceof HTMLElement) || !(source instanceof HTMLTextAreaElement)) return;

  const previous = mounted.get(host);
  if (previous) previous.dispose();

  const monaco = globalThis.monaco;
  if (!monaco?.editor?.create) {
    host.dataset.editor = globalThis.__mdMonacoLoading ? 'loading' : 'degraded';
    return;
  }

  const editor = monaco.editor.create(surface, {
    value: source.value,
    language: host.dataset.language || 'plaintext',
    readOnly: host.dataset.readonly === 'true',
    automaticLayout: false,
    fontFamily: 'JetBrains Mono, SFMono-Regular, Consolas, monospace',
    fontSize: 12,
    lineHeight: 20,
    minimap: { enabled: false },
    scrollBeyondLastLine: false,
    renderWhitespace: 'selection',
    wordWrap: 'off',
    padding: { top: 8, bottom: 8 },
  });
  source.hidden = true;
  host.dataset.editor = 'monaco';

  const change = editor.onDidChangeModelContent(() => {
    const next = editor.getValue();
    if (source.value === next) return;
    source.value = next;
    source.dispatchEvent(new InputEvent('input', { bubbles: true, inputType: 'insertText' }));
  });
  const sourceChange = () => {
    if (editor.getValue() !== source.value) editor.setValue(source.value);
  };
  source.addEventListener('input', sourceChange);
  editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS, () => {
    host.querySelector('#md-monaco-save-proxy')?.click();
  });
  const resize = new ResizeObserver(() => editor.layout());
  resize.observe(surface);

  mounted.set(host, {
    dispose() {
      resize.disconnect();
      source.removeEventListener('input', sourceChange);
      change.dispose();
      editor.dispose();
      source.hidden = false;
    },
  });
}

function scan(root = document) {
  root.querySelectorAll?.('[data-monaco-island]').forEach(mount);
}

scan();
document.addEventListener('md-monaco-ready', () => scan());
document.addEventListener('md-monaco-failed', () => {
  document.querySelectorAll('[data-monaco-island]').forEach((host) => {
    if (host.dataset.editor !== 'monaco') host.dataset.editor = 'degraded';
  });
});
new MutationObserver((records) => {
  for (const record of records) {
    for (const node of record.addedNodes) {
      if (!(node instanceof Element)) continue;
      if (node.matches('[data-monaco-island]')) mount(node);
      scan(node);
    }
  }
}).observe(document.documentElement, { childList: true, subtree: true });
