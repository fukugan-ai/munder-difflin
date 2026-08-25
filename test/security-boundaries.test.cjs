'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { execFileSync } = require('node:child_process');
const loadTs = require('./load-ts.cjs');

const { readFileText, writeFileText, authorizeRootFromAllowlist } = loadTs('src/main/fs.ts');
const { issueListArgs, ciRunListArgs, parseAllowedGitHubRepo } = loadTs('src/main/github.ts');
const { worktreeHasUnintegratedWork } = loadTs('src/main/git.ts');

function workspace() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'md-sec-'));
  const root = path.join(dir, 'repo');
  fs.mkdirSync(root);
  fs.writeFileSync(path.join(dir, 'secret.txt'), 'outside');
  return { dir, root };
}

test('filesystem reads and writes reject a symlink escape', async (t) => {
  if (process.platform === 'win32') return t.skip('symlink creation requires elevated privileges on Windows');
  const { dir, root } = workspace();
  try {
    fs.symlinkSync(dir, path.join(root, 'escape'), 'dir');
    assert.equal((await readFileText(root, 'escape/secret.txt')).ok, false);
    assert.equal((await writeFileText(root, 'escape/new.txt', 'nope')).ok, false);
    assert.equal(fs.existsSync(path.join(dir, 'new.txt')), false);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test('an arbitrary renderer root is not authorized by a main-owned allowlist', () => {
  const { dir, root } = workspace();
  try {
    const allowed = fs.realpathSync(root);
    assert.equal(authorizeRootFromAllowlist(root, [allowed]), allowed);
    assert.equal(authorizeRootFromAllowlist(dir, [allowed]), null);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test('dirty and untracked work makes teardown preservation fail-safe', async () => {
  const { dir, root } = workspace();
  try {
    execFileSync('git', ['init', '-q', '-b', 'main'], { cwd: root });
    execFileSync('git', ['config', 'user.email', 'security-test@example.invalid'], { cwd: root });
    execFileSync('git', ['config', 'user.name', 'Security Test'], { cwd: root });
    fs.writeFileSync(path.join(root, 'tracked.txt'), 'base');
    execFileSync('git', ['add', 'tracked.txt'], { cwd: root });
    execFileSync('git', ['commit', '-qm', 'base'], { cwd: root });
    fs.writeFileSync(path.join(root, 'untracked.txt'), 'preserve me');
    const result = await worktreeHasUnintegratedWork(root, 'main');
    assert.equal(result.keep, true);
    assert.equal(result.dirty, true);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test('GitHub list commands always pin the validated fork with --repo and never request issue bodies', () => {
  const repo = parseAllowedGitHubRepo('git@github.com:fukugan-ai/munder-difflin.git');
  assert.equal(repo, 'fukugan-ai/munder-difflin');
  assert.equal(parseAllowedGitHubRepo('https://github.com/chaitanyagiri/munder-difflin.git'), null);
  assert.equal(parseAllowedGitHubRepo('https://github.com/attacker/repo.git'), null);
  const issueArgs = issueListArgs(repo);
  const ciArgs = ciRunListArgs(repo);
  assert.deepEqual(issueArgs.slice(issueArgs.indexOf('--repo'), issueArgs.indexOf('--repo') + 2), ['--repo', repo]);
  assert.deepEqual(ciArgs.slice(ciArgs.indexOf('--repo'), ciArgs.indexOf('--repo') + 2), ['--repo', repo]);
  assert.equal(issueArgs.join(' ').includes('body'), false);
});

test('renderer assignment treats issue title as delimited data and does not ingest body', () => {
  const src = fs.readFileSync(path.join(__dirname, '..', 'src/renderer/src/components/CommandCenterPanel.tsx'), 'utf8');
  const assign = src.slice(src.indexOf('const assignIssue'), src.indexOf('// Set/clear one agent', src.indexOf('const assignIssue')));
  assert.match(assign, /UNTRUSTED DATA/);
  assert.match(assign, /END ISSUE TITLE/);
  assert.doesNotMatch(assign, /issue\.body/);
});

test('IPC uses main-owned roots and trusted window ownership; teardown has no force removal', () => {
  const main = fs.readFileSync(path.join(__dirname, '..', 'src/main/index.ts'), 'utf8');
  const git = fs.readFileSync(path.join(__dirname, '..', 'src/main/git.ts'), 'utf8');
  assert.match(main, /BrowserWindow\.fromWebContents\(evt\.sender\)/);
  assert.match(main, /readConfig\(\)\.registeredRepos/);
  assert.match(main, /authorizeManagedRoot\(evt, cwd\)/);
  assert.match(main, /finalizeAgentWorktree\(id, wtPath, origCwd, baseBranch\)/);
  assert.doesNotMatch(git, /worktree', 'remove', '--force'/);
});
