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
    let project = Project {
        worktree_root: PathBuf::from(OsString::from_vec(root))
            .canonicalize()
            .map_err(DiscoveryError::Io)?,
        git_common_dir: PathBuf::from(OsString::from_vec(common))
            .canonicalize()
            .map_err(DiscoveryError::Io)?,
    };
    let primary = inventory(&project)?
        .into_iter()
        .next()
        .ok_or_else(|| DiscoveryError::UnsupportedProject(path.clone()))?;
    // A primary Git directory is an actual .git directory beneath the working
    // checkout. Separate-Git-dir repositories are deliberately unsupported:
    // their inventory may name the administration directory, not a checkout.
    if primary.join(".git") != project.git_common_dir
        || std::fs::symlink_metadata(&project.git_common_dir).is_err()
        || std::fs::symlink_metadata(primary.join(".git"))
            .is_ok_and(|m| !m.is_dir() || m.file_type().is_symlink())
    {
        return Err(DiscoveryError::UnsupportedProject(path));
    }
    Ok(project)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkout {
    pub root: PathBuf,
    pub git_dir: Option<PathBuf>,
    pub available: bool,
}
impl Project {
    pub fn git_dir(&self) -> Result<PathBuf, DiscoveryError> {
        let raw = git_query(
            &self.worktree_root,
            &["--path-format=absolute", "--git-dir"],
        )?;
        PathBuf::from(OsString::from_vec(raw))
            .canonicalize()
            .map_err(DiscoveryError::Io)
    }
    pub fn checkouts(&self) -> Result<Vec<Checkout>, DiscoveryError> {
        let roots = inventory(self)?;
        let mut checkouts = Vec::new();
        for root in roots {
            let active = discover(Some(&root))
                .ok()
                .filter(|p| p.worktree_root == root && p.git_common_dir == self.git_common_dir)
                .and_then(|p| p.git_dir().ok());
            let available = active.is_some();
            let git_dir = active.or_else(|| self.retained_git_dir(&root));
            checkouts.push(Checkout {
                root,
                git_dir,
                available,
            });
        }
        Ok(checkouts)
    }
    fn retained_git_dir(&self, root: &Path) -> Option<PathBuf> {
        if self.git_common_dir.parent() == Some(root) {
            return Some(self.git_common_dir.clone());
        }
        for entry in std::fs::read_dir(self.git_common_dir.join("worktrees")).ok()? {
            let Ok(entry) = entry else { continue };
            let metadata = entry.path();
            let Ok(mut raw) = std::fs::read(metadata.join("gitdir")) else {
                continue;
            };
            if raw.pop() != Some(b'\n') {
                continue;
            }
            let pointer = PathBuf::from(OsString::from_vec(raw));
            let pointer = if pointer.is_absolute() {
                pointer
            } else {
                metadata.join(pointer)
            };
            let mut normalized = PathBuf::new();
            for component in pointer.components() {
                match component {
                    std::path::Component::CurDir => {}
                    std::path::Component::ParentDir => {
                        normalized.pop();
                    }
                    c => normalized.push(c.as_os_str()),
                }
            }
            if normalized.parent() == Some(root) {
                return Some(metadata);
            }
        }
        None
    }
}
fn inventory(project: &Project) -> Result<Vec<PathBuf>, DiscoveryError> {
    let output = git_command(&project.worktree_root)
        .args(["worktree", "list", "--porcelain", "-z"])
        .output()
        .map_err(DiscoveryError::GitUnavailable)?;
    if !output.status.success() {
        return Err(DiscoveryError::UnsupportedProject(
            project.worktree_root.clone(),
        ));
    }
    let roots: Vec<_> = output
        .stdout
        .split(|b| *b == 0)
        .filter_map(|part| part.strip_prefix(b"worktree "))
        .map(|path| PathBuf::from(OsString::from_vec(path.to_vec())))
        .collect();
    if roots.is_empty() {
        return Err(DiscoveryError::UnsupportedProject(
            project.worktree_root.clone(),
        ));
    }
    Ok(roots)
}

fn git_command(path: &Path) -> Command {
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
    command.arg("-C").arg(path);
    command
}
fn git_query(path: &Path, args: &[&str]) -> Result<Vec<u8>, DiscoveryError> {
    let output = git_command(path)
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
