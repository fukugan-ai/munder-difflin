# fs_git_ide integration handoff

## Ownership delivered

- Contracts: `web/crates/contracts/src/domains/fs_git_ide/**`
- Server services: `web/crates/services/src/domains/fs_git_ide/**`
- Dioxus component: `web/app/src/components/domains/fs_git_ide/**`
- Styles: `web/app/assets/domains/fs_git_ide.css`
- Browser checks: `web/e2e/fs_git_ide.spec.ts`

## Integration dependencies

The integration owner must keep these shared declarations:

1. `md_web_contracts::domains::fs_git_ide` is public.
2. `md_web_services::domains::fs_git_ide` is registered, and these are re-exported at the crate root:
   `DomainError as FsGitIdeError`, `FsService`, `GitHubService`, `GitService`,
   `WorkspaceRegistry`, `WorktreeProvisioner`.
3. `md-web-services` has a direct `serde_json` dependency.
4. `components::domains::fs_git_ide` is registered and `FsGitIde` is mounted on the workspace route.
5. `app.css` imports `./domains/fs_git_ide.css`.

## Runtime input

- The registered-repository resolver is implemented in `web/app/src/server_fns/workspace.rs`.
  By default it loads `PublicConfig.registered_repos` from the shared PostgreSQL persistence
  repository. A nonempty `MD_REGISTERED_REPOS` platform path-list is the only environment override.
  The browser can select an opaque `WorkspaceId`; it cannot register or submit a root/cwd.
- `git`: local executable; every invocation adds `-c core.hooksPath=/dev/null` (`NUL` on Windows).
- `gh`: optional local executable authenticated for read access to `fukugan-ai/munder-difflin`.
  Missing/unauthenticated `gh` is returned as a UI error and does not weaken the repo allowlist.
- Monaco: `app/assets/domains/fs_git_ide/monaco/vs` contains the project-private pinned runtime,
  workers and language contributions. Dioxus registers it as a folder asset at `/assets/monaco/vs`
  and registers `/assets/monaco_worker.js` without a hash because the Worker API loads it by URL.
  `monaco_bootstrap.js` loads only these same-origin assets;
  there is no CDN or remote fetch. `monaco.rs` owns the narrow state bridge. A textarea appears only
  after an explicit loader failure and is marked `data-editor="degraded"`.
- `FsGitIde { initial_workspace_path: Some(agent.cwd.clone()) }` selects the longest registered
  root containing the Agent cwd. `FsGitIde {}` retains the first-workspace default. The prop is a
  selection hint only: it never registers a path and unmatched paths fall back to the default.

## Server Function routes

- `GET /api/fs-git-ide/workspaces`
- `GET /api/fs-git-ide/list/:workspace_id/:rel_path`
- `GET /api/fs-git-ide/text/:workspace_id/:rel_path`
- `GET /api/fs-git-ide/binary/:workspace_id/:rel_path`
- `POST /api/fs-git-ide/text`
- `POST /api/fs-git-ide/stat-absolute`
- `GET /api/fs-git-ide/git/:workspace_id`
- `GET /api/fs-git-ide/git/main/:workspace_id`
- `GET /api/fs-git-ide/git/diff/:workspace_id/:rel_path`
- `GET /api/fs-git-ide/git/history/:workspace_id/:count/:skip`
- `GET /api/fs-git-ide/git/commit-files/:workspace_id/:revision`
- `GET /api/fs-git-ide/git/show/:workspace_id/:revision/:rel_path`
- `GET /api/fs-git-ide/git/compare/:workspace_id/:base/:head/:three_dot`
- `GET /api/fs-git-ide/git/worktrees/:workspace_id`
- `POST /api/fs-git-ide/git/checkout`
- `GET /api/fs-git-ide/github/issues/:workspace_id`
- `GET /api/fs-git-ide/github/ci/:workspace_id`

All listed routes delegate to existing service capabilities. `git_checkout` derives `repo_busy`
from the process-lifetime PTY list and never accepts that boolean from the browser. It conservatively
refuses when an active Agent cwd is within the selected registered root. Checkout additionally
requires the typed request confirmation, a clean worktree and a validated local ref; it disables
hooks and does no network operation.

The UI exposes binary image preview, paginated commit history, per-commit files and revision text,
2-dot/3-dot comparison, worktree listing, and explicitly confirmed local checkout. Binary image
bytes remain bounded by `FsService` and are encoded locally into a data URL.

## External effects

- File writes are local, root-confined, create/truncate only, and capped at 2 MiB.
- Git is local-only. No fetch, pull, push, commit, tag, or remote mutation is implemented.
- The only GitHub commands are `gh issue list` and `gh run list`; no workflow dispatch/rerun/cancel.
- `parse_allowed_repo` accepts only `fukugan-ai/munder-difflin` and explicitly rejects
  `chaitanyagiri/munder-difflin` and every other repository.

## Cross-domain inputs

- `pty_agents`: process-lifetime registry for checkout busy detection.
- `pty_agents`: create one process-lifetime `WorktreeProvisioner` with an absolute server-owned
  root, then call:
  - `create_isolated_worktree(&WorkspaceRegistry, &ProvisionWorktreeRequest)` before spawn. The
    request contains only `WorkspaceId`, a validated slug-like name, and a local base revision.
    Use the returned `IsolatedWorktree.path` as the PTY cwd and persist its opaque `id` beside the
    agent recipe.
  - `archive(id)` when agent state must be preserved. This performs no Git or filesystem mutation
    and makes later removal through this issuer fail closed.
  - `remove_isolated_worktree(&WorkspaceRegistry, id)` only after the PTY has stopped. It accepts
    no raw path, refuses dirty worktrees, calls `git worktree remove` without `--force`, and
    deliberately preserves the local branch named by the receipt.
  - `get(id)` to resolve an active/archive record inside trusted server integration code.
  `WorktreeProvisioner::new` creates/canonicalizes only its configured root. Provision rollback
  uses non-force `git worktree remove` plus a compare-and-delete `git update-ref` restricted to the
  newly issued branch and its verified base SHA; no fetch/push/remote command exists.
- `config_onboarding`: canonical registered repository list.
- shared event hub: after write/checkout, emit `DomainInvalidated { domain: FsGitIde, revision }` so
  clients refresh. Current component can manually restart resources until the event fan-in lands.

## Verification commands

Run with the task-private `TMPDIR/TMP/TEMP` and `CARGO_TARGET_DIR` already established by the
integration owner:

```text
cargo fmt --all -- --check
cargo check --workspace --all-targets --all-features --offline --locked
cargo clippy --workspace --all-targets --all-features --offline --locked -- -D warnings
cargo test --workspace --all-features --offline --locked
```
