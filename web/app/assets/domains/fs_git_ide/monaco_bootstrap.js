const base = '/assets/monaco/vs';

function ready() {
  globalThis.__mdMonacoLoading = false;
  document.dispatchEvent(new CustomEvent('md-monaco-ready'));
}

function failed() {
  globalThis.__mdMonacoLoading = false;
  document.dispatchEvent(new CustomEvent('md-monaco-failed'));
}

if (globalThis.monaco?.editor?.create) {
  ready();
} else if (!globalThis.__mdMonacoLoading) {
  globalThis.__mdMonacoLoading = true;
  globalThis.MonacoEnvironment = {
    getWorkerUrl() {
      return '/assets/monaco_worker.js';
    },
  };
  const loader = document.createElement('script');
  loader.src = `${base}/loader.js`;
  loader.onload = () => {
    globalThis.require.config({ paths: { vs: base } });
    globalThis.require(['vs/editor/editor.main'], ready, failed);
  };
  loader.onerror = failed;
  document.head.append(loader);
}
