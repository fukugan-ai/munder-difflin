import { useState } from 'react';
import { SchedulesSection } from './SchedulesSection';
import { ContextSection } from './ContextSection';
import { WebhooksSection } from './WebhooksSection';
import { OrgSection } from './OrgSection';
import { Muted, Scroll, TriggerCard } from './ui';

/**
 * TRIGGERS — every way the floor gets woken up without a human typing, in one
 * tab. Four types (src/shared/triggers.ts is the contract): schedules, context,
 * webhooks and organisation. Schedules is the oldest and used to BE this tab.
 *
 * This panel is a sidebar, so four flat forms would open as a wall. Each type is
 * a collapsed card carrying its name, a one-line "what this is", and a live
 * summary chip; schedules opens expanded because it is the incumbent and the
 * office calendar deep-links here. Inside a card, each row collapses the same
 * way, so nothing is more than two disclosures from legible.
 */
export function TriggersTab() {
  const [schedulesSummary, setSchedulesSummary] = useState('');
  const [contextSummary, setContextSummary] = useState('');
  const [webhooksSummary, setWebhooksSummary] = useState('');
  const [orgSummary, setOrgSummary] = useState('');

  return (
    <Scroll>
      <Muted>あなたが入力しなくても作業を開始できる仕組みを管理します。</Muted>
      <div style={{ height: 8 }} />

      <TriggerCard
        title="スケジュール"
        blurb="一定間隔でプロンプトを実行します。"
        summary={schedulesSummary}
        defaultOpen
      >
        <SchedulesSection onSummary={setSchedulesSummary} />
      </TriggerCard>

      <TriggerCard
        title="コンテキスト"
        blurb="コンテキストが増えたらエージェントをコンパクトまたはクリアします。"
        summary={contextSummary}
      >
        <ContextSection onSummary={setContextSummary} />
      </TriggerCard>

      <TriggerCard
        title="Webhook"
        blurb="外部システムから作業を受け付けます。"
        summary={webhooksSummary}
      >
        <WebhooksSection onSummary={setWebhooksSummary} />
      </TriggerCard>

      <TriggerCard
        title="組織"
        blurb="チームメンバーのMunder Difflinからメッセージを受け取ります。"
        summary={orgSummary}
      >
        <OrgSection onSummary={setOrgSummary} />
      </TriggerCard>
    </Scroll>
  );
}
