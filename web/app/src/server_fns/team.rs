use dioxus::prelude::*;
#[cfg(feature = "server")]
use md_web_contracts::domains::config_onboarding::{
    AgentProvider as ConfigAgentProvider, ConfirmTeamInitializedRequest, OnboardingPhase, TeamRole,
};
use md_web_contracts::domains::config_onboarding::{
    ConfirmTeamInitializedResult, FinishOnboardingResult,
};

#[post("/api/config/onboarding/team")]
pub(crate) async fn onboarding_spawn_team(
    receipt: FinishOnboardingResult,
) -> Result<ConfirmTeamInitializedResult, ServerFnError> {
    #[cfg(feature = "server")]
    {
        spawn_team(&receipt).await?;
        super::onboarding_confirm_team(ConfirmTeamInitializedRequest {
            expected_revision: receipt.config.revision,
            initialized_roles: vec![TeamRole::Aria, TeamRole::Implementer, TeamRole::Verifier],
        })
        .await
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = receipt;
        Err(ServerFnError::new("初期チームを起動できません"))
    }
}

#[cfg(feature = "server")]
async fn spawn_team(receipt: &FinishOnboardingResult) -> Result<(), ServerFnError> {
    use md_web_contracts::domains::memory_skills::{
        RoleSkillAssignment, SoftwareTeamRole, TeamSkillAssignments,
    };

    let repository = super::persistence_repository().await?;
    let persisted = md_web_services::domains::config_onboarding::load_config(&repository)
        .await
        .map_err(|_| safe_error())?;
    if persisted.onboarding_phase != OnboardingPhase::TeamStarting
        || persisted.team_initialized
        || persisted != receipt.config
    {
        return Err(ServerFnError::new(
            "初期設定が更新されています。内容を読み直して再試行してください。",
        ));
    }
    if receipt.aria.id != "god" || receipt.aria.name != "Aria" || !receipt.aria.orchestrator {
        return Err(safe_error());
    }

    let assignments = TeamSkillAssignments {
        version: 1,
        assignments: receipt
            .role_skill_assignments
            .iter()
            .map(|assignment| {
                let (agent_id, display_name, role) = match assignment.role {
                    TeamRole::Aria => ("aria", "Aria", SoftwareTeamRole::Orchestrator),
                    TeamRole::Implementer => {
                        ("implementer", "Implementer", SoftwareTeamRole::Implementer)
                    }
                    TeamRole::Verifier => ("verifier", "Verifier", SoftwareTeamRole::Verifier),
                };
                RoleSkillAssignment {
                    agent_id: String::from(agent_id),
                    display_name: String::from(display_name),
                    role,
                    skill_ids: assignment
                        .skills
                        .iter()
                        .map(|skill| skill.managed_id.clone())
                        .collect(),
                    task_condition: None,
                }
            })
            .collect(),
        specialists_on_demand: true,
        updated_at_ms: 0,
    };
    super::memory::save_base_skill_assignments(assignments).await?;

    let provider = pty_provider(&receipt.aria.provider)?;
    let project = receipt
        .aria
        .cwd
        .rsplit(['/', '\\'])
        .find(|part| !part.is_empty())
        .unwrap_or("workspace")
        .to_owned();
    let team = [
        TeamMember::aria(),
        TeamMember::implementer(),
        TeamMember::verifier(),
    ];
    for member in team {
        let (active, _) = super::pty::list_agents().await?;
        let already_active = active.iter().any(|agent| {
            agent.id == member.process_id
                && agent.name == member.display_name
                && agent.provider == provider
                && agent.cwd == receipt.aria.cwd
        });
        if !already_active {
            super::office::office_spawn(member.request(receipt, provider, project.clone())).await?;
            let injection =
                super::memory::assigned_skill_injection(member.assignment_id, &[]).await?;
            super::pty::pty_queue_system(
                member.process_id,
                &skill_prompt(member.display_name, injection),
            )
            .await?;
        }
    }
    Ok(())
}

#[cfg(feature = "server")]
#[derive(Clone, Copy)]
struct TeamMember {
    process_id: &'static str,
    assignment_id: &'static str,
    display_name: &'static str,
    description: &'static str,
    goal: &'static str,
    character: md_web_contracts::domains::office_ui::OfficeCharacter,
    accent: md_web_contracts::domains::office_ui::Accent,
    orchestrator: bool,
    isolate: bool,
}

#[cfg(feature = "server")]
impl TeamMember {
    const fn aria() -> Self {
        Self {
            process_id: "god",
            assignment_id: "aria",
            display_name: "Aria",
            description: "オフィス全体を統括するAria",
            goal: "オフィス全体を統括する",
            character: md_web_contracts::domains::office_ui::OfficeCharacter::Michael,
            accent: md_web_contracts::domains::office_ui::Accent::Lemon,
            orchestrator: true,
            isolate: false,
        }
    }

    const fn implementer() -> Self {
        Self {
            process_id: "implementer",
            assignment_id: "implementer",
            display_name: "Implementer",
            description: "割り当てられた変更を実装する担当",
            goal: "承認済みの変更を実装する",
            character: md_web_contracts::domains::office_ui::OfficeCharacter::Jim,
            accent: md_web_contracts::domains::office_ui::Accent::Sky,
            orchestrator: false,
            isolate: true,
        }
    }

    const fn verifier() -> Self {
        Self {
            process_id: "verifier",
            assignment_id: "verifier",
            display_name: "Verifier",
            description: "実装結果を独立して検証する担当",
            goal: "実装の完了条件を検証する",
            character: md_web_contracts::domains::office_ui::OfficeCharacter::Dwight,
            accent: md_web_contracts::domains::office_ui::Accent::Mint,
            orchestrator: false,
            isolate: true,
        }
    }

    fn request(
        self,
        receipt: &FinishOnboardingResult,
        provider: md_web_contracts::domains::pty_agents::AgentProvider,
        project: String,
    ) -> md_web_contracts::domains::office_ui::OfficeAgentSpawnRequest {
        md_web_contracts::domains::office_ui::OfficeAgentSpawnRequest {
            process: md_web_contracts::domains::pty_agents::SpawnAgentRequest {
                id: String::from(self.process_id),
                name: String::from(self.display_name),
                provider,
                role: md_web_contracts::domains::pty_agents::AgentRole {
                    orchestrator: self.orchestrator,
                    assistant: !self.orchestrator,
                },
                description: String::from(self.description),
                cwd: receipt.aria.cwd.clone(),
                command: receipt.aria.command.clone(),
                args: Vec::new(),
                model: Some(receipt.aria.model.clone()),
                cols: 100,
                rows: 30,
                isolate: self.isolate,
                resume: false,
                require_resume: false,
                resume_session_id: None,
            },
            character: self.character,
            accent: self.accent,
            project,
            goal: String::from(self.goal),
        }
    }
}

#[cfg(feature = "server")]
fn pty_provider(
    provider: &ConfigAgentProvider,
) -> Result<md_web_contracts::domains::pty_agents::AgentProvider, ServerFnError> {
    use md_web_contracts::domains::pty_agents::AgentProvider;
    match provider {
        ConfigAgentProvider::Claude => Ok(AgentProvider::Claude),
        ConfigAgentProvider::Codex => Ok(AgentProvider::Codex),
        _ => Err(ServerFnError::new(
            "選択したエンジンでは初期チームを起動できません。",
        )),
    }
}

#[cfg(feature = "server")]
fn skill_prompt(
    display_name: &str,
    injection: md_web_services::domains::memory_skills::AgentSkillInjection,
) -> String {
    let mut prompt =
        format!("{display_name}として開始します。以下はこの役割へ割り当てられたスキルだけです。\n");
    for skill in injection.skills {
        prompt.push_str("\n--- ASSIGNED SKILL: ");
        prompt.push_str(&skill.skill_id);
        prompt.push_str(" ---\nPATH: ");
        prompt.push_str(&skill.path.to_string_lossy());
        prompt.push('\n');
        prompt.push_str(&skill.instructions);
        prompt.push_str("\n--- END ASSIGNED SKILL ---\n");
    }
    prompt
}

#[cfg(feature = "server")]
fn safe_error() -> ServerFnError {
    ServerFnError::new("初期チームを起動できません")
}
