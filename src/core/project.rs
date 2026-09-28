//! Read-only selection of a Git working checkout.

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The selected checkout and the Git directory shared by its linked worktrees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub worktree_root: PathBuf,
    pub git_common_dir: PathBuf,
}

#[derive(Debug)]
pub enum DiscoveryError {
    UnsupportedProject(PathBuf),
    GitUnavailable(std::io::Error),
    Io(std::io::Error),
}

impl fmt::Display for DiscoveryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedProject(path) => write!(
                f,
                "unsupported project at {}: expected a Git working checkout",
                path.display()
            ),
            Self::GitUnavailable(error) => write!(f, "Git is unavailable: {error}"),
            Self::Io(error) => write!(f, "cannot resolve project path: {error}"),
        }
    }
}

impl std::error::Error for DiscoveryError {}

/// Discover from `selected`, or from the caller's current directory.
///
/// This only invokes Git's read-only `rev-parse` queries. It never changes the
/// caller's checkout or creates Work runtime storage.
pub fn discover(selected: Option<&Path>) -> Result<Project, DiscoveryError> {
    let path = match selected {
        Some(path) => path.to_path_buf(),
        None => std::env::current_dir().map_err(DiscoveryError::Io)?,
    };
    let path = path.canonicalize().map_err(DiscoveryError::Io)?;
    if !path.is_dir() {
        return Err(DiscoveryError::UnsupportedProject(path));
    }

    let output = Command::new("git")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_COMMON_DIR")
        .arg("-C")
        .arg(&path)
        .args([
            "rev-parse",
            "--is-inside-work-tree",
            "--show-toplevel",
            "--path-format=absolute",
            "--git-common-dir",
        ])
        .output()
        .map_err(DiscoveryError::GitUnavailable)?;
    if !output.status.success() {
        return Err(DiscoveryError::UnsupportedProject(path));
    }
    let stdout = String::from_utf8(output.stdout)
        .map_err(|_| DiscoveryError::UnsupportedProject(path.clone()))?;
    let mut lines = stdout.lines();
    if lines.next() != Some("true") {
        return Err(DiscoveryError::UnsupportedProject(path));
    }
    let root = lines
        .next()
        .ok_or_else(|| DiscoveryError::UnsupportedProject(path.clone()))?;
    let common = lines
        .next()
        .ok_or_else(|| DiscoveryError::UnsupportedProject(path.clone()))?;
    if root.is_empty() || common.is_empty() || lines.next().is_some() {
        return Err(DiscoveryError::UnsupportedProject(path));
    }
    Ok(Project {
        worktree_root: PathBuf::from(root)
            .canonicalize()
            .map_err(DiscoveryError::Io)?,
        git_common_dir: PathBuf::from(common)
            .canonicalize()
            .map_err(DiscoveryError::Io)?,
    })
}
