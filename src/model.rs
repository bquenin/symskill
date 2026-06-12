use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use dirs::home_dir;
use walkdir::{DirEntry, WalkDir};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Agent {
    Claude,
    Codex,
    Cursor,
}

impl Agent {
    pub const ALL: [Agent; 3] = [Agent::Claude, Agent::Codex, Agent::Cursor];

    pub fn label(self) -> &'static str {
        match self {
            Agent::Claude => "claude",
            Agent::Codex => "codex",
            Agent::Cursor => "cursor",
        }
    }

    pub fn skills_dir(self) -> PathBuf {
        let home = home_dir().unwrap_or_else(|| PathBuf::from("."));
        match self {
            Agent::Claude => home.join(".claude").join("skills"),
            Agent::Codex => home.join(".codex").join("skills"),
            Agent::Cursor => home.join(".cursor").join("skills"),
        }
    }

    /// Agents whose skill directories this agent also scans, always and
    /// without an opt-out: Cursor picks up claude and codex links by itself.
    pub fn inherits_from(self) -> &'static [Agent] {
        match self {
            Agent::Cursor => &[Agent::Claude, Agent::Codex],
            Agent::Claude | Agent::Codex => &[],
        }
    }
}

#[derive(Clone, Debug)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub path: PathBuf,
    /// Directory containing the skill, relative to its source root.
    /// Empty for top-level skills, `/`-separated for nested ones (e.g. `ops/db`).
    pub group: String,
}

impl Skill {
    pub fn qualified_name(&self) -> String {
        if self.group.is_empty() {
            self.name.clone()
        } else {
            format!("{}/{}", self.group, self.name)
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LinkStatus {
    Linked,
    Missing,
    WrongTarget(PathBuf),
    Occupied,
}

#[derive(Debug)]
pub struct Inventory {
    pub skills: Vec<Skill>,
    pub statuses: BTreeMap<(PathBuf, Agent), LinkStatus>,
    pub warnings: Vec<String>,
}

impl Inventory {
    pub fn load(sources: &[PathBuf]) -> Self {
        let mut skills = Vec::new();
        let mut warnings = Vec::new();

        for source in sources {
            let source = expand_tilde(source);
            if !source.exists() {
                warnings.push(format!("source not found: {}", source.display()));
                continue;
            }
            let source = fs::canonicalize(&source).unwrap_or(source);

            let mut found = Vec::new();
            for entry in WalkDir::new(&source)
                .follow_links(false)
                .into_iter()
                .filter_entry(should_descend)
            {
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(error) => {
                        warnings.push(format!("skipped: {error}"));
                        continue;
                    }
                };
                let is_skill_file = entry.file_type().is_file()
                    && entry
                        .file_name()
                        .to_string_lossy()
                        .eq_ignore_ascii_case("SKILL.md");
                if !is_skill_file {
                    continue;
                }
                let Some(skill_dir) = entry.path().parent() else {
                    continue;
                };
                match read_skill(skill_dir, entry.path()) {
                    Ok(skill) => found.push((skill, group_components(&source, skill_dir))),
                    Err(error) => warnings.push(format!("{error:#}")),
                }
            }

            // Group relative to the deepest directory containing all of this
            // source's skills, so `--source repo` and `--source repo/skills`
            // produce the same view.
            let shared = common_group_prefix(&found);
            for (mut skill, components) in found {
                skill.group = components[shared..].join("/");
                skills.push(skill);
            }
        }

        // Dedup on (name, path) first: overlapping sources can yield the same
        // skill with different groups, which would not be adjacent below.
        skills.sort_by(|left, right| left.path.cmp(&right.path).then(left.name.cmp(&right.name)));
        skills.dedup_by(|left, right| left.name == right.name && left.path == right.path);
        skills.sort_by(|left, right| {
            left.group
                .cmp(&right.group)
                .then(left.name.cmp(&right.name))
                .then(left.path.cmp(&right.path))
        });

        let mut statuses = BTreeMap::new();
        for skill in &skills {
            for agent in Agent::ALL {
                statuses.insert((skill.path.clone(), agent), link_status(skill, agent));
            }
        }

        Self {
            skills,
            statuses,
            warnings,
        }
    }

    pub fn find_all(&self, name: &str) -> Vec<&Skill> {
        self.skills
            .iter()
            .filter(|skill| skill.name == name || skill.qualified_name() == name)
            .collect()
    }

    pub fn status(&self, skill: &Skill, agent: Agent) -> LinkStatus {
        self.statuses
            .get(&(skill.path.clone(), agent))
            .cloned()
            .unwrap_or(LinkStatus::Missing)
    }

    /// The agent whose link makes this skill reachable for `agent` when it
    /// has no direct link of its own (e.g. Cursor reading claude/codex dirs).
    pub fn inherited_via(&self, skill: &Skill, agent: Agent) -> Option<Agent> {
        if self.status(skill, agent) != LinkStatus::Missing {
            return None;
        }
        agent
            .inherits_from()
            .iter()
            .copied()
            .find(|source| self.status(skill, *source) == LinkStatus::Linked)
    }

    pub fn status_label(&self, skill: &Skill, agent: Agent) -> &'static str {
        if self.inherited_via(skill, agent).is_some() {
            return "inherit";
        }
        match self.status(skill, agent) {
            LinkStatus::Linked => "linked",
            LinkStatus::Missing => "missing",
            LinkStatus::WrongTarget(_) => "wrong",
            LinkStatus::Occupied => "blocked",
        }
    }
}

pub fn link_path(skill: &Skill, agent: Agent) -> PathBuf {
    agent.skills_dir().join(&skill.name)
}

fn should_descend(entry: &DirEntry) -> bool {
    let name = entry.file_name().to_string_lossy();
    !(entry.file_type().is_dir()
        && (name.starts_with('.')
            || name == "__pycache__"
            || name == "node_modules"
            || name == "target"))
}

fn read_skill(skill_dir: &Path, skill_file: &Path) -> Result<Skill> {
    let content =
        fs::read_to_string(skill_file).with_context(|| format!("read {}", skill_file.display()))?;
    let dir_name = skill_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("unknown")
        .to_string();
    let (name, description) = parse_frontmatter(&content, &dir_name);

    Ok(Skill {
        name,
        description,
        path: skill_dir.to_path_buf(),
        group: String::new(),
    })
}

/// Components of the directory containing `skill_dir`, relative to `source`.
/// Empty when the skill sits directly under the source root.
fn group_components(source: &Path, skill_dir: &Path) -> Vec<String> {
    skill_dir
        .strip_prefix(source)
        .ok()
        .and_then(|relative| relative.parent())
        .map(|parent| {
            parent
                .components()
                .map(|component| component.as_os_str().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// Number of leading group components shared by every skill in a source.
fn common_group_prefix(found: &[(Skill, Vec<String>)]) -> usize {
    let Some((_, first)) = found.first() else {
        return 0;
    };
    let mut shared = first.len();
    for (_, components) in &found[1..] {
        let mut common = 0;
        while common < shared && common < components.len() && components[common] == first[common] {
            common += 1;
        }
        shared = common;
        if shared == 0 {
            break;
        }
    }
    shared
}

fn parse_frontmatter(content: &str, fallback_name: &str) -> (String, String) {
    let Some(body) = content
        .strip_prefix("---\n")
        .or_else(|| content.strip_prefix("---\r\n"))
    else {
        return (fallback_name.to_string(), String::new());
    };

    let lines: Vec<&str> = body
        .lines()
        .take_while(|line| line.trim_end() != "---")
        .collect();

    let mut name = None;
    let mut description = None;
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        if let Some(value) = line.strip_prefix("name:") {
            let (value, next) = parse_scalar(value, &lines, index);
            name = Some(value);
            index = next;
        } else if let Some(value) = line.strip_prefix("description:") {
            let (value, next) = parse_scalar(value, &lines, index);
            description = Some(value);
            index = next;
        } else {
            index += 1;
        }
    }

    (
        name.filter(|value| !value.is_empty())
            .unwrap_or_else(|| fallback_name.to_string()),
        description.unwrap_or_default(),
    )
}

/// Parse a YAML scalar that is either inline (`key: value`) or a block scalar
/// (`key: >-` followed by indented lines). Block lines are joined with spaces.
fn parse_scalar(inline: &str, lines: &[&str], start: usize) -> (String, usize) {
    let inline = inline.trim();
    if !matches!(inline, ">" | ">-" | ">+" | "|" | "|-" | "|+") {
        return (unquote(inline), start + 1);
    }

    let mut parts = Vec::new();
    let mut index = start + 1;
    while index < lines.len() {
        let line = lines[index];
        if !line.trim().is_empty() && !line.starts_with([' ', '\t']) {
            break;
        }
        let text = line.trim();
        if !text.is_empty() {
            parts.push(text);
        }
        index += 1;
    }
    (parts.join(" "), index)
}

fn unquote(value: &str) -> String {
    let value = value.trim();
    let bytes = value.as_bytes();
    if bytes.len() >= 2
        && (bytes[0] == b'"' || bytes[0] == b'\'')
        && bytes[bytes.len() - 1] == bytes[0]
    {
        value[1..value.len() - 1].to_string()
    } else {
        value.to_string()
    }
}

pub fn link_status(skill: &Skill, agent: Agent) -> LinkStatus {
    let link = link_path(skill, agent);
    let metadata = match fs::symlink_metadata(&link) {
        Ok(metadata) => metadata,
        Err(_) => return LinkStatus::Missing,
    };

    if !metadata.file_type().is_symlink() {
        return LinkStatus::Occupied;
    }

    match fs::read_link(&link) {
        Ok(target) if paths_equivalent(&target, &skill.path) => LinkStatus::Linked,
        Ok(target) => LinkStatus::WrongTarget(target),
        Err(_) => LinkStatus::Occupied,
    }
}

pub fn paths_equivalent(left: &Path, right: &Path) -> bool {
    match (fs::canonicalize(left), fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

fn expand_tilde(path: &Path) -> PathBuf {
    let Some(raw) = path.to_str() else {
        return path.to_path_buf();
    };
    if raw == "~" {
        return home_dir().unwrap_or_else(|| path.to_path_buf());
    }
    if let Some(rest) = raw.strip_prefix("~/")
        && let Some(home) = home_dir()
    {
        return home.join(rest);
    }
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn components(source: &str, skill_dir: &str) -> Vec<String> {
        group_components(Path::new(source), Path::new(skill_dir))
    }

    fn skill_at(dir: &str) -> Skill {
        Skill {
            name: "skill".to_string(),
            description: String::new(),
            path: PathBuf::from(dir),
            group: String::new(),
        }
    }

    #[test]
    fn group_components_is_empty_for_top_level_skills() {
        assert!(components("/src", "/src/jira").is_empty());
        assert!(components("/src", "/src").is_empty());
    }

    #[test]
    fn group_components_uses_parent_directory_for_nested_skills() {
        assert_eq!(components("/src", "/src/ops/db-restore"), vec!["ops"]);
        assert_eq!(components("/src", "/src/a/b/c"), vec!["a", "b"]);
    }

    #[test]
    fn common_group_prefix_collapses_shared_ancestors() {
        let found = vec![
            (
                skill_at("/repo/skills/jira"),
                components("/repo", "/repo/skills/jira"),
            ),
            (
                skill_at("/repo/skills/ops/db"),
                components("/repo", "/repo/skills/ops/db"),
            ),
        ];
        assert_eq!(common_group_prefix(&found), 1);
    }

    #[test]
    fn common_group_prefix_is_zero_with_top_level_skills() {
        let found = vec![
            (skill_at("/src/jira"), components("/src", "/src/jira")),
            (skill_at("/src/ops/db"), components("/src", "/src/ops/db")),
        ];
        assert_eq!(common_group_prefix(&found), 0);
        assert_eq!(common_group_prefix(&[]), 0);
    }

    #[test]
    fn qualified_name_prefixes_group() {
        let mut skill = skill_at("/src/ops/db-restore");
        skill.name = "db-restore".to_string();
        assert_eq!(skill.qualified_name(), "db-restore");

        skill.group = "ops".to_string();
        assert_eq!(skill.qualified_name(), "ops/db-restore");
    }

    fn inventory_with(skill: &Skill, statuses: &[(Agent, LinkStatus)]) -> Inventory {
        Inventory {
            skills: vec![skill.clone()],
            statuses: statuses
                .iter()
                .map(|(agent, status)| ((skill.path.clone(), *agent), status.clone()))
                .collect(),
            warnings: Vec::new(),
        }
    }

    #[test]
    fn cursor_inherits_from_claude_then_codex_links() {
        let skill = skill_at("/src/jira");

        let via_claude = inventory_with(&skill, &[(Agent::Claude, LinkStatus::Linked)]);
        assert_eq!(
            via_claude.inherited_via(&skill, Agent::Cursor),
            Some(Agent::Claude)
        );
        assert_eq!(via_claude.status_label(&skill, Agent::Cursor), "inherit");

        let via_codex = inventory_with(&skill, &[(Agent::Codex, LinkStatus::Linked)]);
        assert_eq!(
            via_codex.inherited_via(&skill, Agent::Cursor),
            Some(Agent::Codex)
        );
    }

    #[test]
    fn no_inheritance_without_a_linked_source_or_with_a_direct_link() {
        let skill = skill_at("/src/jira");

        let unlinked = inventory_with(&skill, &[]);
        assert_eq!(unlinked.inherited_via(&skill, Agent::Cursor), None);

        let direct = inventory_with(
            &skill,
            &[
                (Agent::Claude, LinkStatus::Linked),
                (Agent::Cursor, LinkStatus::Linked),
            ],
        );
        assert_eq!(direct.inherited_via(&skill, Agent::Cursor), None);
        assert_eq!(direct.status_label(&skill, Agent::Cursor), "linked");

        let wrong = inventory_with(
            &skill,
            &[
                (Agent::Claude, LinkStatus::Linked),
                (Agent::Cursor, LinkStatus::WrongTarget(PathBuf::from("/x"))),
            ],
        );
        assert_eq!(
            wrong.inherited_via(&skill, Agent::Cursor),
            None,
            "a wrong direct link is its own problem, not inheritance"
        );
    }

    #[test]
    fn claude_and_codex_never_inherit() {
        let skill = skill_at("/src/jira");
        let inventory = inventory_with(
            &skill,
            &[
                (Agent::Claude, LinkStatus::Linked),
                (Agent::Codex, LinkStatus::Linked),
            ],
        );
        assert_eq!(inventory.inherited_via(&skill, Agent::Claude), None);
        assert_eq!(inventory.inherited_via(&skill, Agent::Codex), None);
    }
}
