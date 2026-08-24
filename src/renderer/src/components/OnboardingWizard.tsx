import { useEffect, useState } from 'react';
import { PixelPanel } from './PixelPanel';
import { PixelButton } from './PixelButton';
import { Icon, type IconName } from './Icon';
import { SpritePortrait } from './SpritePortrait';
import { ProviderLogo } from './ProviderLogo';
import { AGENT_PROVIDER_PRESETS, modelsForProvider, type AgentProvider, type HarnessConfig } from '@/store/config';
import { canReceiveInbox, providerPreset } from '@shared/agentProvider';
import {
  classifyEngineAvailability, engineAvailabilityBadge, engineAvailabilityMessage, engineBlocksOnboarding
} from '@shared/engineAvailability';
import type { ToolStatus } from '@shared/toolCatalog';

export interface OnboardingWizardProps {
  onComplete: (config: HarnessConfig) => void;
}

type Audience = 'technical' | 'non-technical';
type Step = 'persona' | 'welcome' | 'home' | 'orchestrator' | 'repos' | 'permissions' | 'done';

// First-run showcase — the highest-value features a brand-new user should grasp
// before any setup. Each carries a developer-register `desc` and a plain-language
// `descPlain` so the same grid speaks to both audiences (item 1).
interface Feature {
  icon: IconName;
  label: string;
  desc: string;       // technical register
  descPlain: string;  // non-technical register
  tint: string;       // tile background token
  edge: string;       // tile border token
}
const FEATURES: Feature[] = [
  {
    icon: 'mcp',
    label: '11種類のAIを、ひとつのチームに',
    desc: 'Claude Code、Codex、Grok、Kimi、Antigravity、Qwen、OpenCode、Crush、pi、Copilot、Cursorを同じ画面で動かせます。',
    descPlain: 'Claude、Codex、Cursor、Gemini、Grokなど、11種類のAIアシスタントがひとつのチームで連携します。',
    tint: 'var(--cth-lilac-light)', edge: 'var(--cth-lilac)'
  },
  {
    icon: 'gear',
    label: 'MICHAELはあなたの分身',
    desc: 'あなたの分身が依頼を整理してタスクを振り分け、判断が必要なときだけ知らせます。',
    descPlain: 'あなたの分身Michaelが依頼を受け取り、適切なAIへ仕事を任せ、必要なときだけ確認します。',
    tint: 'var(--cth-sky-light)', edge: 'var(--cth-sky)'
  },
  {
    icon: 'web',
    label: '長期記憶',
    desc: '各エージェントのメモは、チームで共有・検索できるMemPalaceに蓄積されます。',
    descPlain: 'AIが過去の作業を覚えているので、毎回ゼロから説明する必要がありません。',
    tint: 'var(--cth-mint-light)', edge: 'var(--cth-mint)'
  },
  {
    icon: 'terminal',
    label: 'コマンドセンター',
    desc: 'ターミナル、チーム、記憶、アクティビティ、タスク、トリガーをひとつの画面で管理します。',
    descPlain: '作業状況、AIの記憶、タスク、トリガーをひとつの画面で確認できます。',
    tint: 'var(--cth-lemon-light)', edge: 'var(--cth-lemon)'
  },
  {
    icon: 'pause',
    label: '安全ガード',
    desc: 'エージェント別のトークン予算、段階的な停止機構、人による承認で安全に制御します。',
    descPlain: '利用上限と安全停止でAIを管理し、大きな操作の前にはあなたへ確認できます。',
    tint: 'var(--cth-coral-light)', edge: 'var(--cth-coral)'
  },
  {
    icon: 'sparkle',
    label: 'すぐ使えるエージェント',
    desc: '設定済みのエージェントをAgent Galleryから選び、ワンクリックで起動できます。',
    descPlain: '設定不要のエージェントをギャラリーからワンクリックで追加できます。',
    tint: 'var(--cth-peach-light)', edge: 'var(--cth-peach)'
  }
];

// One-liner of what each engine is, shown under its row on the orchestrator step
// so a non-technical user knows what they're picking (item 3).
const PROVIDER_BLURB: Partial<Record<AgentProvider, string>> = {
  gemini: 'Gemini CLI - Google Gemini',
  claude: 'Claude Code — Anthropic',
  codex: 'Codex — OpenAI',
  antigravity: 'Antigravity — Google Gemini',
  qwen: 'Qwen — runs a local Qwen model on your machine',
  cursor: 'Cursor Agent CLI — uses your Cursor credits (Luna, Composer, …)'
};

export function OnboardingWizard({ onComplete }: OnboardingWizardProps) {
  const [step, setStep] = useState<Step>('persona');
  // Self-identified audience (item 1). Undefined until chosen on the first screen;
  // the rest of the wizard reads `plain` to swap copy registers.
  const [audience, setAudience] = useState<Audience | undefined>();
  const plain = audience === 'non-technical';

  const [home, setHome] = useState<string>('');
  const [repos, setRepos] = useState<string[]>([]);
  const [autoMode, setAutoMode] = useState<boolean>(true);
  // Anonymous usage stats (TELEMETRY.md). Default ON (opt-out); persisted by
  // finish() so unchecking before finishing means nothing is ever sent.
  const [shareStats, setShareStats] = useState<boolean>(true);
  const [godProvider, setGodProvider] = useState<AgentProvider>('claude');
  const [godModel, setGodModel] = useState<string | undefined>(
    providerPreset('claude').recommendedOrchestratorModel
  );
  const [error, setError] = useState<string | undefined>();
  const [busy, setBusy] = useState(false);

  // Which engine CLIs are actually on this machine. The picker used to record the
  // choice blind; the first check happened when Michael spawned, and for a
  // provider with no installer that meant a first run where nothing ever booted.
  // `undefined` = probe not back yet (or failed): rows show no badge and nothing
  // is blocked, because a broken probe must not lock a new user out.
  const [engines, setEngines] = useState<ToolStatus[] | undefined>();
  const [probing, setProbing] = useState(false);
  const probeEngines = async () => {
    setProbing(true);
    try { setEngines(await window.cth.toolsStatus()); }
    catch { /* leave undefined: unknown, never blocking */ }
    finally { setProbing(false); }
  };
  useEffect(() => { void probeEngines(); }, []);
  const selectedEngine = classifyEngineAvailability(engines, godProvider);
  const engineBlocked = engineBlocksOnboarding(selectedEngine);

  // Permissions & reliability toggles. These apply IMMEDIATELY on change (their
  // own IPC / OS state) — they are NOT part of finish()'s config write. First-run
  // defaults: notifications off (config default), login-item off (fresh install);
  // each reconciles to the real state the IPC returns.
  const [strongKeepalive, setStrongKeepalive] = useState(false);
  const [notifications, setNotifications] = useState(false);
  const [openAtLogin, setOpenAtLogin] = useState(false);

  const toggleStrongKeepalive = async (v: boolean) => {
    setStrongKeepalive(v); // optimistic
    try { setStrongKeepalive((await window.cth.updateConfig({ strongKeepalive: v })).strongKeepalive === true); }
    catch { setStrongKeepalive(!v); }
  };
  const toggleNotifications = async (v: boolean) => {
    setNotifications(v); // optimistic
    try { await window.cth.setNotifications(v); }
    catch { setNotifications(!v); } // revert on failure
  };
  const toggleOpenAtLogin = async (v: boolean) => {
    setOpenAtLogin(v); // optimistic
    try { setOpenAtLogin(await window.cth.setLoginItem(v)); } // reconcile to OS truth
    catch { setOpenAtLogin(!v); }
  };
  const openSettings = (url: string) => { void window.cth.openExternal(url); };

  // Default-suggest a sensible harness home on first render.
  //
  // This used to read `window.process.env.HOME`, which is ALWAYS undefined here:
  // the window runs with `contextIsolation: true` / `nodeIntegration: false` and
  // the preload bridges exactly one object (`cth`), so the renderer's main world
  // has no `process`. The suggestion therefore always collapsed to '' and the
  // field rendered empty — leaving the copy above promising a default the user
  // could not accept, and Finish failing with "Pick a harness home folder first."
  //
  // Suggest the literal `~/HarnessAgents` instead. That is exactly the string
  // #140's normalizeHiveHome()/expandTilde() were built to absorb: it is expanded
  // at the config-write boundary AND at ensureHarnessHome's mkdir, so every
  // downstream reader still sees one absolute path. No new IPC surface.
  useEffect(() => {
    if (!home) setHome('~/HarnessAgents');
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const pickHome = async () => {
    setError(undefined);
    const res = await window.cth.chooseFolder();
    if (res.ok) setHome(res.path);
    else if (res.error !== 'cancelled') setError(res.error);
  };

  const pickRepo = async () => {
    setError(undefined);
    const res = await window.cth.chooseFolder();
    if (res.ok && !repos.includes(res.path)) setRepos([...repos, res.path]);
    else if (!res.ok && res.error !== 'cancelled') setError(res.error);
  };

  const removeRepo = (path: string) => setRepos(repos.filter(r => r !== path));

  const finish = async () => {
    setBusy(true);
    setError(undefined);
    const harnessHome = home.trim(); // whitespace-only is not a folder
    if (!harnessHome) { setError('先に作業フォルダーを選んでください。'); setBusy(false); setStep('home'); return; }
    // The orchestrator step already refuses to advance on this, but a late probe
    // result can change the answer after the user has moved on. Never write a
    // godProvider that is known to be unable to boot.
    if (engineBlocked) {
      setError(`${providerPreset(godProvider).label} はインストールされていません。導入後に「もう一度確認」を押すか、別のエンジンを選んでください。`);
      setBusy(false); setStep('orchestrator'); return;
    }
    const ensure = await window.cth.ensureHarnessHome(harnessHome);
    if (!ensure.ok) {
      setError(ensure.error ?? '作業フォルダーを作成できませんでした');
      setBusy(false);
      return;
    }
    const next = await window.cth.updateConfig({
      onboardingComplete: true,
      audience: audience ?? 'technical',
      harnessHome, // the same trimmed value we just mkdir'd, not the raw field
      registeredRepos: repos,
      autoMode,
      godProvider,
      godModel,
      telemetryEnabled: shareStats
    });
    setBusy(false);
    onComplete(next);
  };

  return (
    <div style={{
      position: 'fixed', inset: 0,
      background: 'var(--cth-cream-200)',
      backgroundImage:
        `repeating-linear-gradient(45deg, rgba(232, 217, 160, 0.4) 0 1px, transparent 1px 8px)`,
      // Scroll the overlay rather than clip the wizard. Step 2 lists every
      // installed CLI engine (8 rows + a model select), which is taller than a
      // 1080p-class window once the OS chrome is subtracted — the panel was
      // being cut off at BOTH edges with no way to reach the buttons.
      display: 'flex',
      overflowY: 'auto',
      zIndex: 200,
      padding: 32
    }}>
      {/* `margin: auto` centers, NOT `align-items: center`. A centered flex item
          that overflows its container is clipped at the TOP and unreachable by
          scrolling (the overflow spills past the scroll origin); auto margins
          center while it fits and collapse to a normal scroll once it doesn't. */}
      <div style={{ width: 640, maxWidth: '94vw', margin: 'auto' }}>
        <PixelPanel
          variant="dialog"
          title={
            step === 'persona' ? 'MUNDER DIFFLINへようこそ'
            : step === 'welcome' ? 'AIチームをご紹介します'
            : step === 'home' ? 'ステップ1/4 · 作業フォルダー'
            : step === 'orchestrator' ? 'ステップ2/4 · 分身に使うAI'
            : step === 'repos' ? (plain ? 'ステップ3/4 · プロジェクト' : 'ステップ3/4 · リポジトリ')
            : step === 'permissions' ? 'ステップ4/4 · 権限と動作設定'
            : '準備完了'
          }
          noPadding
        >
          <div style={{ padding: 20, display: 'flex', flexDirection: 'column', gap: 16, maxHeight: '86vh', overflowY: 'auto' }}>

            {step === 'persona' && (
              <>
                <div style={{ display: 'flex', gap: 12, alignItems: 'flex-start' }}>
                  <div style={{
                    width: 56, height: 56, flexShrink: 0,
                    background: 'var(--cth-sky-light)',
                    boxShadow: 'inset 0 0 0 1.5px var(--cth-ink-500)',
                    display: 'flex', alignItems: 'flex-end', justifyContent: 'center', overflow: 'hidden'
                  }}>
                    <SpritePortrait character="michael" scale={2} />
                  </div>
                  <div>
                    <div style={{ fontFamily: 'var(--cth-font-display)', fontSize: 12, lineHeight: '18px' }}>
                      あなたの分身が24時間働きます
                    </div>
                    <div style={{ fontSize: 12, color: 'var(--cth-ink-700)', lineHeight: '19px' }}>
                      Munder Difflinは、普段使っているCLIエージェントをあなたの分身にします。
                      分身は複数のAIエージェントをまとめ、あなたが離れている間も作業を続けます。
                      コンテキスト、記憶、タスク、トリガー、環境、ファイル、外部連携を一括管理します。
                      <span style={{ color: 'var(--cth-ink-500)' }}> すべてこのPC上で動作します。</span>
                    </div>
                  </div>
                </div>

                <div style={{ fontFamily: 'var(--cth-font-display)', fontSize: 10, color: 'var(--cth-ink-700)' }}>
                  はじめに、使い方に近い方を選んでください
                </div>
                <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 10 }}>
                  <PersonaCard
                    icon="code"
                    title="技術者向け"
                    desc="コードやターミナルを使います。CLIコマンド、フラグ、モデルIDも表示してください。"
                    selected={audience === 'technical'}
                    onClick={() => { setAudience('technical'); setError(undefined); }}
                  />
                  <PersonaCard
                    icon="sparkle"
                    title="一般利用者向け"
                    desc="マーケティング、営業、業務などで使います。専門用語を避けて分かりやすく説明してください。"
                    selected={audience === 'non-technical'}
                    onClick={() => { setAudience('non-technical'); setError(undefined); }}
                  />
                </div>
              </>
            )}

            {step === 'welcome' && (
              <>
                <div style={{ display: 'flex', gap: 12, alignItems: 'center' }}>
                  <div style={{
                    width: 56, height: 56, flexShrink: 0,
                    background: 'var(--cth-sky-light)',
                    boxShadow: 'inset 0 0 0 1.5px var(--cth-ink-500)',
                    display: 'flex', alignItems: 'flex-end', justifyContent: 'center', overflow: 'hidden'
                  }}>
                    <SpritePortrait character="michael" scale={2} />
                  </div>
                  <div>
                    <div style={{
                      fontFamily: 'var(--cth-font-display)',
                      fontSize: 12, lineHeight: '18px'
                    }}>あなたの分身とAIチーム</div>
                    <div style={{ fontSize: 12, color: 'var(--cth-ink-700)', lineHeight: '18px' }}>
                      {plain
                        ? 'あなたの分身がAIチームを動かし、その様子をひとつの画面で確認できます。主な機能はこちらです。'
                        : 'あなたの分身がローカルで常時動くAIコーディングエージェントを統括します。主な機能はこちらです。'}
                    </div>
                  </div>
                </div>

                <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 8 }}>
                  {FEATURES.map((f) => (
                    <div key={f.label} style={{
                      display: 'flex', gap: 10, alignItems: 'flex-start',
                      padding: 10,
                      background: f.tint,
                      boxShadow: `inset 0 0 0 2px ${f.edge}`
                    }}>
                      <div style={{
                        width: 28, height: 28, flexShrink: 0,
                        display: 'flex', alignItems: 'center', justifyContent: 'center',
                        background: 'var(--cth-paper-100)',
                        boxShadow: 'inset 0 0 0 1px var(--cth-ink-300)'
                      }}>
                        <Icon name={f.icon} />
                      </div>
                      <div style={{ minWidth: 0 }}>
                        <div style={{
                          fontFamily: 'var(--cth-font-display)',
                          fontSize: 10, lineHeight: '14px', marginBottom: 3
                        }}>{f.label}</div>
                        <div style={{ fontSize: 12, lineHeight: '16px', color: 'var(--cth-ink-700)' }}>
                          {plain ? f.descPlain : f.desc}
                        </div>
                      </div>
                    </div>
                  ))}
                </div>
              </>
            )}

            {step === 'home' && (
              <>
                {plain ? (
                  <p style={{ margin: 0, lineHeight: '22px' }}>
                    アプリ専用の空フォルダーを用意してください。アプリの設定やエージェントの記憶は
                    すべてここに保存されます。たとえば{' '}
                    <code style={{ fontFamily: 'var(--cth-font-mono)', background: 'var(--cth-paper-100)', padding: '0 4px' }}>
                      ~/HarnessAgents
                    </code>{' '}
                    がおすすめです。存在しない場合は自動で作成します。
                  </p>
                ) : (
                  <p style={{ margin: 0, lineHeight: '22px' }}>
                    エージェントのメタデータ、ログ、ここで作る新しいリポジトリなど、基盤のファイルを
                    保存するフォルダーを選んでください。たとえば{' '}
                    <code style={{ fontFamily: 'var(--cth-font-mono)', background: 'var(--cth-paper-100)', padding: '0 4px' }}>
                      ~/HarnessAgents
                    </code>{' '}
                    がおすすめです。存在しない場合は自動で作成します。
                  </p>
                )}
                <div style={{ display: 'flex', gap: 8 }}>
                  <input
                    value={home}
                    onChange={(e) => setHome(e.target.value)}
                    placeholder="/path/to/HarnessAgents"
                    style={inputStyle}
                  />
                  <PixelButton variant="secondary" size="md" onClick={pickHome}>
                    <span style={{ display: 'inline-flex', gap: 4, alignItems: 'center' }}>
                      <Icon name="folder" /> 選択
                    </span>
                  </PixelButton>
                </div>
                <div style={{ fontSize: 12, color: 'var(--cth-ink-500)' }}>
                  {plain
                    ? '普段このフォルダーを開く必要はありません。再起動しても情報を失わないよう、アプリが記録を保存します。'
                    : 'チームの本部にあたるフォルダーです。エージェントの状態を保存し、再起動後もセッションを再開できます。'}
                </div>
              </>
            )}

            {step === 'orchestrator' && (
              <>
                <p style={{ margin: 0, lineHeight: '22px' }}>
                  {plain ? (
                    <><strong>Michaelはあなたの分身です。</strong>依頼を読み、タスクに分けて適切なAIへ任せます。
                    チームをまとめるMichaelに使うAIエンジンを選んでください。</>
                  ) : (
                    <><strong>Michaelはあなたの分身です。</strong>依頼を整理し、タスクを割り当て、チームを管理し、
                    あなたの判断が必要な事項だけを知らせます。Michaelに使うエンジンとモデルを選んでください。
                    長いコンテキストを扱える高性能なモデルがおすすめです。</>
                  )}
                </p>

                {/* What is a CLI agent / your clone — item 3 */}
                <div style={{
                  display: 'flex', gap: 8, alignItems: 'flex-start', padding: 10,
                  background: 'var(--cth-lemon-light)', boxShadow: 'inset 0 0 0 1px var(--cth-ink-300)',
                  fontSize: 12, lineHeight: '17px', color: 'var(--cth-ink-700)'
                }}>
                  <span style={{ flexShrink: 0, marginTop: 1 }}><Icon name="sparkle" /></span>
                  <span>
                    {plain ? (
                      <><strong>CLIエージェント</strong>とは、PC上で動くAIコーディングアシスタントです。
                      Claude Code（Anthropic）、Codex（OpenAI）、Antigravity（Google Gemini）などがあります。
                      <strong>あなたの分身</strong>は、AIチーム全体を常時まとめるエージェントです。
                      Claude CodeのOpus 4.8（1M）をおすすめします。ほかのAIは後から追加・変更できます。</>
                    ) : (
                      <>各項目は<strong>CLIエンジン</strong>です。インストール済みのものはすぐ使え、
                      「初回起動時に導入」と表示されたものはMichaelの初回起動時に設定されます。
                      <strong>あなたの分身</strong>（Michael）がチーム全体を統括します。
                      推奨はClaude Code・Opus 4.8・1Mです。ほかのプロバイダーは後からエージェントごとに設定できます。</>
                    )}
                  </span>
                </div>

                <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
                  {AGENT_PROVIDER_PRESETS.filter((p) => canReceiveInbox(p.id)).map((p) => {
                    const sel = godProvider === p.id;
                    return (
                      <label key={p.id} style={{
                        display: 'flex', alignItems: 'center', gap: 10,
                        padding: '8px 10px',
                        background: sel ? 'var(--cth-mint-light)' : 'var(--cth-paper-100)',
                        boxShadow: `inset 0 0 0 ${sel ? 2 : 1}px ${sel ? 'var(--cth-mint)' : 'var(--cth-ink-300)'}`,
                        cursor: 'pointer'
                      }}>
                        <input
                          type="radio"
                          name="godProvider"
                          value={p.id}
                          checked={sel}
                          onChange={() => {
                            setGodProvider(p.id);
                            // Reset the model to the new provider's recommended pick so the
                            // dropdown below always shows a valid model for the chosen engine.
                            setGodModel(p.recommendedOrchestratorModel);
                          }}
                          style={{ width: 16, height: 16, flexShrink: 0 }}
                        />
                        <span style={{
                          width: 22, height: 22, flexShrink: 0, display: 'flex',
                          alignItems: 'center', justifyContent: 'center', color: 'var(--cth-ink-900)'
                        }}>
                          <ProviderLogo provider={p.id} size={18} />
                        </span>
                        <span style={{ flex: 1, minWidth: 0 }}>
                          <span style={{ display: 'block', fontFamily: 'var(--cth-font-display)', fontSize: 11 }}>
                            {p.label.toUpperCase()}
                          </span>
                          {PROVIDER_BLURB[p.id] && (
                            <span style={{ display: 'block', fontSize: 11, color: 'var(--cth-ink-500)' }}>
                              {PROVIDER_BLURB[p.id]}
                            </span>
                          )}
                        </span>
                        {(() => {
                          const a = classifyEngineAvailability(engines, p.id);
                          const badge = engineAvailabilityBadge(a);
                          if (!badge) return null;
                          const bad = a.state === 'not-installable';
                          return (
                            <span title={a.path ?? undefined} style={{
                              fontSize: 10, padding: '1px 5px', lineHeight: '16px',
                              background: a.state === 'installed' ? 'var(--cth-mint-light)' : bad ? 'var(--cth-paper-100)' : 'var(--cth-cream-200)',
                              color: bad ? 'var(--cth-ink-500)' : 'var(--cth-ink-900)',
                              boxShadow: 'inset 0 0 0 1px var(--cth-ink-300)',
                              fontFamily: 'var(--cth-font-display)', flexShrink: 0
                            }}>{badge}</span>
                          );
                        })()}
                        {p.id === 'claude' && (
                          <span style={{
                            fontSize: 10, padding: '1px 5px', lineHeight: '16px',
                            background: 'var(--cth-lemon)',
                            boxShadow: 'inset 0 0 0 1px var(--cth-ink-300)',
                            fontFamily: 'var(--cth-font-display)', flexShrink: 0
                          }}>おすすめ</span>
                        )}
                      </label>
                    );
                  })}
                </div>
                {engineBlocked && (
                  <div style={{
                    display: 'flex', flexDirection: 'column', gap: 8, padding: 10,
                    background: 'var(--cth-paper-100)', boxShadow: 'inset 0 0 0 2px var(--cth-ink-900)',
                    fontSize: 12, lineHeight: '17px', color: 'var(--cth-ink-900)'
                  }}>
                    <span>{engineAvailabilityMessage(selectedEngine, providerPreset(godProvider).label)}</span>
                    <div style={{ display: 'flex', gap: 8, alignItems: 'center', flexWrap: 'wrap' }}>
                      <PixelButton variant="secondary" size="sm" onClick={() => { void probeEngines(); }} disabled={probing}>
                        {probing ? '確認中…' : 'もう一度確認'}
                      </PixelButton>
                      {selectedEngine.docsUrl && (
                        <PixelButton variant="ghost" size="sm" onClick={() => { void window.cth.openExternal(selectedEngine.docsUrl!); }}>
                          インストール手順
                        </PixelButton>
                      )}
                    </div>
                  </div>
                )}
                <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
                  <div style={{ fontSize: 12, color: 'var(--cth-ink-500)' }}>モデル</div>
                  <select
                    value={godModel ?? ''}
                    onChange={(e) => setGodModel(e.target.value || undefined)}
                    style={inputStyle}
                  >
                    {modelsForProvider(godProvider).map((m) => (
                      <option key={m.label} value={m.id ?? ''}>{m.label}</option>
                    ))}
                  </select>
                  <div style={{ fontSize: 12, color: 'var(--cth-ink-500)' }}>
                    ここで設定するのはMichaelだけです。ほかのエージェントには別のプロバイダーを設定できます。
                  </div>
                </div>
              </>
            )}

            {step === 'repos' && (
              <>
                <p style={{ margin: 0, lineHeight: '22px' }}>
                  {plain ? (
                    <><strong>プロジェクト</strong>を追加してください。プロジェクトは、AIに作業してほしいコード、
                    文書、メモなどを置くフォルダーです。新規作成も既存フォルダーの選択もでき、後から追加できます。</>
                  ) : (
                    <>エージェントに作業させるリポジトリを追加してください。各フォルダーがひとつの
                    <strong>プロジェクト</strong>になり、複数のエージェントで共有できます。後から追加もできます。</>
                  )}
                </p>
                <div style={{
                  display: 'flex', flexDirection: 'column', gap: 6,
                  maxHeight: 200, overflowY: 'auto'
                }}>
                  {repos.length === 0 && (
                    <div style={{
                      padding: 12,
                      fontSize: 13,
                      color: 'var(--cth-ink-500)',
                      background: 'var(--cth-paper-200)',
                      textAlign: 'center'
                    }}>
                      {plain
                        ? 'プロジェクトはまだありません。任意なので、後から追加できます。'
                        : 'リポジトリはまだありません。任意ですが、追加をおすすめします。'}
                    </div>
                  )}
                  {repos.map((r) => (
                    <div key={r} style={{
                      display: 'flex', alignItems: 'center', gap: 8,
                      padding: '6px 10px',
                      background: 'var(--cth-paper-100)',
                      boxShadow: 'inset 0 0 0 1px var(--cth-ink-100)'
                    }}>
                      <Icon name="folder" />
                      <span style={{
                        flex: 1,
                        fontFamily: 'var(--cth-font-mono)', fontSize: 13,
                        whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis'
                      }}>{r}</span>
                      <PixelButton variant="ghost" size="sm" onClick={() => removeRepo(r)}>
                        <Icon name="x" />
                      </PixelButton>
                    </div>
                  ))}
                </div>
                <PixelButton variant="secondary" size="md" onClick={pickRepo}>
                  <span style={{ display: 'inline-flex', gap: 6, alignItems: 'center' }}>
                    <Icon name="plus" /> {plain ? 'プロジェクトを追加' : 'リポジトリを追加'}
                  </span>
                </PixelButton>
              </>
            )}

            {step === 'permissions' && (
              <>
                {/* AUTONOMY — merged from the old "auto mode" step (item 5). One choice
                    that maps to each engine's flag (item 6): autoMode → claude
                    bypassPermissions / codex --dangerously-bypass-approvals-and-sandbox,
                    etc.; off → each engine's ask-first default. */}
                <div style={{ fontFamily: 'var(--cth-font-display)', fontSize: 10, color: 'var(--cth-ink-700)' }}>
                  エージェントにどこまで任せますか？
                </div>
                <label style={{
                  display: 'flex', alignItems: 'center', gap: 10,
                  padding: 12,
                  background: autoMode ? 'var(--cth-mint-light)' : 'var(--cth-cream-200)',
                  boxShadow: `inset 0 0 0 2px ${autoMode ? 'var(--cth-mint)' : 'var(--cth-ink-500)'}`,
                  cursor: 'pointer'
                }}>
                  <input
                    type="checkbox"
                    checked={autoMode}
                    onChange={(e) => setAutoMode(e.target.checked)}
                    style={{ width: 18, height: 18, flexShrink: 0 }}
                  />
                  <div>
                    <div style={{ fontFamily: 'var(--cth-font-display)', fontSize: 10, lineHeight: '14px' }}>
                      {plain ? 'エージェントに自動で作業を任せる' : '自律実行（自動モード）'}
                    </div>
                    <div style={{ fontSize: 13, color: 'var(--cth-ink-700)' }}>
                      {plain
                        ? (autoMode
                            ? 'オン：確認で止まらずにタスクを進めます。'
                            : 'オフ：ファイル変更やコマンド実行の前に確認します。')
                        : (autoMode
                            ? 'オン：ClaudeはbypassPermissions、Codexは承認とsandboxを省略して停止せずに実行します。'
                            : 'オフ：各エージェントが編集やシェルコマンドの前に確認します。')}
                    </div>
                  </div>
                </label>
                <div style={{ fontSize: 12, color: 'var(--cth-ink-500)' }}>
                  {plain
                    ? 'エージェント専用のプロジェクトで使う場合におすすめです。後からエージェントごとに変更できます。'
                    : '管理画面から自律実行する場合に適しますが、本番リポジトリでは注意が必要です。エージェント追加画面で個別に変更できます。'}
                </div>

                <div style={{ height: 1, background: 'var(--cth-ink-300)', margin: '2px 0' }} />

                {/* RELIABILITY — keeping work firing while you're away. */}
                <div style={{ fontFamily: 'var(--cth-font-display)', fontSize: 10, color: 'var(--cth-ink-700)' }}>
                  離席中も作業を続けるための設定
                </div>
                <p style={{ margin: 0, lineHeight: '20px', fontSize: 12, color: 'var(--cth-ink-700)' }}>
                  {plain
                    ? 'エージェントは離席中もスケジュールやターミナルで作業を続けます。次の設定で動作を維持し、必要な通知を受け取れます。'
                    : 'エージェントはスケジュールとターミナルで作業を続けます。Macがスリープすると一時停止し、復帰後に再開します。データは失われません。'}
                </p>

                <ToggleRow
                  icon="clock"
                  label="離席中も動作を続ける"
                  desc="エージェントの稼働中はMacのスリープを防ぎ、スケジュールとターミナルを予定どおり動かします。バッテリー消費が増えるため、電源接続時におすすめです。初期設定はオフです。"
                  on={strongKeepalive}
                  tint="var(--cth-mint-light)"
                  edge="var(--cth-mint)"
                  onChange={toggleStrongKeepalive}
                />

                <ToggleRow
                  icon="bell"
                  label="デスクトップ通知"
                  desc="エージェントが確認を求めたときや、ターミナルの再起動が必要なときに通知します。初回はmacOSが許可を求めます。"
                  on={notifications}
                  tint="var(--cth-peach-light)"
                  edge="var(--cth-peach)"
                  onChange={toggleNotifications}
                />

                <ToggleRow
                  icon="play"
                  label="ログイン時に起動"
                  desc="再起動後にアプリを自動で開き、予定されたミッションを再開します。この設定はすぐに反映されます。"
                  on={openAtLogin}
                  tint="var(--cth-sky-light)"
                  edge="var(--cth-sky)"
                  onChange={toggleOpenAtLogin}
                />

                <ToggleRow
                  icon="info"
                  label="匿名の利用統計を共有"
                  desc="アプリ起動、エージェント追加、機能利用などの匿名イベントを改善に役立てます。プロンプト、コード、ファイルパス、AIの出力は送信しません。詳細はTELEMETRY.mdで確認でき、設定からいつでも変更できます。"
                  on={shareStats}
                  tint="var(--cth-lemon-light)"
                  edge="var(--cth-lemon)"
                  onChange={() => setShareStats(!shareStats)}
                />

                {/* LEVER 4 — instruction-only: macOS won't let the app flip Energy, so we deep-link the pane. */}
                <div style={{
                  display: 'flex', gap: 10, alignItems: 'flex-start', padding: 10,
                  background: 'var(--cth-lemon-light)',
                  boxShadow: 'inset 0 0 0 1px var(--cth-ink-300)'
                }}>
                  <span style={{
                    width: 28, height: 28, flexShrink: 0, display: 'flex',
                    alignItems: 'center', justifyContent: 'center',
                    background: 'var(--cth-paper-100)', boxShadow: 'inset 0 0 0 1px var(--cth-ink-300)'
                  }}>
                    <Icon name="gear" />
                  </span>
                  <div style={{ minWidth: 0, display: 'flex', flexDirection: 'column', gap: 8 }}>
                    <div>
                      <div style={{ fontFamily: 'var(--cth-font-display)', fontSize: 10, lineHeight: '14px', marginBottom: 3 }}>
                        電源接続時のスリープを防ぐ（手動設定）
                      </div>
                      <div style={{ fontSize: 12, lineHeight: '16px', color: 'var(--cth-ink-700)' }}>
                        この項目はmacOS側で設定します。「バッテリー」→「オプション」で、電源アダプタ使用時の
                        「ディスプレイがオフのときに自動でスリープさせない」をオンにすると、画面消灯中もタイマーが動きます。
                        未設定でMacがスリープしても作業内容は残り、復帰後に再開されます。
                      </div>
                    </div>
                    <PixelButton variant="secondary" size="sm"
                      onClick={() => openSettings('x-apple.systempreferences:com.apple.preference.battery')}>
                      <span style={{ display: 'inline-flex', gap: 6, alignItems: 'center' }}>
                        <Icon name="arrow-right" /> バッテリー設定を開く
                      </span>
                    </PixelButton>
                  </div>
                </div>
              </>
            )}

            {error && (
              <div style={{
                padding: '6px 10px',
                background: 'var(--cth-coral-light)',
                boxShadow: 'inset 0 0 0 1px var(--cth-coral)',
                fontSize: 13,
                color: 'var(--cth-ink-900)',
                overflowWrap: 'anywhere'
              }}>{error}</div>
            )}

            {/* Footer / nav */}
            <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginTop: 4 }}>
              <Dots step={step} />
              <div style={{ display: 'flex', gap: 8 }}>
                {step !== 'persona' && step !== 'welcome' && (
                  <PixelButton variant="ghost" size="md" onClick={() => setStep(prevStep(step))} disabled={busy}>
                    戻る
                  </PixelButton>
                )}
                {step === 'welcome' && (
                  <PixelButton variant="ghost" size="md" onClick={() => setStep('persona')} disabled={busy}>
                    戻る
                  </PixelButton>
                )}
                {step !== 'permissions' && (
                  <PixelButton
                    variant="primary"
                    size="md"
                    onClick={() => {
                      // Validate the home step HERE. Without this the only check
                      // lives in finish(), so an empty field walks you through all
                      // four steps and then bounces you back to step 1 to be told.
                      if (step === 'home' && !home.trim()) {
                        setError('先に作業フォルダーを選んでください。');
                        return;
                      }
                      // Same idea for the engine: refuse here, with the reason on
                      // screen, instead of letting a pick that cannot boot through
                      // to a Michael that never starts.
                      if (step === 'orchestrator' && engineBlocked) {
                        setError(`${providerPreset(godProvider).label} はインストールされていません。導入後に「もう一度確認」を押すか、別のエンジンを選んでください。`);
                        return;
                      }
                      setError(undefined);
                      setStep(nextStep(step));
                    }}
                    disabled={(step === 'persona' && !audience) || (step === 'orchestrator' && engineBlocked)}
                  >
                    {step === 'welcome' ? '設定を始める' : '次へ'}
                  </PixelButton>
                )}
                {step === 'permissions' && (
                  <PixelButton variant="primary" size="md" onClick={finish} disabled={busy}>
                    {busy ? '保存中…' : '完了'}
                  </PixelButton>
                )}
              </div>
            </div>
          </div>
        </PixelPanel>
      </div>
    </div>
  );
}

function PersonaCard({ icon, title, desc, selected, onClick }: {
  icon: IconName;
  title: string;
  desc: string;
  selected: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      style={{
        textAlign: 'left', cursor: 'pointer', border: 'none',
        padding: 12, display: 'flex', flexDirection: 'column', gap: 6,
        background: selected ? 'var(--cth-mint-light)' : 'var(--cth-paper-100)',
        boxShadow: `inset 0 0 0 ${selected ? 2 : 1}px ${selected ? 'var(--cth-mint)' : 'var(--cth-ink-300)'}`
      }}
    >
      <span style={{
        width: 28, height: 28, display: 'flex', alignItems: 'center', justifyContent: 'center',
        background: 'var(--cth-paper-100)', boxShadow: 'inset 0 0 0 1px var(--cth-ink-300)'
      }}>
        <Icon name={icon} />
      </span>
      <span style={{ fontFamily: 'var(--cth-font-display)', fontSize: 11, lineHeight: '15px', color: 'var(--cth-ink-900)' }}>
        {title}
      </span>
      <span style={{ fontSize: 12, lineHeight: '16px', color: 'var(--cth-ink-700)' }}>
        {desc}
      </span>
    </button>
  );
}

function ToggleRow({ icon, label, desc, on, tint, edge, onChange }: {
  icon: IconName;
  label: string;
  desc: string;
  on: boolean;
  tint: string; // background token when on
  edge: string; // border token when on
  onChange: (v: boolean) => void;
}) {
  return (
    <label style={{
      display: 'flex', gap: 10, alignItems: 'flex-start', padding: 10,
      background: on ? tint : 'var(--cth-paper-100)',
      boxShadow: `inset 0 0 0 ${on ? 2 : 1}px ${on ? edge : 'var(--cth-ink-300)'}`,
      cursor: 'pointer'
    }}>
      <input
        type="checkbox"
        checked={on}
        onChange={(e) => onChange(e.target.checked)}
        style={{ width: 18, height: 18, flexShrink: 0, marginTop: 5 }}
      />
      <span style={{
        width: 28, height: 28, flexShrink: 0, display: 'flex',
        alignItems: 'center', justifyContent: 'center',
        background: 'var(--cth-paper-100)', boxShadow: 'inset 0 0 0 1px var(--cth-ink-300)'
      }}>
        <Icon name={icon} />
      </span>
      <span style={{ minWidth: 0 }}>
        <span style={{ display: 'block', fontFamily: 'var(--cth-font-display)', fontSize: 10, lineHeight: '14px', marginBottom: 3 }}>
          {label}
        </span>
        <span style={{ display: 'block', fontSize: 12, lineHeight: '16px', color: 'var(--cth-ink-700)' }}>
          {desc}
        </span>
      </span>
    </label>
  );
}

function Dots({ step }: { step: Step }) {
  const order: Step[] = ['persona', 'welcome', 'home', 'orchestrator', 'repos', 'permissions'];
  return (
    <div style={{ display: 'flex', gap: 4 }}>
      {order.map((s) => (
        <span key={s} style={{
          width: 8, height: 8,
          background: s === step ? 'var(--cth-ink-900)' : 'var(--cth-cream-300)',
          boxShadow: 'inset 0 0 0 1px var(--cth-ink-300)'
        }} />
      ))}
    </div>
  );
}

function nextStep(s: Step): Step {
  return s === 'persona' ? 'welcome'
    : s === 'welcome' ? 'home'
    : s === 'home' ? 'orchestrator'
    : s === 'orchestrator' ? 'repos'
    : s === 'repos' ? 'permissions'
    : 'done';
}
function prevStep(s: Step): Step {
  return s === 'permissions' ? 'repos'
    : s === 'repos' ? 'orchestrator'
    : s === 'orchestrator' ? 'home'
    : s === 'home' ? 'welcome'
    : s === 'welcome' ? 'persona'
    : 'persona';
}

const inputStyle: React.CSSProperties = {
  flex: 1,
  padding: '6px 8px 4px',
  background: 'var(--cth-paper-100)',
  border: 'none',
  boxShadow: 'inset 0 0 0 1px var(--cth-ink-100)',
  fontFamily: 'var(--cth-font-mono)',
  fontSize: 13,
  color: 'var(--cth-ink-900)',
  outline: 'none'
};
