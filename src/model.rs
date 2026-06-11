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
}

#[derive(Clone, Debug)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub path: PathBuf,
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
                if !(entry.file_type().is_file() && entry.file_name() == "SKILL.md") {
                    continue;
                }
                let Some(skill_dir) = entry.path().parent() else {
                    continue;
                };
                match read_skill(skill_dir, entry.path()) {
                    Ok(skill) => skills.push(skill),
                    Err(error) => warnings.push(format!("{error:#}")),
                }
            }
        }

        skills.sort_by(|left, right| left.name.cmp(&right.name).then(left.path.cmp(&right.path)));
        skills.dedup_by(|left, right| left.name == right.name && left.path == right.path);

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
            .filter(|skill| skill.name == name)
            .collect()
    }

    pub fn status(&self, skill: &Skill, agent: Agent) -> LinkStatus {
        self.statuses
            .get(&(skill.path.clone(), agent))
            .cloned()
            .unwrap_or(LinkStatus::Missing)
    }

    pub fn status_label(&self, skill: &Skill, agent: Agent) -> &'static str {
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
    })
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
