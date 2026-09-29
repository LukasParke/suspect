//! Baselines: compare against what was published, not against a file that
//! happens to be checked out.
//!
//! `--baseline <git-ref>` resolves a previous revision of the same path out
//! of git and diffs against that. At scale this is the difference between
//! "compare against whatever I have" and "compare against the last
//! release", which is what a service actually needs to know.

use std::path::Path;
use std::process::Command;

/// A baseline resolution failure.
#[derive(Debug)]
pub enum BaselineError {
    /// git is not available.
    GitUnavailable(String),
    /// The path did not exist at the baseline ref.
    MissingAtRef {
        /// The git ref.
        git_ref: String,
        /// The path within the repository.
        path: String,
    },
    /// git failed.
    GitFailed {
        /// The git ref.
        git_ref: String,
        /// git's stderr.
        message: String,
    },
}

impl std::fmt::Display for BaselineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::GitUnavailable(message) => {
                write!(f, "baseline resolution needs git: {message}")
            }
            Self::MissingAtRef { git_ref, path } => {
                write!(f, "{path} did not exist at `{git_ref}`")
            }
            Self::GitFailed { git_ref, message } => {
                write!(f, "git show failed for `{git_ref}`: {message}")
            }
        }
    }
}

impl std::error::Error for BaselineError {}

/// The bytes of `path` as of `git_ref`, materialized into a temporary file
/// so the existing file-based comparisons run unchanged.
///
/// # Errors
/// git being unavailable, the path missing at that ref, or git failing.
pub fn materialize(git_ref: &str, path: &Path) -> Result<std::path::PathBuf, BaselineError> {
    let absolute = path
        .canonicalize()
        .map_err(|_| BaselineError::MissingAtRef {
            git_ref: git_ref.to_owned(),
            path: path.display().to_string(),
        })?;
    let repository = absolute.parent().and_then(repo_root).ok_or_else(|| {
        BaselineError::GitUnavailable(format!(
            "{} is not inside a git repository",
            absolute.display()
        ))
    })?;
    let relative = absolute
        .strip_prefix(&repository)
        .map_err(|_| BaselineError::MissingAtRef {
            git_ref: git_ref.to_owned(),
            path: absolute.display().to_string(),
        })?
        .to_string_lossy()
        .into_owned();

    let output = Command::new("git")
        .arg("-C")
        .arg(&repository)
        .args(["show", &format!("{git_ref}:{relative}")])
        .output()
        .map_err(|e| BaselineError::GitUnavailable(e.to_string()))?;
    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(if message.contains("does not exist") {
            BaselineError::MissingAtRef {
                git_ref: git_ref.to_owned(),
                path: relative,
            }
        } else {
            BaselineError::GitFailed {
                git_ref: git_ref.to_owned(),
                message,
            }
        });
    }

    // A temporary file beside the working copy keeps relative `$ref`
    // resolution working: the baseline document resolves its references in
    // the same directory as the current one.
    let staged = absolute.with_extension(format!(
        "{}.suspect-baseline",
        absolute
            .extension()
            .map(|e| e.to_string_lossy().into_owned())
            .unwrap_or_else(|| "tmp".to_owned())
    ));
    std::fs::write(&staged, &output.stdout).map_err(|e| BaselineError::GitFailed {
        git_ref: git_ref.to_owned(),
        message: format!("{}: {e}", staged.display()),
    })?;
    Ok(staged)
}

/// The repository root containing `dir`, if any.
fn repo_root(dir: &Path) -> Option<std::path::PathBuf> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let root = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if root.is_empty() {
        return None;
    }
    Some(std::path::PathBuf::from(root))
}

/// Removes a staged baseline file, ignoring failures.
pub fn cleanup(staged: &Path) {
    let _ = std::fs::remove_file(staged);
}
