//! Git, through the `git` command. T08 turns this into the `Git` struct; for now `ork init` only
//! needs [`add_exclude`].

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, bail};

/// Adds `pattern` to `<git common dir>/info/exclude` unless a line already says it, so `.ork/`
/// stays out of `git status` without touching the user's `.gitignore`. Returns whether it added
/// the line.
pub fn add_exclude(root: &Path, pattern: &str) -> anyhow::Result<bool> {
    let exclude = common_dir(root)?.join("info").join("exclude");
    let existing = std::fs::read_to_string(&exclude).unwrap_or_default();
    if existing.lines().any(|line| line.trim() == pattern) {
        return Ok(false);
    }

    let mut text = existing;
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(pattern);
    text.push('\n');
    if let Some(dir) = exclude.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    std::fs::write(&exclude, text).with_context(|| format!("writing {}", exclude.display()))?;
    Ok(true)
}

/// The shared `.git` directory (the same one for every worktree of the repo).
fn common_dir(root: &Path) -> anyhow::Result<PathBuf> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "--git-common-dir"])
        .output()
        .context("running git (is it installed?)")?;
    if !output.status.success() {
        bail!(
            "git rev-parse --git-common-dir failed in {}: {}",
            root.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let dir = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    // git prints it relative to the directory it ran in when it can.
    Ok(if dir.is_absolute() {
        dir
    } else {
        root.join(dir)
    })
}
