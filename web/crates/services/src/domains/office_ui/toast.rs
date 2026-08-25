use md_web_contracts::domains::office_ui::CompletionNotice;

const MAX_VISIBLE: usize = 4;
const AUTO_DISMISS_MS: i64 = 9_000;

/// Deterministic reducer for the Dioxus completion-toast view.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CompletionToastStack {
    notices: Vec<CompletionNotice>,
}

impl CompletionToastStack {
    #[must_use]
    pub fn notices(&self) -> &[CompletionNotice] {
        &self.notices
    }

    pub fn push(&mut self, notice: CompletionNotice) {
        let duplicate = self.notices.iter().any(|current| {
            current.correlation_id == notice.correlation_id
                && current.completed_at_ms == notice.completed_at_ms
        });
        if duplicate {
            return;
        }
        if self.notices.len() == MAX_VISIBLE {
            self.notices.remove(0);
        }
        self.notices.push(notice);
    }

    pub fn dismiss(&mut self, correlation_id: &str, completed_at_ms: i64) {
        self.notices.retain(|notice| {
            notice.correlation_id != correlation_id || notice.completed_at_ms != completed_at_ms
        });
    }

    pub fn expire(&mut self, now_ms: i64) {
        self.notices
            .retain(|notice| now_ms.saturating_sub(notice.completed_at_ms) < AUTO_DISMISS_MS);
    }
}

#[cfg(test)]
mod tests {
    use md_web_contracts::domains::office_ui::CompletionNotice;

    use super::CompletionToastStack;

    fn notice(id: &str, completed_at_ms: i64) -> CompletionNotice {
        CompletionNotice {
            correlation_id: String::from(id),
            kind: String::from("task"),
            target_agent_id: String::from("god"),
            task_id: None,
            summary: String::from("完了"),
            completed_at_ms,
            objective: None,
        }
    }

    #[test]
    fn notices_start_empty() {
        assert!(CompletionToastStack::default().notices().is_empty());
    }

    #[test]
    fn push_deduplicates_same_completion() {
        let mut stack = CompletionToastStack::default();
        stack.push(notice("a", 1));
        stack.push(notice("a", 1));

        assert_eq!(stack.notices().len(), 1);
    }

    #[test]
    fn push_keeps_only_four_newest() {
        let mut stack = CompletionToastStack::default();
        for index in 0..5 {
            stack.push(notice(&index.to_string(), index));
        }

        assert_eq!(stack.notices()[0].correlation_id, "1");
    }

    #[test]
    fn dismiss_removes_exact_completion() {
        let mut stack = CompletionToastStack::default();
        stack.push(notice("a", 1));
        stack.dismiss("a", 1);

        assert!(stack.notices().is_empty());
    }

    #[test]
    fn expire_removes_notice_at_nine_seconds() {
        let mut stack = CompletionToastStack::default();
        stack.push(notice("a", 1));
        stack.expire(9_001);

        assert!(stack.notices().is_empty());
    }
}
