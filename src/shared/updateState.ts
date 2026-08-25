/**
 * Auto-update status model + presentation mapping.
 *
 * Deliberately electron-free: main (src/main/updater.ts) produces these states
 * from electron-updater events, the toolbar badge
 * (src/renderer/src/components/UpdateBadge.tsx) renders them, and the rules that
 * matter — which state wins when two arrive out of order, what the button says
 * and does — live here where they can be unit-tested without booting Electron.
 */

export type UpdateStatus =
  /** Nothing known yet (fresh window, or dev build where we never check). */
  | { state: 'idle' }
  | { state: 'checking' }
  | { state: 'not-available' }
  | { state: 'available'; version: string; notes?: string }
  | { state: 'downloading'; version: string; percent: number }
  | { state: 'downloaded'; version: string; notes?: string }
  /** This install can't self-update (win-portable, or the native path failed):
   *  notify-only, link to the release page. `reason` is the underlying error.
   *  `notes` is the release body — the notify-only poll already reads the same
   *  `releases/latest` JSON that carries it, so the toast can show "what's new"
   *  here too without a second request. */
  | { state: 'available-manual'; version: string; url: string; reason?: string; notes?: string;
      /** Direct asset for THIS platform/arch, when the release has one. The
       *  modal's primary button downloads it; without it the button falls back
       *  to the releases page. */
      downloadUrl?: string }
  /** First launch after the version moved: `version` is the one now RUNNING and
   *  `notes` its release body, so the renderer can show that release's page. */
  | { state: 'just-updated'; version: string; notes?: string }
  | { state: 'error'; message: string };

export type UpdateAction = 'none' | 'check' | 'download' | 'restart' | 'open-release' | 'manual';

export const REPO = 'chaitanyagiri/munder-difflin';

/** The installer for THIS machine in the release tagged v{version}, by the
 *  names electron-builder.yml produces. Used when a status carries no
 *  `downloadUrl` of its own (the native updater path never does). */
export function installerUrl(version: string, platform: string, arch: string): string {
  const v = version.replace(/^v/, '');
  const file = platform === 'darwin' ? `Munder-Difflin-${v}-mac-${arch}.dmg`
    : platform === 'win32' ? `Munder-Difflin-${v}-win-x64-setup.exe`
    : `Munder-Difflin-${v}-linux-x86_64.AppImage`;
  return `https://github.com/${REPO}/releases/download/v${v}/${file}`;
}

/** The newer release a status knows about, or null. Every state that names a
 *  version newer than the running one counts, whatever the updater is doing
 *  with it: the manual path is always on offer. */
export function pendingVersion(status: UpdateStatus | null, current: string): string | null {
  if (!status || !('version' in status)) return null;
  if (status.state === 'just-updated') return null;
  return isNewer(status.version, current) ? status.version : null;
}

/** Where a manual download of `status`'s release goes: the asset the release
 *  itself named when it did, else the conventional installer URL. */
export function manualDownloadUrl(status: UpdateStatus, platform: string, arch: string): string | null {
  if (!('version' in status) || status.state === 'just-updated') return null;
  if (status.state === 'available-manual' && status.downloadUrl) return status.downloadUrl;
  return installerUrl(status.version, platform, arch);
}

export interface UpdateBadgeView {
  /** Extra text beside the version, or null to show the version alone. */
  label: string | null;
  /** What a click does. 'none' renders the badge non-interactive. */
  action: UpdateAction;
  tone: 'idle' | 'busy' | 'ready' | 'warn';
  /** Tooltip — the only place the underlying error is ever surfaced verbatim. */
  title: string;
  busy: boolean;
}

/** `1.2.3` / `v1.2.3` -> [1,2,3]; null for anything that isn't semver-ish. */
export function parseVersion(v: string): [number, number, number] | null {
  const m = String(v ?? '').trim().replace(/^v/, '').match(/^(\d+)\.(\d+)\.(\d+)/);
  return m ? [Number(m[1]), Number(m[2]), Number(m[3])] : null;
}

export function isNewer(candidate: string, current: string): boolean {
  const a = parseVersion(candidate);
  const b = parseVersion(current);
  if (!a || !b) return false;
  for (let i = 0; i < 3; i++) {
    if (a[i] !== b[i]) return a[i] > b[i];
  }
  return false;
}

/** Download percentages arrive as floats and, on a resumed/differential
 *  download, occasionally out of range. Clamp so the UI can't render `-0%`
 *  or `104%`. */
export function clampPercent(n: number): number {
  if (!Number.isFinite(n)) return 0;
  return Math.max(0, Math.min(100, Math.round(n)));
}

/** How far along the update pipeline a state is. A later stage is never
 *  replaced by an earlier one for the SAME version — see `reduceStatus`. */
function rank(s: UpdateStatus): number {
  switch (s.state) {
    case 'idle': return 0;
    case 'checking': return 1;
    case 'not-available': return 1;
    case 'error': return 1;
    case 'just-updated': return 1;
    case 'available-manual': return 2;
    case 'available': return 3;
    case 'downloading': return 4;
    case 'downloaded': return 5;
  }
}

function versionOf(s: UpdateStatus): string | null {
  return 'version' in s ? s.version : null;
}

/**
 * Fold a new status into the current one.
 *
 * The rule that matters: once an update is staged, the 6-hourly re-check (or a
 * manual "check now") must NOT wipe the "restart to update" affordance out from
 * under the user — `checking` / `not-available` / a transient `error` are all
 * lower-rank and lose. A genuinely NEWER version always wins, so a long-running
 * app that sees 0.3.7 while 0.3.6 is staged moves forward rather than sticking.
 */
export function reduceStatus(prev: UpdateStatus | null, next: UpdateStatus): UpdateStatus {
  if (!prev) return next;
  const pv = versionOf(prev);
  const nv = versionOf(next);
  if (pv && nv && isNewer(nv, pv)) return next;   // a newer release supersedes
  if (pv && nv && pv !== nv) return next;         // different (e.g. rolled back) release
  return rank(next) >= rank(prev) ? next : prev;
}

/**
 * What the toolbar badge shows and does for a given status.
 *
 * `currentVersion` is the running app's version — it is always rendered next to
 * the logo, so every one of these views is "v0.3.6" plus at most one extra chip.
 */
export function describeUpdate(status: UpdateStatus | null, currentVersion: string): UpdateBadgeView {
  const v = currentVersion;
  // The title-bar badge is the MANUAL path, always: click downloads the
  // installer and the user replaces the app. Auto-update (download, restart)
  // lives in Settings -> Updates. So any state that names a newer release reads
  // the same here, whatever the background updater is doing with it.
  if (status?.state === 'downloading') {
    // Settings started the automatic download; the chip reports progress and
    // nothing else, so the two paths are not raced against each other.
    return {
      label: `ダウンロード中 ${clampPercent(status.percent)}%`, action: 'none', tone: 'busy', busy: true,
      title: `v${status.version}をダウンロード中… ${clampPercent(status.percent)}%`
    };
  }
  const pending = pendingVersion(status, v);
  if (pending) {
    const why = status?.state === 'available-manual' && status.reason
      ? `（この環境では自動更新できません：${status.reason}）` : '';
    return {
      label: `v${pending} · ダウンロード`, action: 'manual', tone: 'ready', busy: false,
      title: `クリックしてv${pending}をダウンロードし、現在のアプリを置き換えます${why}`
    };
  }
  switch (status?.state) {
    case 'checking':
      return { label: '確認中…', action: 'none', tone: 'busy', busy: true, title: `アップデートを確認中（現在v${v}）` };
    case 'error':
      return {
        label: '確認に失敗', action: 'check', tone: 'warn', busy: false,
        title: `${status.message} — クリックして再試行`
      };
    case 'not-available':
    case 'just-updated':
      // A check has confirmed it, so say so. Idle (no check yet) stays bare.
      return { label: '最新版', action: 'check', tone: 'idle', busy: false, title: `v${v}は最新版です — クリックして再確認` };
    case 'idle':
    default:
      return { label: null, action: 'check', tone: 'idle', busy: false, title: `v${v} — クリックしてアップデートを確認` };
  }
}

export interface UpdateSettingsView {
  /** Headline: the version that matters right now — yours, or the one waiting. */
  headline: string;
  /** One sentence of explanation. Carries the verbatim error when there is one. */
  detail: string;
  /** Primary button label, or null while the updater is mid-flight and there is
   *  nothing useful to press. */
  button: string | null;
  action: UpdateAction;
  busy: boolean;
  tone: 'idle' | 'busy' | 'ready' | 'warn';
}

/**
 * What the Settings → General "Updates" block shows and does.
 *
 * Separate from `describeUpdate` on purpose. The toolbar chip has room for two
 * words and has to stay quiet when nothing is happening, so its idle state says
 * nothing at all; Settings is where someone goes *to ask*, so every state gets a
 * full sentence and — outside the two mid-flight states — a button. The states
 * and the transitions between them are shared, which is the part that has to
 * stay in sync.
 */
export function describeUpdateSettings(
  status: UpdateStatus | null,
  currentVersion: string
): UpdateSettingsView {
  const v = currentVersion;
  switch (status?.state) {
    case 'checking':
      return {
        headline: `現在のバージョン：v${v}`,
        detail: '新しいリリースを確認中…',
        button: null, action: 'none', busy: true, tone: 'busy'
      };
    case 'available':
      return {
        headline: `v${status.version}を利用できます`,
        detail: `現在はv${v}です。今すぐダウンロードし、準備ができたら再起動してください。`,
        button: `v${status.version}をダウンロード`, action: 'download', busy: false, tone: 'ready'
      };
    case 'downloading':
      return {
        headline: `v${status.version}をダウンロード中`,
        detail: `${clampPercent(status.percent)}%完了。作業を続けられます。再起動は自動では行いません。`,
        button: null, action: 'none', busy: true, tone: 'busy'
      };
    case 'downloaded':
      return {
        headline: `v${status.version}をインストールできます`,
        detail: `Munder Difflinを再起動し、v${v}からの更新を完了します。`,
        button: '再起動して更新', action: 'restart', busy: false, tone: 'ready'
      };
    case 'available-manual':
      return {
        headline: `v${status.version}を利用できます`,
        detail: status.reason
          ? `この環境では自動更新できません（${status.reason}）。リリースページからダウンロードしてください。`
          : 'この環境では自動更新できません。リリースページからダウンロードしてください。',
        button: status.downloadUrl ? `v${status.version}をダウンロード` : 'リリースページを開く',
        action: 'open-release', busy: false, tone: 'warn'
      };
    case 'just-updated':
      return {
        headline: `現在のバージョン：v${v}`,
        detail: '更新が完了しました。最新版です。',
        button: 'アップデートを確認', action: 'check', busy: false, tone: 'idle'
      };
    case 'error':
      return {
        headline: 'アップデートの確認に失敗しました',
        detail: `${status.message}（現在v${v}）。`,
        button: '再試行', action: 'check', busy: false, tone: 'warn'
      };
    case 'not-available':
      return {
        headline: `v${v}は最新版です`,
        detail: '最新の状態です。インストールする更新はありません。',
        button: '再確認', action: 'check', busy: false, tone: 'idle'
      };
    case 'idle':
    default:
      return {
        headline: `現在のバージョン：v${v}`,
        detail: 'アップデートは6時間ごとに自動確認します。必要なら今すぐ確認できます。',
        button: 'アップデートを確認', action: 'check', busy: false, tone: 'idle'
      };
  }
}

/** What to do with the installer once it has downloaded, per platform. Shown
 *  on the title-bar badge's hover card and in the notice after the click. */
export function manualInstallSteps(platform: string): { os: string; steps: string[] } {
  if (platform === 'darwin') {
    return {
      os: 'macOS',
      steps: [
        '.dmgを開き、Munder Difflinを「アプリケーション」へドラッグします。確認されたら「置き換える」を選びます。',
        'このアプリを終了し、「アプリケーション」から新版を開いて同じプロジェクトを選びます。'
      ]
    };
  }
  if (platform === 'win32') {
    return {
      os: 'Windows',
      steps: [
        'このアプリを終了し、ダウンロードしたセットアップ.exeを実行してインストール済み版を置き換えます。',
        'Munder Difflinをもう一度開き、同じプロジェクトを選びます。'
      ]
    };
  }
  return {
    os: 'Linux',
    steps: [
      'ダウンロードした.AppImageへ実行権限を付け（chmod +x）、現在使用中のファイルを置き換えます。',
      'このアプリを終了し、新しいAppImageを起動して同じプロジェクトを選びます。'
    ]
  };
}
