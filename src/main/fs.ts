import { constants, lstatSync, realpathSync } from 'node:fs';
import { lstat, open, readdir, realpath } from 'node:fs/promises';
import { dirname, isAbsolute, join, normalize, relative, resolve, sep } from 'node:path';
import { homedir } from 'node:os';
import { imageMimeForPath } from '../shared/imageTypes';

/**
 * Confines `path` inside `root` to prevent path-traversal escapes.
 * Returns the resolved absolute path on success, or null on violation.
 *
 * Exported so other main-process modules (e.g. git.ts) validate caller-supplied
 * relative paths against a workspace root with the SAME guard — there is exactly
 * one path-escape policy in the app and it lives here.
 */
export function safeJoin(root: string, rel: string): string | null {
  const absRoot = resolve(root);
  const absPath = isAbsolute(rel) ? normalize(rel) : resolve(absRoot, rel);
  const rel2 = relative(absRoot, absPath);
  if (rel2.startsWith('..') || isAbsolute(rel2)) return null;
  return absPath;
}

/** Exact-root capability check used by main-owned IPC allowlists. */
export function authorizeRootFromAllowlist(requested: string, allowedRoots: readonly string[]): string | null {
  try {
    if (!requested || requested.includes('\0') || lstatSync(requested).isSymbolicLink()) return null;
    const canonical = realpathSync(requested);
    return allowedRoots.includes(canonical) ? canonical : null;
  } catch { return null; }
}

function inside(root: string, candidate: string): boolean {
  const rel = relative(root, candidate);
  return rel === '' || (!rel.startsWith('..') && !isAbsolute(rel));
}

/** Resolve a renderer-selected path without following symlinks at any component. */
async function securePath(root: string, rel: string, allowMissingLeaf = false): Promise<string | null> {
  const lexical = safeJoin(root, rel);
  if (!lexical) return null;
  try {
    const rootStat = await lstat(root);
    if (rootStat.isSymbolicLink() || !rootStat.isDirectory()) return null;
    const canonicalRoot = await realpath(root);
    const relativeParts = relative(resolve(root), lexical).split(sep).filter(Boolean);
    let cursor = canonicalRoot;
    for (let i = 0; i < relativeParts.length; i++) {
      cursor = join(cursor, relativeParts[i]);
      try {
        const s = await lstat(cursor);
        if (s.isSymbolicLink()) return null;
      } catch {
        if (!(allowMissingLeaf && i === relativeParts.length - 1)) return null;
      }
    }
    const boundaryTarget = allowMissingLeaf ? await realpath(dirname(cursor)) : await realpath(cursor);
    return inside(canonicalRoot, boundaryTarget) ? cursor : null;
  } catch {
    return null;
  }
}

const PATH_ERROR = 'path is unavailable or outside the authorized workspace';
const IO_ERROR = 'file operation failed';

export interface DirEntry {
  name: string;
  isDir: boolean;
  size: number;
  mtime: number;
}

export async function listDir(root: string, rel: string): Promise<{
  ok: true; entries: DirEntry[]; path: string;
} | { ok: false; error: string }> {
  const abs = await securePath(root, rel);
  if (!abs) return { ok: false, error: PATH_ERROR };
  try {
    const names = await readdir(abs);
    const entries = await Promise.all(names.map(async (name): Promise<DirEntry> => {
      try {
        const s = await lstat(join(abs, name));
        if (s.isSymbolicLink()) return { name, isDir: false, size: 0, mtime: 0 };
        return { name, isDir: s.isDirectory(), size: s.size, mtime: s.mtimeMs };
      } catch {
        return { name, isDir: false, size: 0, mtime: 0 };
      }
    }));
    entries.sort((a, b) => {
      if (a.isDir !== b.isDir) return a.isDir ? -1 : 1;
      return a.name.localeCompare(b.name);
    });
    return { ok: true, entries, path: abs };
  } catch (e) {
    return { ok: false, error: IO_ERROR };
  }
}

const MAX_READ_BYTES = 2 * 1024 * 1024; // 2 MB

export async function readFileText(root: string, rel: string): Promise<{
  ok: true; content: string; path: string; size: number;
} | { ok: false; error: string }> {
  const abs = await securePath(root, rel);
  if (!abs) return { ok: false, error: PATH_ERROR };
  try {
    const handle = await open(abs, constants.O_RDONLY | constants.O_NOFOLLOW);
    const s = await handle.stat();
    if (s.size > MAX_READ_BYTES) {
      await handle.close();
      return { ok: false, error: `file too large (${(s.size / 1024 / 1024).toFixed(1)} MB)` };
    }
    const buf = await handle.readFile();
    await handle.close();
    // Reject obvious binary files based on null-byte sniff
    if (buf.includes(0)) return { ok: false, error: 'binary file (not displayable)' };
    return { ok: true, content: buf.toString('utf8'), path: abs, size: s.size };
  } catch (e) {
    return { ok: false, error: IO_ERROR };
  }
}

/**
 * Ceiling for the BINARY read. Deliberately larger than MAX_READ_BYTES (2 MB):
 * that cap exists to stop Monaco choking on a huge text buffer, and applying it
 * to images would reject the exact files people want to look at — a retina
 * screenshot of a full 5K display is routinely 3–6 MB. 10 MB covers real
 * screenshots and design assets while still refusing to hand the renderer a
 * video-sized payload over structured clone.
 */
const MAX_BINARY_READ_BYTES = 10 * 1024 * 1024; // 10 MB

/**
 * Read a file as raw BYTES, confined to `root` by the same `safeJoin` guard as
 * every other fs entry point here.
 *
 * Exists because the text path deliberately refuses binary content (the
 * null-byte sniff in readFileText), which meant a PNG in an agent's workspace
 * was completely unviewable inside the app — the IDE opened a tab that said
 * "binary file (not displayable)" and stopped there. The renderer cannot reach
 * the file itself: the CSP is `default-src 'self'` with no `file:` source and
 * there is no registered file protocol, so `<img src="file://…">` silently
 * fails. Bytes therefore have to travel over IPC, and the renderer turns them
 * into a `blob:` URL (already allowed by `img-src`).
 *
 * NEVER reads unbounded: the size is checked from stat BEFORE opening, and
 * re-checked against the bytes actually read so a file that grows between the
 * two calls can't slip past the cap.
 */
export async function readFileBinary(root: string, rel: string, maxBytes = MAX_BINARY_READ_BYTES): Promise<{
  ok: true; bytes: Uint8Array<ArrayBuffer>; mime: string; path: string; size: number;
} | { ok: false; error: string }> {
  const abs = await securePath(root, rel);
  if (!abs) return { ok: false, error: PATH_ERROR };
  try {
    const handle = await open(abs, constants.O_RDONLY | constants.O_NOFOLLOW);
    const s = await handle.stat();
    // Directories and FIFOs are the trap here: readFile on a directory throws
    // (fine) but on a FIFO it BLOCKS forever with no size to check against, which
    // would hang the IPC call and, with it, the renderer's loading state.
    if (!s.isFile()) { await handle.close(); return { ok: false, error: 'not a regular file' }; }
    if (s.size > maxBytes) {
      await handle.close();
      return { ok: false, error: `file too large (${(s.size / 1024 / 1024).toFixed(1)} MB)` };
    }
    const buf = await handle.readFile();
    await handle.close();
    if (buf.byteLength > maxBytes) {
      // The file grew between stat and read. Rare, but the cap is a memory
      // guarantee for the renderer, not an advisory.
      return { ok: false, error: 'file grew past the size limit while reading' };
    }
    // COPY into a freshly-allocated Uint8Array instead of forwarding the Buffer.
    // Node serves small reads out of a shared 8 KB Buffer pool, so a pooled
    // Buffer is a VIEW onto memory that also holds unrelated recently-read
    // bytes; structured-clone carries the whole backing ArrayBuffer across the
    // IPC boundary, not just the view. Copying keeps the renderer's payload
    // exactly the file and nothing else.
    const bytes = new Uint8Array(buf.byteLength);
    bytes.set(buf);
    return {
      ok: true,
      bytes,
      mime: imageMimeForPath(abs) ?? 'application/octet-stream',
      path: abs,
      size: s.size
    };
  } catch (e) {
    return { ok: false, error: IO_ERROR };
  }
}

export async function writeFileText(root: string, rel: string, content: string): Promise<{
  ok: true; path: string;
} | { ok: false; error: string }> {
  const abs = await securePath(root, rel, true);
  if (!abs) return { ok: false, error: PATH_ERROR };
  try {
    const handle = await open(
      abs,
      constants.O_WRONLY | constants.O_CREAT | constants.O_TRUNC | constants.O_NOFOLLOW,
      0o600
    );
    await handle.writeFile(content, 'utf8');
    await handle.close();
    return { ok: true, path: abs };
  } catch {
    return { ok: false, error: IO_ERROR };
  }
}

/**
 * Expand a leading `~` to the user's home dir and return an absolute, normalized
 * path. SYNC on purpose — every consumer (spawn guards, cwd validation, config
 * writes) is synchronous.
 *
 * Only a SHELL expands `~`; Node's `fs`/`child_process` treat it as a literal
 * directory name, so a user-typed `~/dev/foo` fails every existsSync/statSync and
 * dies with `cwd does not exist`. This is applied at INGESTION (project add,
 * `pty:spawn`) so the registry only ever stores an ABSOLUTE cwd, with
 * defense-in-depth at the consumers.
 *
 * Non-tilde absolute paths are returned resolved/normalized. Anything that is
 * still RELATIVE after expansion is returned untouched (trimmed) so callers keep
 * their own "not-absolute" errors instead of it being silently resolved against
 * the Electron process cwd. Empty input passes straight through. Windows paths
 * (`C:\…`, UNC) are unaffected — they never start with `~`.
 */
export function expandTilde(p: string): string {
  if (typeof p !== 'string') return p;
  const t = p.trim();
  if (!t) return p;
  let out = t;
  if (t === '~') out = homedir();
  else if (t.startsWith('~/') || t.startsWith('~\\')) out = join(homedir(), t.slice(2));
  if (!isAbsolute(out)) return t;
  return resolve(out);
}

/**
 * Normalize the hive home and its recent-list in one place (#140).
 *
 * Onboarding SUGGESTS `~/HarnessAgents` in a free-text field, so the single most
 * common setup path — accept the default, press Finish — used to persist a literal
 * `~`. Finish immediately creates that directory, and Node's mkdir has no concept
 * of `~`: it tried to create a folder literally named "~" and died with
 * `ENOENT: no such file or directory, mkdir '~/HarnessAgents'`, wedging the wizard
 * on its last step. Expanding at the config-write boundary means every downstream
 * reader — mkdir, the hive root, the launch picker — sees one absolute path.
 *
 * `prior` is normalized too: entries written before this existed would otherwise
 * let the launch picker hand a stale `~/…` string straight back and reintroduce
 * the same failure. Deduped against the new home, newest first, capped.
 */
export function normalizeHiveHome(
  home: string,
  prior: readonly string[] = [],
  cap = 8
): { home: string; recentHives: string[] } {
  const abs = expandTilde(home);
  const seen = new Set<string>([abs]);
  const recentHives = [abs];
  for (const h of prior) {
    if (typeof h !== 'string' || !h.trim()) continue;
    const e = expandTilde(h);
    if (seen.has(e)) continue;
    seen.add(e);
    recentHives.push(e);
  }
  return { home: abs, recentHives: recentHives.slice(0, cap) };
}

/** Existence/metadata check for an ABSOLUTE path (v0.3.4 — backs the terminal
 *  ⌘-click markdown flow). `~/` is expanded here (the renderer doesn't know
 *  the home dir). Read-only metadata: returns whether a regular file exists and
 *  the normalized absolute path; never file contents. */
export async function statAbs(p: string, authorizedRoot?: string): Promise<{ exists: boolean; isFile: boolean; path: string }> {
  const abs = expandTilde(p);
  if (!isAbsolute(abs) || !authorizedRoot) return { exists: false, isFile: false, path: '' };
  try {
    const secured = await securePath(authorizedRoot, abs);
    if (!secured) return { exists: false, isFile: false, path: '' };
    const s = await lstat(secured);
    return { exists: true, isFile: s.isFile(), path: secured };
  } catch {
    return { exists: false, isFile: false, path: '' };
  }
}
