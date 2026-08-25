'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

const root = path.join(__dirname, '..');
const shimRoot = path.join(root, 'tools/npm/electron-builder-squirrel-windows');

test('the local Squirrel peer matches app-builder and fails closed when selected', () => {
  const manifest = JSON.parse(fs.readFileSync(path.join(shimRoot, 'package.json'), 'utf8'));
  assert.equal(manifest.name, 'electron-builder-squirrel-windows');
  assert.equal(manifest.version, '26.15.3');
  assert.equal(manifest.private, true);
  assert.equal(manifest.dependencies, undefined);

  const target = require(shimRoot).default;
  assert.throws(
    () => new target(),
    /Squirrel\.Windows is unsupported; use the configured NSIS or portable Windows target\./
  );
});

test('the Windows builder config exposes only NSIS and portable targets', () => {
  const config = fs.readFileSync(path.join(root, 'electron-builder.yml'), 'utf8');
  const windows = config.slice(config.indexOf('\nwin:'), config.indexOf('\nnsis:'));
  assert.match(windows, /target:\s*\n\s*- target: nsis\s*\n\s*arch: \[x64\]\s*\n\s*- target: portable/);
  assert.doesNotMatch(windows, /- target: squirrel/);
});
