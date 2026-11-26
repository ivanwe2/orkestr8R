//! Git, through the `git` command: worktrees for jobs, commits, diffs, merges.
//!
//! Every call runs `git -c user.name=orkestr8r -c user.email=orkestr8r@localhost -c
//! core.quotepath=false …`, so no global git config is needed and paths come back as plain text.
//! git runs with the same scrubbed environment as other commands, but with stdout and stderr
//! apart: its warnings on stderr must not end up in what we parse.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context, bail};

use crate::orders::FileChange;
use crate::process::{ProcessRunner, find};

#[derive(Debug, Clone)]
pub struct Git {
    runner: ProcessRunner,
}

impl Git {
    pub fn new(runner: ProcessRunner) -> Self {
        Self { runner }
    }

    /// `git ARGS` in `dir`; stdout on success, an error with the output otherwise.
    pub async fn run(&self, dir: &Path, args: &[&str]) -> anyhow::Result<String> {
        let output = self.output(dir, args).await?;
        if !output.status.success() {
            bail!(
                "git {} failed: {}",
                args.join(" "),
                combined(&output.stdout, &output.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    async fn output(&self, dir: &Path, args: &[&str]) -> anyhow::Result<std::process::Output> {
        let git = find("git", self.runner.env(), dir).context("git: not found on PATH")?;
        tokio::process::Command::new(git)
            .args([
                "-c",
                "user.name=orkestr8r",
                "-c",
                "user.email=orkestr8r@localhost",
                "-c",
                "core.quotepath=false",
            ])
            .args(args)
            .current_dir(dir)
            .env_clear()
            .envs(self.runner.child_env())
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output()
            .await
            .context("running git")
    }

    /// The top of the working tree that contains `cwd`.
    pub async fn repo_root(&self, cwd: &Path) -> anyhow::Result<PathBuf> {
        let out = self
            .run(cwd, &["rev-parse", "--show-toplevel"])
            .await
            .context("not inside a git repository")?;
        Ok(PathBuf::from(out.trim()))
    }

    pub async fn head_sha(&self, root: &Path) -> anyhow::Result<String> {
        Ok(self
            .run(root, &["rev-parse", "HEAD"])
            .await?
            .trim()
            .to_string())
    }

    /// The branch checked out in `root`, or `HEAD` when detached.
    pub async fn current_branch(&self, root: &Path) -> anyhow::Result<String> {
        Ok(self
            .run(root, &["rev-parse", "--abbrev-ref", "HEAD"])
            .await?
            .trim()
            .to_string())
    }

    /// No uncommitted changes, ignoring anything under `.ork/`.
    pub async fn is_clean(&self, root: &Path) -> anyhow::Result<bool> {
        let status = self.run(root, &["status", "--porcelain"]).await?;
        Ok(status.lines().all(|line| {
            let path = line.get(3..).unwrap_or_default().trim_matches('"');
            path.starts_with(".ork/") || path == ".ork"
        }))
    }

    /// Adds `pattern` to `<git common dir>/info/exclude` unless a line already says it, so `.ork/`
    /// stays out of `git status` without touching the user's `.gitignore`. Returns whether it
    /// added the line.
    pub async fn add_exclude(&self, root: &Path, pattern: &str) -> anyhow::Result<bool> {
        let common = PathBuf::from(
            self.run(root, &["rev-parse", "--git-common-dir"])
                .await?
                .trim(),
        );
        // git prints it relative to the directory it ran in when it can.
        let common = if common.is_absolute() {
            common
        } else {
            root.join(common)
        };
        let exclude = common.join("info").join("exclude");

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

    /// A new worktree at `path` on a new `branch`, starting at `base_sha`.
    pub async fn worktree_add(
        &self,
        root: &Path,
        path: &Path,
        branch: &str,
        base_sha: &str,
    ) -> anyhow::Result<()> {
        let path = path.to_string_lossy();
        self.run(
            root,
            &["worktree", "add", "-q", "-b", branch, &path, base_sha],
        )
        .await?;
        Ok(())
    }

    /// A worktree at `sha` with no branch: for a look at the base commit (and gone after).
    pub async fn worktree_add_detached(
        &self,
        root: &Path,
        path: &Path,
        sha: &str,
    ) -> anyhow::Result<()> {
        let path = path.to_string_lossy();
        self.run(root, &["worktree", "add", "-q", "--detach", &path, sha])
            .await?;
        Ok(())
    }

    /// Stages everything and commits it, without the repo's hooks (this is ork's own commit on a job
    /// branch, not the user's). The new commit's sha, or `None` when nothing changed.
    pub async fn commit_all(&self, dir: &Path, message: &str) -> anyhow::Result<Option<String>> {
        self.run(dir, &["add", "-A"]).await?;
        if self
            .run(dir, &["status", "--porcelain"])
            .await?
            .trim()
            .is_empty()
        {
            return Ok(None);
        }
        self.run(dir, &["commit", "-q", "--no-verify", "-m", message])
            .await?;
        Ok(Some(self.head_sha(dir).await?))
    }

    /// What changed from `base_sha` to `branch`, per file. Binary files count as 0/0.
    pub async fn num_stat(
        &self,
        root: &Path,
        base_sha: &str,
        branch: &str,
    ) -> anyhow::Result<Vec<FileChange>> {
        let range = format!("{base_sha}..{branch}");
        let out = self
            .run(
                root,
                &[
                    "diff",
                    "--numstat",
                    "--no-renames",
                    "--end-of-options",
                    &range,
                ],
            )
            .await?;
        Ok(out
            .lines()
            .filter_map(|line| {
                let mut parts = line.splitn(3, '\t');
                let added = parts.next()?.parse().unwrap_or(0);
                let removed = parts.next()?.parse().unwrap_or(0);
                let path = parts.next()?.to_string();
                Some(FileChange {
                    path,
                    added,
                    removed,
                })
            })
            .collect())
    }

    /// `git diff base..branch`, or its `--stat`.
    pub async fn diff(
        &self,
        root: &Path,
        base_sha: &str,
        branch: &str,
        stat: bool,
    ) -> anyhow::Result<String> {
        let range = format!("{base_sha}..{branch}");
        let mut args = vec!["diff"];
        if stat {
            args.push("--stat");
        }
        // Revisions after --end-of-options: nothing that looks like an option is taken as one.
        args.push("--end-of-options");
        args.push(&range);
        self.run(root, &args).await
    }

    /// Merges `branch` into the branch checked out in `root`: a `--no-ff` merge commit, or one
    /// squashed commit. On a conflict the merge is aborted (nothing changes) and it's an error.
    pub async fn merge(
        &self,
        root: &Path,
        branch: &str,
        squash: bool,
        message: &str,
    ) -> anyhow::Result<()> {
        if squash {
            let merged = self
                .output(root, &["merge", "--squash", "--end-of-options", branch])
                .await?;
            if !merged.status.success() {
                // A squash merge leaves no MERGE_HEAD, so `merge --abort` can't undo it.
                let _ = self.run(root, &["reset", "--merge"]).await;
                bail!(
                    "merge conflict: {}",
                    combined(&merged.stdout, &merged.stderr).trim()
                );
            }
            self.run(root, &["commit", "-q", "-m", message]).await?;
        } else {
            let merged = self
                .output(
                    root,
                    &[
                        "merge",
                        "--no-ff",
                        "--no-edit",
                        "-m",
                        message,
                        "--end-of-options",
                        branch,
                    ],
                )
                .await?;
            if !merged.status.success() {
                let _ = self.run(root, &["merge", "--abort"]).await;
                bail!(
                    "merge conflict: {}",
                    combined(&merged.stdout, &merged.stderr).trim()
                );
            }
        }
        Ok(())
    }

    pub async fn worktree_remove(&self, root: &Path, path: &Path) -> anyhow::Result<()> {
        self.run(
            root,
            &["worktree", "remove", "--force", &path.to_string_lossy()],
        )
        .await?;
        Ok(())
    }

    pub async fn branch_delete(
        &self,
        root: &Path,
        branch: &str,
        force: bool,
    ) -> anyhow::Result<()> {
        self.run(
            root,
            &[
                "branch",
                if force { "-D" } else { "-d" },
                "--end-of-options",
                branch,
            ],
        )
        .await?;
        Ok(())
    }

    /// Tracked and untracked (not ignored) files under `dir`, relative to it, with `/`.
    pub async fn ls_files(&self, dir: &Path) -> anyhow::Result<Vec<String>> {
        let out = self
            .run(
                dir,
                &["ls-files", "--cached", "--others", "--exclude-standard"],
            )
            .await?;
        Ok(out.lines().map(String::from).collect())
    }
}

fn combined(stdout: &[u8], stderr: &[u8]) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(stdout),
        String::from_utf8_lossy(stderr)
    )
}
