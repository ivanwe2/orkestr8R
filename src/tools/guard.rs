//! The path guard: every path the grunt names is resolved here, and refused if it leads outside
//! its worktree, into `.git`, or (for writes) outside the order's `write_scope`.

use std::path::{Component, Path, PathBuf};

use anyhow::Context;
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};

#[derive(Debug, Clone)]
pub struct PathGuard {
    /// The worktree, canonical.
    root: PathBuf,
    scope: GlobSet,
    scope_patterns: Vec<String>,
}

impl PathGuard {
    /// `root` is the worktree; `write_scope` holds globs relative to it (`src/**`, `tests/*.rs`).
    pub fn new(root: &Path, write_scope: &[String]) -> anyhow::Result<Self> {
        let root = dunce::canonicalize(root)
            .with_context(|| format!("{} doesn't exist", root.display()))?;
        let mut builder = GlobSetBuilder::new();
        for pattern in write_scope {
            // literal_separator: `*` stays within one directory, as in a shell; `**` crosses them.
            let glob = GlobBuilder::new(pattern)
                .literal_separator(true)
                .case_insensitive(cfg!(windows))
                .build()
                .with_context(|| format!("write_scope: {pattern} is not a valid glob"))?;
            builder.add(glob);
        }
        Ok(Self {
            root,
            scope: builder.build()?,
            scope_patterns: write_scope.to_vec(),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The real full path for `rel`, or why it's refused (written for the model to read).
    pub fn resolve(&self, rel: &str, write: bool) -> Result<PathBuf, String> {
        let shown = if rel.is_empty() { "." } else { rel };

        // 1. Lexically: `..` and `.` resolved without touching the disk. An absolute `rel` replaces
        //    the root in `join`, and then fails the same check.
        let path = normalize(&self.root.join(rel));
        let Some(lexical) = relative_to(&path, &self.root) else {
            return Err(format!("refused: {shown} is outside the repository"));
        };
        if cfg!(windows)
            && lexical
                .components()
                .any(|c| is_windows_alias(&c.as_os_str().to_string_lossy()))
        {
            return Err(format!(
                "refused: {shown}: `:` and names ending in `.` or a space aren't allowed (on Windows \
                 they name other files)"
            ));
        }
        if has_git_segment(&lexical) {
            return Err(format!(
                "refused: {shown} is inside .git, which is off limits"
            ));
        }

        // 2. On disk: a link anywhere on the path decides where it really leads. Canonicalize the
        //    deepest part that exists (by symlink_metadata, so a dangling link counts as existing),
        //    put the rest back on, and judge that real path: inside the root, not in .git, and (for
        //    writes) in scope. `src/hooks -> ../.git/hooks` is refused here, though it looks fine.
        let mut existing = path.clone();
        let mut rest = Vec::new();
        while std::fs::symlink_metadata(&existing).is_err() {
            let Some(name) = existing.file_name() else {
                break;
            };
            rest.push(name.to_owned());
            existing.pop();
        }
        let Ok(mut real) = dunce::canonicalize(&existing) else {
            return Err(format!(
                "refused: {shown} goes through a link that leads nowhere"
            ));
        };
        real.extend(rest.iter().rev());
        let Some(inside) = relative_to(&real, &self.root) else {
            return Err(format!(
                "refused: {shown} leads outside the repository through a link"
            ));
        };
        if has_git_segment(&inside) {
            return Err(format!("refused: {shown} leads into .git through a link"));
        }

        // 3. Writes must also be in scope.
        if write {
            let rel_slash = to_slash(&inside);
            if self.scope_patterns.is_empty() {
                return Err(format!(
                    "refused: {shown}: this order may not change any files"
                ));
            }
            if !self.scope.is_match(&rel_slash) {
                return Err(format!(
                    "refused: {rel_slash} is outside the files you may change ({})",
                    self.scope_patterns.join(", ")
                ));
            }
        }
        Ok(real)
    }

    /// `path` relative to the root, with `/` (for messages and globs).
    pub fn display(&self, path: &Path) -> String {
        relative_to(path, &self.root)
            .map(|rel| to_slash(&rel))
            .unwrap_or_else(|| path.display().to_string())
    }
}

fn has_git_segment(path: &Path) -> bool {
    path.components()
        .any(|c| same(&c.as_os_str().to_string_lossy(), ".git"))
}

/// A Windows name that points at another file: `a.txt:stream` (an alternate data stream), or a
/// trailing `.` or space, which Windows drops (`.git.` is `.git`).
fn is_windows_alias(part: &str) -> bool {
    part.contains(':') || part.ends_with('.') || part.ends_with(' ')
}

/// `.` and `..` resolved without touching the disk.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// `path` relative to `root` when it's the root or under it, comparing component by component
/// (case-insensitively on Windows).
fn relative_to(path: &Path, root: &Path) -> Option<PathBuf> {
    let mut path_parts = path.components();
    for root_part in root.components() {
        let part = path_parts.next()?;
        if !same(
            &part.as_os_str().to_string_lossy(),
            &root_part.as_os_str().to_string_lossy(),
        ) {
            return None;
        }
    }
    Some(path_parts.collect())
}

fn same(a: &str, b: &str) -> bool {
    if cfg!(windows) {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

fn to_slash(path: &Path) -> String {
    let parts: Vec<String> = path
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    if parts.is_empty() {
        ".".to_string()
    } else {
        parts.join("/")
    }
}
