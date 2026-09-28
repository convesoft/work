//! Read-only selection of a Git working checkout.

use std::ffi::OsString;
use std::fmt;
use std::os::unix::ffi::OsStringExt;
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

    if git_query(&path, &["--is-inside-work-tree"])? != b"true" {
        return Err(DiscoveryError::UnsupportedProject(path));
    }
    // Query each path separately: Git terminates each answer with a newline,
    // while a valid Unix path may itself contain newlines or non-UTF-8 bytes.
    let root = git_query(&path, &["--path-format=absolute", "--show-toplevel"])?;
    let common = git_query(&path, &["--path-format=absolute", "--git-common-dir"])?;
    if root.is_empty() || common.is_empty() {
        return Err(DiscoveryError::UnsupportedProject(path));
    }
    Ok(Project {
        worktree_root: PathBuf::from(OsString::from_vec(root))
            .canonicalize()
            .map_err(DiscoveryError::Io)?,
        git_common_dir: PathBuf::from(OsString::from_vec(common))
            .canonicalize()
            .map_err(DiscoveryError::Io)?,
    })
}

fn git_query(path: &Path, args: &[&str]) -> Result<Vec<u8>, DiscoveryError> {
    // These repository-local overrides are listed by `git rev-parse
    // --local-env-vars`. They belong to the caller's Git context, not to the
    // explicitly selected checkout. The ceiling is a separate discovery limit.
    const REPOSITORY_ENV: &[&str] = &[
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_CONFIG",
        "GIT_CONFIG_PARAMETERS",
        "GIT_CONFIG_COUNT",
        "GIT_OBJECT_DIRECTORY",
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_IMPLICIT_WORK_TREE",
        "GIT_GRAFT_FILE",
        "GIT_INDEX_FILE",
        "GIT_NO_REPLACE_OBJECTS",
        "GIT_REPLACE_REF_BASE",
        "GIT_PREFIX",
        "GIT_SHALLOW_FILE",
        "GIT_COMMON_DIR",
        "GIT_CEILING_DIRECTORIES",
    ];
    let mut command = Command::new("git");
    for name in REPOSITORY_ENV {
        command.env_remove(name);
    }
    let output = command
        .arg("-C")
        .arg(path)
        .arg("rev-parse")
        .args(args)
        .output()
        .map_err(DiscoveryError::GitUnavailable)?;
    if !output.status.success() {
        return Err(DiscoveryError::UnsupportedProject(path.to_path_buf()));
    }
    let mut bytes = output.stdout;
    if bytes.pop() != Some(b'\n') {
        return Err(DiscoveryError::UnsupportedProject(path.to_path_buf()));
    }
    Ok(bytes)
}
