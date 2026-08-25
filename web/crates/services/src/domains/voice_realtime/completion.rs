#![forbid(unsafe_code)]

use md_web_contracts::domains::voice_realtime::{
    CompletionKind, CompletionVia, RealtimeCompletion,
};

const MAX_PENDING: usize = 200;
const MAX_QUEUED: usize = 50;
const PENDING_TTL_MS: i64 = 24 * 60 * 60 * 1_000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingDispatch {
    pub correlation_id: String,
    pub kind: CompletionKind,
    pub target_agent_id: String,
    pub task_id: Option<String>,
    pub objective: Option<String>,
    pub dispatched_at_ms: i64,
    pub dispatch_message_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletionTask {
    pub id: String,
    pub status: String,
    pub title: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletionInboxMessage {
    pub id: String,
    pub from: String,
    pub in_reply_to: Option<String>,
    pub body: String,
    pub created_at_ms: i64,
}

#[derive(Clone, Debug, Default)]
pub struct CompletionDetector {
    pending: Vec<PendingDispatch>,
    queued: Vec<RealtimeCompletion>,
    session_live: bool,
}

impl CompletionDetector {
    pub fn track(&mut self, dispatch: PendingDispatch, now_ms: i64) {
        self.prune(now_ms);
        if let Some(existing) = self
            .pending
            .iter_mut()
            .find(|existing| existing.correlation_id == dispatch.correlation_id)
        {
            *existing = dispatch;
            return;
        }
        self.pending.push(dispatch);
        if self.pending.len() > MAX_PENDING {
            self.pending
                .sort_unstable_by_key(|item| item.dispatched_at_ms);
            let overflow = self.pending.len() - MAX_PENDING;
            self.pending.drain(..overflow);
        }
    }

    pub fn untrack(&mut self, correlation_id: &str) {
        self.pending
            .retain(|pending| pending.correlation_id != correlation_id);
    }

    pub fn set_session_live(&mut self, live: bool) {
        self.session_live = live;
    }

    pub fn poll(
        &mut self,
        tasks: &[CompletionTask],
        inbox: &[CompletionInboxMessage],
        now_ms: i64,
    ) -> Vec<RealtimeCompletion> {
        self.prune(now_ms);
        let mut completed = Vec::new();
        self.pending.retain(|pending| {
            let event = detect_one(pending, tasks, inbox, now_ms);
            if let Some(event) = event {
                completed.push(event);
                false
            } else {
                true
            }
        });
        if self.session_live {
            return completed;
        }
        self.queued.extend(completed);
        if self.queued.len() > MAX_QUEUED {
            let overflow = self.queued.len() - MAX_QUEUED;
            self.queued.drain(..overflow);
        }
        Vec::new()
    }

    pub fn drain_queued(&mut self) -> Vec<RealtimeCompletion> {
        std::mem::take(&mut self.queued)
    }

    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    fn prune(&mut self, now_ms: i64) {
        self.pending.retain(|pending| {
            pending.dispatched_at_ms <= 0
                || now_ms.saturating_sub(pending.dispatched_at_ms) <= PENDING_TTL_MS
        });
    }
}

fn detect_one(
    pending: &PendingDispatch,
    tasks: &[CompletionTask],
    inbox: &[CompletionInboxMessage],
    now_ms: i64,
) -> Option<RealtimeCompletion> {
    if let Some(task_id) = pending.task_id.as_deref()
        && let Some(task) = tasks
            .iter()
            .find(|task| task.id == task_id && task.status == "done")
    {
        return Some(completion(
            pending,
            CompletionVia::CardDone,
            format!("{} が完了しました。", task.title),
            now_ms,
            None,
        ));
    }

    let reply = inbox.iter().find(|message| {
        message.from == pending.target_agent_id
            && message.created_at_ms >= pending.dispatched_at_ms
            && pending
                .dispatch_message_id
                .as_deref()
                .is_some_and(|id| message.in_reply_to.as_deref() == Some(id))
    })?;
    let summary = if reply.body.trim().is_empty() {
        format!("{} の作業が完了しました。", pending.target_agent_id)
    } else {
        neutralize_for_voice(&reply.body)
    };
    Some(completion(
        pending,
        CompletionVia::InboxReply,
        summary,
        reply.created_at_ms,
        Some(reply.id.clone()),
    ))
}

fn completion(
    pending: &PendingDispatch,
    via: CompletionVia,
    summary: String,
    completed_at_ms: i64,
    message_id: Option<String>,
) -> RealtimeCompletion {
    RealtimeCompletion {
        correlation_id: pending.correlation_id.clone(),
        kind: pending.kind,
        target_agent_id: pending.target_agent_id.clone(),
        task_id: pending.task_id.clone(),
        summary,
        completed_at_ms,
        objective: pending.objective.clone(),
        via: Some(via),
        message_id,
    }
}

fn neutralize_for_voice(text: &str) -> String {
    let collapsed = text
        .replace(['\r', '\n', '(', ')'], " ")
        .split_whitespace()
        .take(60)
        .collect::<Vec<_>>()
        .join(" ");
    collapsed.chars().take(300).collect()
}

#[cfg(test)]
mod tests {
    use md_web_contracts::domains::voice_realtime::{CompletionKind, CompletionVia};

    use super::{CompletionDetector, CompletionInboxMessage, CompletionTask, PendingDispatch};

    fn pending() -> PendingDispatch {
        PendingDispatch {
            correlation_id: String::from("dispatch-1"),
            kind: CompletionKind::Dispatch,
            target_agent_id: String::from("worker"),
            task_id: Some(String::from("task-1")),
            objective: None,
            dispatched_at_ms: 10,
            dispatch_message_id: Some(String::from("message-1")),
        }
    }

    #[test]
    fn done_card_emits_when_session_is_live() {
        let mut detector = CompletionDetector::default();
        detector.track(pending(), 10);
        detector.set_session_live(true);
        let events = detector.poll(
            &[CompletionTask {
                id: String::from("task-1"),
                status: String::from("done"),
                title: String::from("移植"),
            }],
            &[],
            20,
        );

        assert_eq!(
            events.first().and_then(|event| event.via),
            Some(CompletionVia::CardDone)
        );
    }

    #[test]
    fn completion_queues_while_session_is_closed() {
        let mut detector = CompletionDetector::default();
        detector.track(pending(), 10);
        let events = detector.poll(
            &[CompletionTask {
                id: String::from("task-1"),
                status: String::from("done"),
                title: String::from("移植"),
            }],
            &[],
            20,
        );

        assert!(events.is_empty());
        assert_eq!(detector.drain_queued().len(), 1);
    }

    #[test]
    fn matching_reply_completes_dispatch() {
        let mut record = pending();
        record.task_id = None;
        let mut detector = CompletionDetector::default();
        detector.track(record, 10);
        detector.set_session_live(true);
        let events = detector.poll(
            &[],
            &[CompletionInboxMessage {
                id: String::from("reply-1"),
                from: String::from("worker"),
                in_reply_to: Some(String::from("message-1")),
                body: String::from("完了しました"),
                created_at_ms: 20,
            }],
            20,
        );

        assert_eq!(
            events.first().and_then(|event| event.via),
            Some(CompletionVia::InboxReply)
        );
    }

    #[test]
    fn stale_pending_is_pruned() {
        let mut detector = CompletionDetector::default();
        detector.track(pending(), 10);
        let day_plus = 24 * 60 * 60 * 1_000 + 11;
        let _events = detector.poll(&[], &[], day_plus);

        assert_eq!(detector.pending_count(), 0);
    }
}
