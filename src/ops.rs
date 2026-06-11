use std::fs;

use anyhow::{Context, Result, bail};

use crate::model::{Agent, LinkStatus, Skill, link_path, link_status, paths_equivalent};

#[derive(Debug)]
pub struct OperationResult {
    pub message: String,
}

pub fn link_skill(skill: &Skill, agent: Agent) -> Result<OperationResult> {
    let link = link_path(skill, agent);

    if fs::symlink_metadata(&link).is_ok() {
        bail!("target already exists: {}", link.display());
    }

    if let Some(parent) = link.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }

    create_dir_symlink(&skill.path, &link)
        .with_context(|| format!("link {} -> {}", link.display(), skill.path.display()))?;

    Ok(OperationResult {
        message: format!("linked {}", link.display()),
    })
}

pub fn unlink_skill(skill: &Skill, agent: Agent) -> Result<OperationResult> {
    let link = link_path(skill, agent);
    let metadata = match fs::symlink_metadata(&link) {
        Ok(metadata) => metadata,
        Err(_) => {
            return Ok(OperationResult {
                message: "already missing".to_string(),
            });
        }
    };

    if !metadata.file_type().is_symlink() {
        bail!("refusing to remove non-symlink: {}", link.display());
    }

    let target = fs::read_link(&link).with_context(|| format!("read {}", link.display()))?;
    if !paths_equivalent(&target, &skill.path) {
        bail!(
            "refusing to remove symlink to different target: {} -> {}",
            link.display(),
            target.display()
        );
    }

    fs::remove_file(&link).with_context(|| format!("remove {}", link.display()))?;
    Ok(OperationResult {
        message: format!("unlinked {}", link.display()),
    })
}

pub fn fix_skill(skill: &Skill, agent: Agent) -> Result<OperationResult> {
    let link = link_path(skill, agent);
    match link_status(skill, agent) {
        LinkStatus::Linked => Ok(OperationResult {
            message: "already linked".to_string(),
        }),
        LinkStatus::Missing => link_skill(skill, agent),
        LinkStatus::WrongTarget(target) => {
            fs::remove_file(&link).with_context(|| format!("remove {}", link.display()))?;
            create_dir_symlink(&skill.path, &link)
                .with_context(|| format!("link {} -> {}", link.display(), skill.path.display()))?;
            Ok(OperationResult {
                message: format!("replaced {} -> {}", link.display(), target.display()),
            })
        }
        LinkStatus::Occupied => bail!("target is occupied: {}", link.display()),
    }
}

pub fn toggle_skill(skill: &Skill, agent: Agent) -> Result<OperationResult> {
    match link_status(skill, agent) {
        LinkStatus::Linked => unlink_skill(skill, agent),
        LinkStatus::Missing => link_skill(skill, agent),
        LinkStatus::WrongTarget(target) => bail!(
            "target points elsewhere: {} -> {}",
            link_path(skill, agent).display(),
            target.display()
        ),
        LinkStatus::Occupied => bail!("target is occupied: {}", link_path(skill, agent).display()),
    }
}

#[cfg(unix)]
fn create_dir_symlink(source: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(source, link)
}

#[cfg(windows)]
fn create_dir_symlink(source: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(source, link)
}
