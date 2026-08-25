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

    let assignments = runtime_skill_assignments(&receipt.role_skill_assignments);
    super::memory::save_base_skill_assignments(assignments).await?;

    let provider = pty_provider(&receipt.aria.provider)?;
    let project = receipt
        .aria
        .cwd
        .rsplit(['/', '\\'])
        .find(|part| !part.is_empty())
        .unwrap_or("workspace")
        .to_owned();
    let workspace_authority = recovery_workspace_authority(receipt)?;
    let team = [
        TeamMember::aria(),
        TeamMember::implementer(),
        TeamMember::verifier(),
    ];
    for member in team {
        let request = member.request(receipt, provider, project.clone());
        let persisted = repository
            .get_floor_agent("local", member.process_id)
            .await
            .map_err(|_| safe_error())?;
        let state =
            classify_persisted_member(persisted.as_ref(), &request.process, &workspace_authority)?;
        let previous_revision = persisted.as_ref().map(|value| value.revision);
        match state {
            PersistedMemberState::Missing => {
                super::office::office_spawn(request).await?;
            }
            PersistedMemberState::Active => {}
            PersistedMemberState::Archived => {
                let _transition_result =
                    super::pty::pty_unarchive(String::from(member.process_id)).await;
                verify_transition_postcondition(
                    &repository,
                    member.process_id,
                    &request.process,
                    &workspace_authority,
                    previous_revision,
                )
                .await?;
            }
            PersistedMemberState::Restorable => {
                let agent = persisted.ok_or_else(safe_error)?.agent;
                let _transition_result = super::pty::pty_restore(
                    md_web_contracts::domains::pty_agents::RestoreAgentRequest {
                        agent,
                        prefer_worktree: true,
                    },
                )
                .await;
                verify_transition_postcondition(
                    &repository,
                    member.process_id,
                    &request.process,
                    &workspace_authority,
                    previous_revision,
                )
                .await?;
            }
        }
        // A TeamStarting retry may observe a role whose process was persisted just
        // before an earlier skill-injection failure. Re-apply the role assignment
        // even when the existing process is reused so repair cannot confirm a team
        // that never received its assigned skills.
        let injection = super::memory::assigned_skill_injection(member.assignment_id, &[]).await?;
        super::pty::pty_queue_system(
            member.process_id,
            &skill_prompt(member.display_name, injection),
        )
        .await?;
    }
    super::office::office_snapshot().await?;
    Ok(())
}

#[cfg(feature = "server")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PersistedMemberState {
    Missing,
    Active,
    Archived,
    Restorable,
}

#[cfg(feature = "server")]
struct RecoveryWorkspaceAuthority {
    source_paths: Vec<std::path::PathBuf>,
    source_workspace_id: md_web_contracts::domains::fs_git_ide::WorkspaceId,
    private_root: md_web_services::domains::fs_git_ide::PrivateWorkspaceRoot,
}

#[cfg(feature = "server")]
fn recovery_workspace_authority(
    receipt: &FinishOnboardingResult,
) -> Result<RecoveryWorkspaceAuthority, ServerFnError> {
    let mut source_paths = std::env::var_os("MD_REGISTERED_REPOS")
        .map(|value| std::env::split_paths(&value).collect::<Vec<_>>())
        .unwrap_or_default();
    if source_paths.is_empty() {
        source_paths.extend(
            receipt
                .config
                .registered_repos
                .iter()
                .map(std::path::PathBuf::from),
        );
    }
    let registry = md_web_services::domains::fs_git_ide::WorkspaceRegistry::from_source_paths(
        source_paths.clone(),
    );
    let source_workspace_id = registry
        .list()
        .into_iter()
        .find(|workspace| workspace.display_path == receipt.aria.cwd)
        .map(|workspace| workspace.id)
        .ok_or_else(safe_error)?;
    let harness_home = std::env::var_os("MD_HARNESS_HOME")
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            receipt
                .config
                .harness_home
                .as_ref()
                .map(std::path::PathBuf::from)
        })
        .filter(|path| path.is_absolute())
        .ok_or_else(safe_error)?;
    let private_root = md_web_services::domains::fs_git_ide::PrivateWorkspaceRoot::new(
        harness_home.join("worktrees"),
    )
    .map_err(|_| safe_error())?;
    Ok(RecoveryWorkspaceAuthority {
        source_paths,
        source_workspace_id,
        private_root,
    })
}

#[cfg(feature = "server")]
fn classify_persisted_member(
    persisted: Option<&md_web_contracts::domains::persistence::PersistedFloorAgent>,
    expected: &md_web_contracts::domains::pty_agents::SpawnAgentRequest,
    workspace_authority: &RecoveryWorkspaceAuthority,
) -> Result<PersistedMemberState, ServerFnError> {
    use md_web_contracts::domains::pty_agents::AgentStatus;

    let Some(persisted) = persisted else {
        return Ok(PersistedMemberState::Missing);
    };
    if !persisted_member_matches(&persisted.agent, expected, workspace_authority) {
        return Err(ServerFnError::new(
            "保存済みの初期チームが現在の設定と一致しません。設定を確認してください。",
        ));
    }
    match persisted.agent.status {
        AgentStatus::Archived if persisted.agent.archived => Ok(PersistedMemberState::Archived),
        AgentStatus::Restorable => Ok(PersistedMemberState::Restorable),
        AgentStatus::Starting
        | AgentStatus::Idle
        | AgentStatus::Working
        | AgentStatus::Waiting
        | AgentStatus::Blocked
        | AgentStatus::Looping
            if !persisted.agent.archived && persisted.agent.pty_id.is_some() =>
        {
            Ok(PersistedMemberState::Active)
        }
        _ => Err(ServerFnError::new(
            "保存済みの初期チームを安全に再開できません。状態を確認してください。",
        )),
    }
}

#[cfg(feature = "server")]
fn persisted_member_matches(
    record: &md_web_contracts::domains::pty_agents::AgentRecord,
    expected: &md_web_contracts::domains::pty_agents::SpawnAgentRequest,
    workspace_authority: &RecoveryWorkspaceAuthority,
) -> bool {
    use md_web_contracts::domains::fs_git_ide::WorkspaceCapability;

    let Some(capability) = record.workspace_capability.as_ref() else {
        return false;
    };
    let admitted = md_web_services::domains::fs_git_ide::WorkspaceRegistry::from_source_paths(
        workspace_authority.source_paths.clone(),
    )
    .with_private_workspaces(&workspace_authority.private_root, [capability.clone()])
    .list()
    .into_iter()
    .any(|workspace| {
        workspace.id == capability.workspace_id
            && workspace.capability == WorkspaceCapability::PrivateMutable
            && workspace.display_path == capability.path
    });
    record.id == expected.id
        && record.name == expected.name
        && record.provider == expected.provider
        && record.role == expected.role
        && record.description == expected.description
        && record.command == expected.command
        && record.args == expected.args
        && record.model == expected.model
        && capability.source_workspace_id == workspace_authority.source_workspace_id
        && admitted
        && capability.path == record.cwd
        && record.worktree_path.as_deref() == Some(capability.path.as_str())
}

#[cfg(feature = "server")]
async fn verify_transition_postcondition(
    repository: &md_web_services::domains::persistence::PgPersistenceRepository,
    agent_id: &str,
    expected: &md_web_contracts::domains::pty_agents::SpawnAgentRequest,
    workspace_authority: &RecoveryWorkspaceAuthority,
    previous_revision: Option<i64>,
) -> Result<(), ServerFnError> {
    let current = repository
        .get_floor_agent("local", agent_id)
        .await
        .map_err(|_| safe_error())?
        .ok_or_else(safe_error)?;
    let is_active = classify_persisted_member(Some(&current), expected, workspace_authority)
        .is_ok_and(|state| state == PersistedMemberState::Active);
    let revision_advanced = previous_revision.is_none_or(|revision| current.revision > revision);
    if is_active && revision_advanced {
        Ok(())
    } else {
        Err(ServerFnError::new(
            "保存済みの初期チームを再開できませんでした。再試行してください。",
        ))
    }
}

#[cfg(feature = "server")]
fn runtime_skill_assignments(
    configured: &[md_web_contracts::domains::config_onboarding::RoleSkillAssignment],
) -> md_web_contracts::domains::memory_skills::TeamSkillAssignments {
    use md_web_contracts::domains::memory_skills::{
        RoleSkillAssignment, SoftwareTeamRole, TeamSkillAssignments,
    };

    TeamSkillAssignments {
        version: 1,
        assignments: configured
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
                        // `managed_id` identifies a catalog source/version (for example
                        // `2:local-development`). Runtime injection resolves the installed
                        // directory by its canonical skill name.
                        .map(|skill| skill.name.clone())
                        .collect(),
                    task_condition: None,
                }
            })
            .collect(),
        specialists_on_demand: true,
        updated_at_ms: 0,
    }
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
            isolate: true,
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

#[cfg(all(test, feature = "server"))]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use md_web_contracts::domains::config_onboarding::{RoleSkillAssignment, TeamRole};
    use md_web_contracts::domains::fs_git_ide::{PrivateWorkspaceCapability, WorkspaceId};
    use md_web_contracts::domains::memory_skills::{LocalSkill, SkillProvider, SkillScope};
    use md_web_contracts::domains::persistence::PersistedFloorAgent;
    use md_web_contracts::domains::pty_agents::{
        AgentProvider, AgentRecord, AgentRole, AgentStatus, SpawnAgentRequest,
    };

    use super::{
        PersistedMemberState, RecoveryWorkspaceAuthority, classify_persisted_member,
        runtime_skill_assignments,
    };

    struct TestAuthority {
        root: PathBuf,
        recovery: RecoveryWorkspaceAuthority,
    }

    impl Drop for TestAuthority {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn test_authority() -> Result<TestAuthority, Box<dyn std::error::Error>> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());
        let root = std::env::temp_dir().join(format!(
            "md-team-private-authority-{}-{nonce}",
            std::process::id()
        ));
        let source = root.join("source");
        let private = root.join("harness/worktrees");
        fs::create_dir_all(&source)?;
        fs::create_dir_all(&private)?;
        Ok(TestAuthority {
            root,
            recovery: RecoveryWorkspaceAuthority {
                source_paths: vec![source],
                source_workspace_id: WorkspaceId(String::from("source-1")),
                private_root: md_web_services::domains::fs_git_ide::PrivateWorkspaceRoot::new(
                    private,
                )?,
            },
        })
    }

    fn skill(name: &str) -> LocalSkill {
        LocalSkill {
            id: format!("user:{name}"),
            name: String::from(name),
            description: String::new(),
            provider: SkillProvider::Codex,
            scope: SkillScope::User,
            managed_id: format!("2:{name}"),
        }
    }

    #[test]
    fn namespaced_catalog_ids_become_canonical_runtime_skill_ids() {
        let configured = [
            (
                TeamRole::Aria,
                vec!["aria-orchestration", "local-development"],
            ),
            (TeamRole::Implementer, vec!["local-development"]),
            (TeamRole::Verifier, vec!["perfectionist-reviewer"]),
        ]
        .into_iter()
        .map(|(role, names)| RoleSkillAssignment {
            role,
            skills: names.into_iter().map(skill).collect(),
        })
        .collect::<Vec<_>>();

        let runtime = runtime_skill_assignments(&configured);

        assert_eq!(runtime.assignments.len(), 3);
        assert_eq!(runtime.assignments[0].skill_ids[0], "aria-orchestration");
        assert!(runtime.assignments.iter().all(|assignment| {
            assignment
                .skill_ids
                .iter()
                .all(|skill_id| !skill_id.contains(':'))
        }));
    }

    fn spawn_request(id: &str, name: &str, orchestrator: bool) -> SpawnAgentRequest {
        SpawnAgentRequest {
            id: String::from(id),
            name: String::from(name),
            provider: AgentProvider::Codex,
            role: AgentRole {
                orchestrator,
                assistant: !orchestrator,
            },
            description: format!("{name} description"),
            cwd: String::from("/source/repository"),
            command: String::from("codex"),
            args: Vec::new(),
            model: Some(String::from("gpt-5.6-codex")),
            cols: 100,
            rows: 30,
            isolate: true,
            resume: false,
            require_resume: false,
            resume_session_id: None,
        }
    }

    fn persisted(
        request: &SpawnAgentRequest,
        revision: i64,
        status: AgentStatus,
        authority: &RecoveryWorkspaceAuthority,
    ) -> Result<PersistedFloorAgent, Box<dyn std::error::Error>> {
        let capability_id = format!("wt-{}", request.id);
        let private_path = authority.private_root.path().join(&capability_id);
        fs::create_dir_all(&private_path)?;
        let private_path = private_path.to_string_lossy().into_owned();
        Ok(PersistedFloorAgent {
            floor_id: String::from("local"),
            revision,
            agent: AgentRecord {
                id: request.id.clone(),
                name: request.name.clone(),
                provider: request.provider,
                role: request.role,
                description: request.description.clone(),
                cwd: private_path.clone(),
                command: request.command.clone(),
                args: request.args.clone(),
                model: request.model.clone(),
                status,
                action_ja: String::from("待機中"),
                pty_id: (!matches!(status, AgentStatus::Archived | AgentStatus::Restorable))
                    .then(|| format!("pty-{}", request.id)),
                worktree_path: Some(private_path.clone()),
                workspace_capability: Some(PrivateWorkspaceCapability {
                    id: capability_id.clone(),
                    workspace_id: WorkspaceId(format!("private-{capability_id}")),
                    source_workspace_id: WorkspaceId(String::from("source-1")),
                    path: private_path,
                }),
                session_id: None,
                archived: status == AgentStatus::Archived,
            },
            updated_at_ms: 0,
        })
    }

    #[test]
    fn partial_archived_team_recovery_plan_is_idempotent() -> Result<(), Box<dyn std::error::Error>>
    {
        let authority = test_authority()?;
        let requests = [
            spawn_request("god", "Aria", true),
            spawn_request("implementer", "Implementer", false),
            spawn_request("verifier", "Verifier", false),
        ];
        let partial = [
            Some(persisted(
                &requests[0],
                2,
                AgentStatus::Archived,
                &authority.recovery,
            )?),
            Some(persisted(
                &requests[1],
                2,
                AgentStatus::Archived,
                &authority.recovery,
            )?),
            None,
        ];

        let initial = requests
            .iter()
            .zip(&partial)
            .map(|(request, record)| {
                classify_persisted_member(record.as_ref(), request, &authority.recovery)
            })
            .collect::<Result<Vec<_>, _>>();

        assert_eq!(
            initial.ok(),
            Some(vec![
                PersistedMemberState::Archived,
                PersistedMemberState::Archived,
                PersistedMemberState::Missing,
            ])
        );

        let completed = [
            persisted(&requests[0], 3, AgentStatus::Idle, &authority.recovery)?,
            persisted(&requests[1], 3, AgentStatus::Idle, &authority.recovery)?,
            persisted(&requests[2], 1, AgentStatus::Idle, &authority.recovery)?,
        ];
        let duplicate_retry = requests
            .iter()
            .zip(&completed)
            .map(|(request, record)| {
                classify_persisted_member(Some(record), request, &authority.recovery)
            })
            .collect::<Result<Vec<_>, _>>();

        assert_eq!(
            duplicate_retry.ok(),
            Some(vec![PersistedMemberState::Active; 3])
        );
        assert_eq!(completed.map(|record| record.revision), [3, 3, 1]);
        Ok(())
    }

    #[test]
    fn invalid_private_capability_identity_path_and_root_are_rejected()
    -> Result<(), Box<dyn std::error::Error>> {
        let authority = test_authority()?;
        let request = spawn_request("god", "Aria", true);
        let valid = persisted(&request, 2, AgentStatus::Archived, &authority.recovery)?;

        assert_eq!(
            classify_persisted_member(Some(&valid), &request, &authority.recovery).ok(),
            Some(PersistedMemberState::Archived)
        );

        let mut wrong_identity = valid.clone();
        if let Some(capability) = wrong_identity.agent.workspace_capability.as_mut() {
            capability.workspace_id = WorkspaceId(String::from("private-wrong"));
        }
        assert!(
            classify_persisted_member(Some(&wrong_identity), &request, &authority.recovery)
                .is_err()
        );

        let mut wrong_leaf = valid.clone();
        if let Some(capability) = wrong_leaf.agent.workspace_capability.as_mut() {
            capability.id = String::from("different-leaf");
        }
        assert!(
            classify_persisted_member(Some(&wrong_leaf), &request, &authority.recovery).is_err()
        );

        let outside = authority.root.join("outside/wt-god");
        fs::create_dir_all(&outside)?;
        let outside = outside.to_string_lossy().into_owned();
        let mut wrong_root = valid;
        wrong_root.agent.cwd.clone_from(&outside);
        wrong_root.agent.worktree_path = Some(outside.clone());
        if let Some(capability) = wrong_root.agent.workspace_capability.as_mut() {
            capability.path = outside;
        }

        assert!(
            classify_persisted_member(Some(&wrong_root), &request, &authority.recovery).is_err()
        );
        Ok(())
    }
}
