use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use md_web_contracts::domains::memory_skills::{
    CatalogSkill, LocalSkill, SkillActionResponse, SkillProvider, SkillScope,
};

use super::DomainError;

const MAX_SKILL_FILES: usize = 60;
const MAX_SKILL_BYTES: u64 = 2 * 1024 * 1024;
const MAX_SKILL_DEPTH: usize = 5;

#[derive(Clone, Debug)]
pub struct SkillRoot {
    pub path: PathBuf,
    pub provider: SkillProvider,
    pub scope: SkillScope,
}

pub struct SkillService {
    roots: Vec<SkillRoot>,
    install_root: PathBuf,
}

impl SkillService {
    pub fn new(roots: Vec<SkillRoot>, install_root: PathBuf) -> Self {
        Self {
            roots,
            install_root,
        }
    }

    pub fn list_local(&self) -> Vec<LocalSkill> {
        let mut best = BTreeMap::<String, LocalSkill>::new();
        for (root_index, root) in self.roots.iter().enumerate() {
            let Ok(entries) = fs::read_dir(&root.path) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
                    continue;
                };
                let (skill_name, description) = if path.is_dir() {
                    let markdown = path.join("SKILL.md");
                    let Ok(text) = fs::read_to_string(markdown) else {
                        continue;
                    };
                    let frontmatter = parse_frontmatter(&text);
                    (
                        frontmatter.name.unwrap_or_else(|| name.to_owned()),
                        frontmatter.description.unwrap_or_default(),
                    )
                } else if root.provider != SkillProvider::Claude {
                    (name.to_owned(), String::from("CLI plugin"))
                } else {
                    continue;
                };
                let key = format!("{:?}:{}", root.provider, skill_name.to_lowercase());
                let candidate = LocalSkill {
                    id: format!("{:?}:{name}", root.scope).to_lowercase(),
                    name: skill_name,
                    description,
                    provider: root.provider,
                    scope: root.scope,
                    managed_id: format!("{root_index}:{name}"),
                };
                let replace = best
                    .get(&key)
                    .is_none_or(|current| scope_rank(candidate.scope) > scope_rank(current.scope));
                if replace {
                    best.insert(key, candidate);
                }
            }
        }
        best.into_values().collect()
    }

    pub fn parse_catalog(&self, markdown: &str) -> Vec<CatalogSkill> {
        parse_catalog(markdown)
    }

    pub fn install_from_staging(
        &self,
        staged_root: &Path,
        requested_name: &str,
    ) -> Result<SkillActionResponse, DomainError> {
        let directory = safe_skill_name(requested_name)?;
        if !staged_root.join("SKILL.md").is_file() {
            return Err(DomainError::InvalidInput("staged skill has no SKILL.md"));
        }
        let destination = self.install_root.join(&directory);
        if destination.exists() {
            return Ok(SkillActionResponse {
                ok: false,
                managed_id: None,
                error: Some(String::from("skill is already installed")),
                unsupported: false,
            });
        }
        let inventory = inventory(staged_root, staged_root, 0)?;
        let total = inventory.iter().try_fold(0_u64, |sum, (_, bytes)| {
            sum.checked_add(*bytes)
                .ok_or(DomainError::InvalidInput("skill size overflow"))
        })?;
        if inventory.len() > MAX_SKILL_FILES || total > MAX_SKILL_BYTES {
            return Err(DomainError::InvalidInput("skill exceeds install limits"));
        }
        fs::create_dir_all(&self.install_root)?;
        let temp = self.install_root.join(format!(".{directory}.installing"));
        if temp.exists() {
            fs::remove_dir_all(&temp)?;
        }
        fs::create_dir(&temp)?;
        for (source, _) in inventory {
            let relative = source
                .strip_prefix(staged_root)
                .map_err(|_| DomainError::OutsideManagedRoot)?;
            let target = temp.join(relative);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(source, target)?;
        }
        fs::rename(&temp, &destination)?;
        Ok(SkillActionResponse {
            ok: true,
            managed_id: Some(format!("install:{directory}")),
            error: None,
            unsupported: false,
        })
    }

    pub fn uninstall(&self, managed_id: &str) -> Result<SkillActionResponse, DomainError> {
        let (managed_root, name) = self.resolve_managed_id(managed_id)?;
        let root = managed_root.canonicalize()?;
        let target = root.join(name).canonicalize().map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                DomainError::NotFound
            } else {
                DomainError::Io(error)
            }
        })?;
        if target == root || !target.starts_with(&root) || !target.join("SKILL.md").is_file() {
            return Err(DomainError::OutsideManagedRoot);
        }
        fs::remove_dir_all(target)?;
        Ok(SkillActionResponse {
            ok: true,
            managed_id: None,
            error: None,
            unsupported: false,
        })
    }

    fn resolve_managed_id(&self, managed_id: &str) -> Result<(&Path, String), DomainError> {
        let (root_id, raw_name) = managed_id
            .split_once(':')
            .ok_or(DomainError::InvalidInput("invalid managed skill id"))?;
        let name = safe_skill_name(raw_name)?;
        if root_id == "install" {
            return Ok((&self.install_root, name));
        }
        let index = root_id
            .parse::<usize>()
            .map_err(|_| DomainError::InvalidInput("invalid managed skill id"))?;
        let root = self
            .roots
            .get(index)
            .ok_or(DomainError::InvalidInput("unknown managed skill root"))?;
        if root.scope == SkillScope::Bundled {
            return Err(DomainError::OutsideManagedRoot);
        }
        Ok((&root.path, name))
    }
}

struct Frontmatter {
    name: Option<String>,
    description: Option<String>,
}

fn parse_frontmatter(markdown: &str) -> Frontmatter {
    let Some(body) = markdown
        .strip_prefix("---\n")
        .and_then(|value| value.split_once("\n---"))
    else {
        return Frontmatter {
            name: None,
            description: None,
        };
    };
    let mut name = None;
    let mut description = None;
    let mut description_lines = Vec::new();
    let mut in_description = false;
    for line in body.0.lines() {
        if let Some(value) = line.strip_prefix("name:") {
            name = Some(clean_yaml_scalar(value));
            in_description = false;
        } else if let Some(value) = line.strip_prefix("description:") {
            let value = value.trim();
            in_description = matches!(value, "|" | ">" | "|-" | ">-");
            if !in_description && !value.is_empty() {
                description = Some(clean_yaml_scalar(value));
            }
        } else if in_description && (line.starts_with(' ') || line.starts_with('\t')) {
            let value = line.trim();
            if !value.is_empty() {
                description_lines.push(value);
            }
        } else {
            in_description = false;
        }
    }
    if !description_lines.is_empty() {
        description = Some(description_lines.join(" "));
    }
    Frontmatter { name, description }
}

fn clean_yaml_scalar(value: &str) -> String {
    value
        .trim()
        .trim_matches(|character| matches!(character, '\'' | '"'))
        .to_owned()
}

fn parse_catalog(markdown: &str) -> Vec<CatalogSkill> {
    let mut category = String::from("Skills");
    let mut skills = Vec::new();
    let mut seen = BTreeMap::<String, ()>::new();
    for line in markdown.lines().map(str::trim) {
        if let Some(heading) = line
            .strip_prefix("## ")
            .or_else(|| line.strip_prefix("### "))
        {
            category = heading
                .trim_start_matches(|character: char| !character.is_alphanumeric())
                .trim()
                .to_owned();
            continue;
        }
        if !line.starts_with('|') {
            continue;
        }
        let cells: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
        if cells.len() < 3 || cells[0].contains("---") {
            continue;
        }
        let name = cells[0].trim_matches('*').trim();
        let Some(url) = markdown_link(cells[2]) else {
            continue;
        };
        let Some(owner) = github_owner(url) else {
            continue;
        };
        let key = format!("{}:{name}", owner.to_lowercase());
        if name.eq_ignore_ascii_case("name") || seen.insert(key, ()).is_some() {
            continue;
        }
        skills.push(CatalogSkill {
            name: name.to_owned(),
            description: cells[1].to_owned(),
            url: url.to_owned(),
            category: category.clone(),
            owner: owner.to_owned(),
        });
    }
    skills
}

fn markdown_link(value: &str) -> Option<&str> {
    let start = value.find("https://")?;
    let tail = &value[start..];
    let end = tail.find([')', ' ', '>']).unwrap_or(tail.len());
    Some(&tail[..end])
}

fn github_owner(url: &str) -> Option<&str> {
    url.strip_prefix("https://github.com/")?
        .split('/')
        .next()
        .filter(|value| !value.is_empty())
}

fn safe_skill_name(value: &str) -> Result<String, DomainError> {
    if value.is_empty()
        || value.len() > 64
        || value.contains("..")
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(DomainError::InvalidInput("invalid skill name"));
    }
    Ok(value.to_owned())
}

fn inventory(
    root: &Path,
    current: &Path,
    depth: usize,
) -> Result<Vec<(PathBuf, u64)>, DomainError> {
    if depth > MAX_SKILL_DEPTH {
        return Err(DomainError::InvalidInput("skill nesting exceeds limit"));
    }
    let mut files = Vec::new();
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            files.extend(inventory(root, &entry.path(), depth + 1)?);
        } else if metadata.is_file() {
            if !entry.path().starts_with(root) {
                return Err(DomainError::OutsideManagedRoot);
            }
            files.push((entry.path(), metadata.len()));
        }
        if files.len() > MAX_SKILL_FILES {
            return Err(DomainError::InvalidInput("skill file count exceeds limit"));
        }
    }
    Ok(files)
}

fn scope_rank(scope: SkillScope) -> u8 {
    match scope {
        SkillScope::Project => 3,
        SkillScope::User => 2,
        SkillScope::Bundled => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_catalog, parse_frontmatter, safe_skill_name};

    #[test]
    fn multiline_description_is_joined() {
        let parsed = parse_frontmatter("---\nname: demo\ndescription: |\n  first\n  second\n---\n");

        assert_eq!(parsed.description.as_deref(), Some("first second"));
    }

    #[test]
    fn unsafe_skill_name_is_rejected() {
        assert!(safe_skill_name("../../escape").is_err());
    }

    #[test]
    fn catalog_requires_github_source() {
        let markdown =
            "## Utilities\n| **demo** | Demo | [Source](https://github.com/acme/demo) |\n";
        let skills = parse_catalog(markdown);

        assert_eq!(
            skills.first().map(|skill| skill.owner.as_str()),
            Some("acme")
        );
    }
}
