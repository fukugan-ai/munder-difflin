use std::fmt::{Display, Formatter};

use md_web_contracts::domains::config_onboarding::{RoleSkillAssignment, TeamRole};
use md_web_contracts::domains::memory_skills::LocalSkill;

const ARIA_SKILLS: &[&str] = &[
    "aria-orchestration",
    "graph-engineering",
    "project-documentation",
];
const IMPLEMENTER_SKILLS: &[&str] = &["local-development", "web-project-standards"];
const VERIFIER_SKILLS: &[&str] = &["perfectionist-reviewer"];

/// Failure resolving the selected base skill or mandatory software standards.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TeamSkillError {
    BaseSkillUnresolved,
    MandatorySkillUnresolved,
}

impl Display for TeamSkillError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::BaseSkillUnresolved => "selected base skill is unresolved",
            Self::MandatorySkillUnresolved => "mandatory software standard skill is unresolved",
        })
    }
}

impl std::error::Error for TeamSkillError {}

/// Builds the fixed three-person team from memory_skills' resolved LocalSkill DTOs.
pub fn resolve_minimal_team(
    resolved: &[LocalSkill],
    base_skill_managed_id: &str,
) -> Result<Vec<RoleSkillAssignment>, TeamSkillError> {
    let base = resolved
        .iter()
        .find(|skill| skill.managed_id == base_skill_managed_id)
        .cloned()
        .ok_or(TeamSkillError::BaseSkillUnresolved)?;
    Ok(vec![
        assignment(TeamRole::Aria, ARIA_SKILLS, &base, resolved)?,
        assignment(TeamRole::Implementer, IMPLEMENTER_SKILLS, &base, resolved)?,
        assignment(TeamRole::Verifier, VERIFIER_SKILLS, &base, resolved)?,
    ])
}

fn assignment(
    role: TeamRole,
    mandatory: &[&str],
    base: &LocalSkill,
    resolved: &[LocalSkill],
) -> Result<RoleSkillAssignment, TeamSkillError> {
    let mut skills = Vec::with_capacity(mandatory.len() + 1);
    for name in mandatory {
        let skill = resolved
            .iter()
            .find(|skill| skill.name.eq_ignore_ascii_case(name))
            .cloned()
            .ok_or(TeamSkillError::MandatorySkillUnresolved)?;
        skills.push(skill);
    }
    if !skills
        .iter()
        .any(|skill| skill.managed_id == base.managed_id)
    {
        skills.push(base.clone());
    }
    Ok(RoleSkillAssignment { role, skills })
}

#[cfg(test)]
mod tests {
    use md_web_contracts::domains::config_onboarding::TeamRole;
    use md_web_contracts::domains::memory_skills::{LocalSkill, SkillProvider, SkillScope};

    use super::{TeamSkillError, resolve_minimal_team};

    fn skill(name: &str) -> LocalSkill {
        LocalSkill {
            id: format!("user:{name}"),
            name: String::from(name),
            description: String::new(),
            provider: SkillProvider::Codex,
            scope: SkillScope::User,
            managed_id: format!("0:{name}"),
        }
    }

    fn resolved() -> Vec<LocalSkill> {
        [
            "aria-orchestration",
            "graph-engineering",
            "project-documentation",
            "local-development",
            "web-project-standards",
            "perfectionist-reviewer",
            "rust-base",
        ]
        .into_iter()
        .map(skill)
        .collect()
    }

    #[test]
    fn minimal_team_has_exact_three_roles_and_selected_base() {
        let assignments = resolve_minimal_team(&resolved(), "0:rust-base");

        assert!(matches!(assignments, Ok(value) if value.len() == 3
            && value[0].role == TeamRole::Aria
            && value[1].role == TeamRole::Implementer
            && value[2].role == TeamRole::Verifier
            && value.iter().all(|assignment| assignment.skills.iter().any(|skill| skill.managed_id == "0:rust-base"))));
    }

    #[test]
    fn missing_mandatory_standard_blocks_completion() {
        let skills = vec![skill("rust-base")];

        assert_eq!(
            resolve_minimal_team(&skills, "0:rust-base"),
            Err(TeamSkillError::MandatorySkillUnresolved)
        );
    }

    #[test]
    fn unresolved_base_selection_blocks_completion() {
        assert_eq!(
            resolve_minimal_team(&resolved(), "missing"),
            Err(TeamSkillError::BaseSkillUnresolved)
        );
    }
}
