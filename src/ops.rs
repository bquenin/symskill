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

    remove_link(&link).with_context(|| format!("remove {}", link.display()))?;
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
            replace_link(&link, &skill.path)?;
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

/// Repoint an existing symlink at `source` without ever leaving the caller
/// with nothing: the old link is renamed aside first, and renamed back if
/// creating the replacement fails.
fn replace_link(link: &std::path::Path, source: &std::path::Path) -> Result<()> {
    let backup = backup_path(link);

    if let Ok(metadata) = fs::symlink_metadata(&backup) {
        if !metadata.file_type().is_symlink() {
            bail!("refusing to overwrite non-symlink: {}", backup.display());
        }
        remove_link(&backup).with_context(|| format!("remove {}", backup.display()))?;
    }

    fs::rename(link, &backup).with_context(|| format!("move {} aside", link.display()))?;

    match create_dir_symlink(source, link) {
        Ok(()) => {
            let _ = remove_link(&backup);
            Ok(())
        }
        Err(error) => match fs::rename(&backup, link) {
            Ok(()) => Err(error)
                .with_context(|| format!("link {} -> {}", link.display(), source.display())),
            Err(restore) => Err(error).with_context(|| {
                format!(
                    "link {} -> {} failed and the original link could not be restored \
                     from {} ({restore}); move it back by hand",
                    link.display(),
                    source.display(),
                    backup.display()
                )
            }),
        },
    }
}

fn backup_path(link: &std::path::Path) -> std::path::PathBuf {
    let mut name = link.file_name().unwrap_or_default().to_os_string();
    name.push(".symskill-old");
    link.with_file_name(name)
}

#[cfg(unix)]
fn create_dir_symlink(source: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(source, link)
}

#[cfg(windows)]
fn create_dir_symlink(source: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(source, link)
}

#[cfg(unix)]
fn remove_link(link: &std::path::Path) -> std::io::Result<()> {
    fs::remove_file(link)
}

/// Windows distinguishes file and directory symlinks: `remove_file` fails with
/// `ERROR_ACCESS_DENIED` on the directory symlinks `create_dir_symlink` makes.
#[cfg(windows)]
fn remove_link(link: &std::path::Path) -> std::io::Result<()> {
    use std::os::windows::fs::FileTypeExt;

    if fs::symlink_metadata(link)?.file_type().is_symlink_dir() {
        fs::remove_dir(link)
    } else {
        fs::remove_file(link)
    }
}
