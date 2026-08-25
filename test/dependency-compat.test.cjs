'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { spawnSync } = require('node:child_process');

const root = path.join(__dirname, '..');

test('the updater equality alias preserves same-run cache reuse decisions', async (t) => {
  const cacheDir = fs.mkdtempSync(path.join(os.tmpdir(), 'md-updater-equality-'));
  t.after(() => fs.rmSync(cacheDir, { recursive: true, force: true }));
  const updateFile = path.join(cacheDir, 'update.bin');
  fs.writeFileSync(updateFile, 'update');

  const { DownloadedUpdateHelper } = require('electron-updater/out/DownloadedUpdateHelper');
  const helper = new DownloadedUpdateHelper(cacheDir);
  const versionInfo = { version: '1.2.3', files: [{ url: 'update.bin', sha512: 'same' }] };
  const fileInfo = { info: { url: 'update.bin', sha512: 'same' } };
  await helper.setDownloadedFile(updateFile, null, versionInfo, fileInfo, 'update.bin', false);

  const logger = { info() {}, warn() {} };
  assert.equal(await helper.validateDownloadedPath(updateFile, { ...versionInfo }, { info: { ...fileInfo.info } }, logger), updateFile);
  assert.equal(await helper.validateDownloadedPath(updateFile, { ...versionInfo, version: '1.2.4' }, fileInfo, logger), null);
});

test('@electron/get 3 proxy initialization accepts global-agent 4', () => {
  const proxyModule = path.join(
    root,
    'node_modules/app-builder-lib/node_modules/@electron/get/dist/cjs/proxy.js'
  );
  const child = spawnSync(
    process.execPath,
    ['-e', "require(process.argv[1]).initializeProxy()", proxyModule],
    {
      env: { ...process.env, GLOBAL_AGENT_HTTP_PROXY: 'http://127.0.0.1:9' },
      encoding: 'utf8'
    }
  );
  assert.equal(child.status, 0, child.stderr);
});

test('the tunnel runtime loads with multer 2 compatibility available', () => {
  assert.equal(typeof require('tunnelmole').tunnelmole, 'function');
  const multer = require('multer');
  assert.equal(typeof multer, 'function');
  assert.equal(typeof multer({ dest: os.tmpdir() }).single, 'function');
});
