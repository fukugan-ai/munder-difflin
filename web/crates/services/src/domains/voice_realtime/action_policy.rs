#![forbid(unsafe_code)]

use md_web_contracts::domains::voice_realtime::{RealtimeActionRequest, VoiceActionVerb};

const CONFIRM_TTL_MS: i64 = 120_000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VoicePolicyError {
    MissingTarget,
    MassTargetForbidden,
    GodTargetForbidden,
    SettingForbidden,
    InvalidSettingValue,
    MissingTypedInput,
    NoPendingAction,
    ConfirmationExpired,
    ConfirmationMismatch,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ActionDisposition {
    Execute(Box<RealtimeActionRequest>),
    AwaitConfirmation {
        pending_id: String,
        confirm_word: &'static str,
        spoken: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum ConfirmationOutcome {
    Execute(Box<RealtimeActionRequest>),
    Cancelled,
}

#[derive(Clone, Debug)]
struct PendingAction {
    id: String,
    request: RealtimeActionRequest,
    confirm_word: &'static str,
    created_at_ms: i64,
}

#[derive(Clone, Debug, Default)]
pub struct ActionPolicy {
    pending: Option<PendingAction>,
    next_id: u64,
}

impl ActionPolicy {
    pub fn propose(
        &mut self,
        request: RealtimeActionRequest,
        god_agent_id: Option<&str>,
        now_ms: i64,
    ) -> Result<ActionDisposition, VoicePolicyError> {
        self.pending = None;
        validate_request(&request, god_agent_id)?;
        if is_soft(&request)? {
            return Ok(ActionDisposition::Execute(Box::new(request)));
        }

        self.next_id = self.next_id.saturating_add(1);
        let id = format!("voice-confirm-{}", self.next_id);
        let confirm_word = confirm_word(request.verb);
        let target = request
            .agent_id
            .as_deref()
            .or(request.setting_key.as_deref())
            .or(request.title.as_deref())
            .unwrap_or("この操作");
        let spoken = format!(
            "{target}への操作を実行します。続けるには confirm または {confirm_word} と言ってください。"
        );
        self.pending = Some(PendingAction {
            id: id.clone(),
            request,
            confirm_word,
            created_at_ms: now_ms,
        });
        Ok(ActionDisposition::AwaitConfirmation {
            pending_id: id,
            confirm_word,
            spoken,
        })
    }

    pub fn confirm(
        &mut self,
        pending_id: &str,
        phrase: &str,
        now_ms: i64,
    ) -> Result<ConfirmationOutcome, VoicePolicyError> {
        let pending = self
            .pending
            .take()
            .ok_or(VoicePolicyError::NoPendingAction)?;
        if pending.id != pending_id {
            return Err(VoicePolicyError::ConfirmationMismatch);
        }
        if now_ms.saturating_sub(pending.created_at_ms) > CONFIRM_TTL_MS {
            return Err(VoicePolicyError::ConfirmationExpired);
        }
        let normalized = phrase.trim().to_ascii_lowercase();
        if normalized != "confirm"
            && !normalized
                .split_whitespace()
                .any(|word| word == pending.confirm_word)
        {
            return Err(VoicePolicyError::ConfirmationMismatch);
        }
        Ok(ConfirmationOutcome::Execute(Box::new(pending.request)))
    }

    pub fn cancel(&mut self) -> ConfirmationOutcome {
        self.pending = None;
        ConfirmationOutcome::Cancelled
    }

    pub fn pending_id(&self) -> Option<&str> {
        self.pending.as_ref().map(|pending| pending.id.as_str())
    }
}

fn validate_request(
    request: &RealtimeActionRequest,
    god_agent_id: Option<&str>,
) -> Result<(), VoicePolicyError> {
    if is_agent_targeted(request.verb) {
        let target = request
            .agent_id
            .as_deref()
            .filter(|target| !target.trim().is_empty())
            .ok_or(VoicePolicyError::MissingTarget)?;
        if matches!(
            target.trim().to_ascii_lowercase().as_str(),
            "all" | "everyone" | "*"
        ) {
            return Err(VoicePolicyError::MassTargetForbidden);
        }
        if is_god_forbidden(request.verb) && god_agent_id.is_some_and(|god| god == target) {
            return Err(VoicePolicyError::GodTargetForbidden);
        }
    }
    if request.verb == VoiceActionVerb::UpdateSetting {
        validate_setting(request)?;
    }
    match request.verb {
        VoiceActionVerb::Spawn if request.spawn_request.is_none() => {
            return Err(VoicePolicyError::MissingTypedInput);
        }
        VoiceActionVerb::EditSchedule | VoiceActionVerb::CreateSchedule => {
            request
                .mission
                .as_ref()
                .ok_or(VoicePolicyError::MissingTypedInput)?
                .validate()
                .map_err(|_| VoicePolicyError::InvalidSettingValue)?;
        }
        VoiceActionVerb::GateTool
            if request
                .tool_name
                .as_deref()
                .is_none_or(|tool| tool.trim().is_empty())
                || request.enabled.is_none() =>
        {
            return Err(VoicePolicyError::MissingTypedInput);
        }
        _ => {}
    }
    Ok(())
}

fn validate_setting(request: &RealtimeActionRequest) -> Result<(), VoicePolicyError> {
    let key = request
        .setting_key
        .as_deref()
        .ok_or(VoicePolicyError::SettingForbidden)?;
    let value = request
        .setting_value
        .as_deref()
        .ok_or(VoicePolicyError::InvalidSettingValue)?;
    match key {
        "notifications" | "freeflowEnabled" | "strongKeepalive" | "autoUpdate" | "autoMode"
        | "semanticMemory" => parse_bool(value),
        "terminalTheme" => one_of(value, &["light", "dark"]),
        "realtimeIdleDisconnectMs" => parse_range(value, 30_000, 3_600_000),
        "defaultModel" | "godProvider" | "godModel"
            if !value.trim().is_empty() && value.len() <= 200 =>
        {
            Ok(())
        }
        "defaultModel" | "godProvider" | "godModel" => Err(VoicePolicyError::InvalidSettingValue),
        _ => Err(VoicePolicyError::SettingForbidden),
    }
}

fn parse_bool(value: &str) -> Result<(), VoicePolicyError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "false" | "on" | "off" | "yes" | "no" | "1" | "0" => Ok(()),
        _ => Err(VoicePolicyError::InvalidSettingValue),
    }
}

fn parse_range(value: &str, min: i64, max: i64) -> Result<(), VoicePolicyError> {
    let parsed = value
        .trim()
        .parse::<i64>()
        .map_err(|_| VoicePolicyError::InvalidSettingValue)?;
    if (min..=max).contains(&parsed) {
        Ok(())
    } else {
        Err(VoicePolicyError::InvalidSettingValue)
    }
}

fn one_of(value: &str, allowed: &[&str]) -> Result<(), VoicePolicyError> {
    if allowed.contains(&value.trim()) {
        Ok(())
    } else {
        Err(VoicePolicyError::InvalidSettingValue)
    }
}

fn is_soft(request: &RealtimeActionRequest) -> Result<bool, VoicePolicyError> {
    if request.verb == VoiceActionVerb::UpdateSetting {
        let key = request
            .setting_key
            .as_deref()
            .ok_or(VoicePolicyError::SettingForbidden)?;
        return Ok(matches!(
            key,
            "notifications"
                | "terminalTheme"
                | "freeflowEnabled"
                | "strongKeepalive"
                | "autoUpdate"
                | "realtimeIdleDisconnectMs"
        ));
    }
    Ok(matches!(
        request.verb,
        VoiceActionVerb::Ping
            | VoiceActionVerb::Dispatch
            | VoiceActionVerb::Steer
            | VoiceActionVerb::CreateTask
            | VoiceActionVerb::AssignTask
            | VoiceActionVerb::UpdateTask
            | VoiceActionVerb::DeleteTask
            | VoiceActionVerb::WaitFor
            | VoiceActionVerb::Resume
            | VoiceActionVerb::AutoDelivery
            | VoiceActionVerb::GateTool
            | VoiceActionVerb::Unarchive
    ))
}

fn is_agent_targeted(verb: VoiceActionVerb) -> bool {
    matches!(
        verb,
        VoiceActionVerb::Ping
            | VoiceActionVerb::Dispatch
            | VoiceActionVerb::Steer
            | VoiceActionVerb::Kill
            | VoiceActionVerb::Pause
            | VoiceActionVerb::Halt
            | VoiceActionVerb::Resume
            | VoiceActionVerb::AutoDelivery
            | VoiceActionVerb::GateTool
            | VoiceActionVerb::Archive
            | VoiceActionVerb::Unarchive
            | VoiceActionVerb::ClearContext
    )
}

fn is_god_forbidden(verb: VoiceActionVerb) -> bool {
    matches!(
        verb,
        VoiceActionVerb::Kill
            | VoiceActionVerb::Pause
            | VoiceActionVerb::Halt
            | VoiceActionVerb::Archive
    )
}

fn confirm_word(verb: VoiceActionVerb) -> &'static str {
    match verb {
        VoiceActionVerb::Spawn => "spawn",
        VoiceActionVerb::Kill => "kill",
        VoiceActionVerb::Pause => "pause",
        VoiceActionVerb::Halt => "halt",
        VoiceActionVerb::Archive => "archive",
        VoiceActionVerb::ClearContext => "clear",
        VoiceActionVerb::EditSchedule | VoiceActionVerb::CreateSchedule => "schedule",
        VoiceActionVerb::UpdateSetting => "setting",
        _ => "confirm",
    }
}

#[cfg(test)]
mod tests {
    use md_web_contracts::domains::voice_realtime::{RealtimeActionRequest, VoiceActionVerb};

    use super::{ActionDisposition, ActionPolicy, ConfirmationOutcome, VoicePolicyError};

    fn request(verb: VoiceActionVerb, target: Option<&str>) -> RealtimeActionRequest {
        RealtimeActionRequest {
            verb,
            agent_id: target.map(String::from),
            task_id: None,
            text: None,
            title: None,
            objective: None,
            provider: None,
            setting_key: None,
            setting_value: None,
            spawn_request: None,
            mission: None,
            tool_name: None,
            enabled: None,
        }
    }

    fn pending_id(
        result: Result<ActionDisposition, VoicePolicyError>,
    ) -> Result<String, VoicePolicyError> {
        match result {
            Ok(ActionDisposition::AwaitConfirmation { pending_id, .. }) => Ok(pending_id),
            Ok(ActionDisposition::Execute(_)) => Err(VoicePolicyError::NoPendingAction),
            Err(error) => Err(error),
        }
    }

    #[test]
    fn ping_executes_without_confirmation() {
        let mut policy = ActionPolicy::default();
        let result = policy.propose(request(VoiceActionVerb::Ping, Some("worker")), None, 0);

        assert!(matches!(result, Ok(ActionDisposition::Execute(_))));
    }

    #[test]
    fn kill_requires_confirmation() {
        let mut policy = ActionPolicy::default();
        let result = policy.propose(request(VoiceActionVerb::Kill, Some("worker")), None, 0);

        assert!(matches!(
            result,
            Ok(ActionDisposition::AwaitConfirmation { .. })
        ));
    }

    #[test]
    fn mass_kill_is_forbidden() {
        let mut policy = ActionPolicy::default();
        let result = policy.propose(request(VoiceActionVerb::Kill, Some("all")), None, 0);

        assert_eq!(result, Err(VoicePolicyError::MassTargetForbidden));
    }

    #[test]
    fn gate_requires_typed_tool_state() {
        let mut policy = ActionPolicy::default();
        let result = policy.propose(request(VoiceActionVerb::GateTool, Some("worker")), None, 0);

        assert_eq!(result, Err(VoicePolicyError::MissingTypedInput));
    }

    #[test]
    fn god_kill_is_forbidden() {
        let mut policy = ActionPolicy::default();
        let result = policy.propose(request(VoiceActionVerb::Kill, Some("god")), Some("god"), 0);

        assert_eq!(result, Err(VoicePolicyError::GodTargetForbidden));
    }

    #[test]
    fn confirmation_expires() -> Result<(), VoicePolicyError> {
        let mut policy = ActionPolicy::default();
        let proposed = policy.propose(request(VoiceActionVerb::Kill, Some("worker")), None, 0);
        let pending_id = pending_id(proposed)?;
        let result = policy.confirm(&pending_id, "kill", 120_001);

        assert_eq!(result, Err(VoicePolicyError::ConfirmationExpired));
        Ok(())
    }

    #[test]
    fn exact_confirm_executes_pending_action() -> Result<(), VoicePolicyError> {
        let mut policy = ActionPolicy::default();
        let proposed = policy.propose(request(VoiceActionVerb::Kill, Some("worker")), None, 0);
        let pending_id = pending_id(proposed)?;
        let result = policy.confirm(&pending_id, "confirm", 1);

        assert!(matches!(result, Ok(ConfirmationOutcome::Execute(_))));
        Ok(())
    }
}
