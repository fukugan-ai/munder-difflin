import { useEffect, useRef, useState, type ReactNode } from 'react';
import type { ContextRule, ContextTriggerConfig } from '@shared/triggers';
import { getContextTrigger, setContextTrigger } from './api';
import {
  Callout, Field, Hint, IntervalPicker, Muted, PctField, SubCard, SubHeader, Toggle,
  fmtInterval, textareaStyle
} from './ui';

/**
 * CONTEXT — the trigger that fires on an agent's own terminal filling up rather
 * than on the clock alone. Two rules, and they are not the same operation:
 * compaction SUMMARISES the context, clearing THROWS IT AWAY.
 */

const WRITE_DEBOUNCE_MS = 400;

export function ContextSection({ onSummary }: { onSummary?: (s: string) => void }) {
  const [cfg, setCfg] = useState<ContextTriggerConfig | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    let alive = true;
    getContextTrigger().then((c) => { if (alive) setCfg(c); }).catch(() => { /* defaults */ });
    return () => {
      alive = false;
      if (timer.current) clearTimeout(timer.current);
    };
  }, []);

  useEffect(() => {
    if (!cfg) return;
    const on = [cfg.compact.enabled ? 'compact' : null, cfg.clear.enabled ? 'clear' : null].filter(Boolean);
    onSummary?.(on.length ? on.join(' + ') : 'both off');
  }, [cfg, onSummary]);

  // Optimistic + debounced: the controls answer instantly, and a burst of typing
  // in the message box collapses into one write instead of one per keystroke.
  const commit = (next: ContextTriggerConfig) => {
    setCfg(next);
    if (timer.current) clearTimeout(timer.current);
    timer.current = setTimeout(() => setContextTrigger(next), WRITE_DEBOUNCE_MS);
  };
  const patch = (key: 'compact' | 'clear', fields: Partial<ContextRule>) => {
    if (!cfg) return;
    commit({ ...cfg, [key]: { ...cfg[key], ...fields } });
  };

  if (!cfg) return <Muted>少々お待ちください…</Muted>;

  return (
    <>
      <Muted>
        前回実行から指定時間が経過し、かつエージェントのコンテキスト使用率が指定値以上になった場合だけ実行します。
        0%なら時間だけで実行します。
      </Muted>
      <div style={{ height: 8 }} />

      <RuleCard
        title="コンパクト"
        blurb="コンテキストを要約して会話を続けられるようにします。"
        rule={cfg.compact}
        messageLabel="要約で重視する内容"
        messageHint="プロバイダーのコンパクトコマンドへ追加します。空欄ならコマンドだけを送ります。"
        messagePlaceholder="要約に必ず残す内容…"
        onPatch={(fields) => patch('compact', fields)}
      />

      <RuleCard
        title="クリア"
        blurb="コンテキストを破棄します。要約は作成されません。"
        rule={cfg.clear}
        messageLabel="COMMAND"
        messageHint="そのまま送信します。空欄ならクリアコマンドだけを送ります。"
        messagePlaceholder="/clear"
        caution={
          <>
            クリアはコンテキストを破棄する操作で、コンパクトの簡易版ではありません。
            作業中のエージェントは内容を忘れます。別の方法で保持している場合を除き、オフにしてください。
          </>
        }
        onPatch={(fields) => patch('clear', fields)}
      />
    </>
  );
}

function RuleCard({ title, blurb, rule, messageLabel, messageHint, messagePlaceholder, caution, onPatch }: {
  title: string;
  blurb: string;
  rule: ContextRule;
  messageLabel: string;
  messageHint: string;
  messagePlaceholder: string;
  caution?: ReactNode;
  onPatch: (fields: Partial<ContextRule>) => void;
}) {
  const [open, setOpen] = useState(false);
  return (
    <SubCard>
      <SubHeader
        open={open}
        onToggle={() => setOpen((o) => !o)}
        title={title}
        sub={blurb}
        right={<Toggle on={rule.enabled} onClick={() => onPatch({ enabled: !rule.enabled })} />}
      />
      {/* The caution is always on screen — it is why this ships off — but it only
          goes coral once the destructive rule is actually armed. A red box over a
          switched-off setting is crying wolf. */}
      {caution && <Callout tone={rule.enabled ? 'warn' : 'note'}>{caution}</Callout>}
      {!open && (
        <Hint>
          {rule.enabled
            ? <>{fmtInterval(rule.everyMs)}ごと、コンテキストが{rule.minContextPct}%を超えたら実行。</>
            : <>オフ</>}
        </Hint>
      )}
      {open && (
        <div style={{ marginTop: 4 }}>
          <Field label="最短実行間隔">
            {/* Main clamps a context cadence to 1 minute … 24 hours, so the
                picker offers exactly that range and never labels a value it
                cannot actually store. */}
            <IntervalPicker
              value={rule.everyMs}
              onChange={(everyMs) => onPatch({ everyMs })}
              minMs={60_000}
              maxMs={86_400_000}
            />
          </Field>
          <Field label="コンテキスト使用率">
            <PctField value={rule.minContextPct} onChange={(minContextPct) => onPatch({ minContextPct })} />
            <Hint>実行に必要なコンテキスト使用率です。0%なら時間だけで判定します。</Hint>
          </Field>
          <Field label="大容量ウィンドウの使用率">
            <PctField
              value={rule.minContextPctLargeWindow}
              onChange={(minContextPctLargeWindow) => onPatch({ minContextPctLargeWindow })}
            />
            <Hint>約100万トークンのウィンドウに使います。低い割合でも大量のテキストになります。</Hint>
          </Field>
          <Field label={messageLabel}>
            <textarea
              value={rule.message}
              onChange={(e) => onPatch({ message: e.target.value })}
              rows={3}
              placeholder={messagePlaceholder}
              style={textareaStyle}
            />
            <Hint>{messageHint}</Hint>
          </Field>
        </div>
      )}
    </SubCard>
  );
}
