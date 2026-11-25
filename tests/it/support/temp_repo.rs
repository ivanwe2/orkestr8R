//! A temporary git repository: `git init`, an identity, `core.autocrlf false` (so files are byte
//! for byte what the test wrote, on every OS), and helpers to write and commit.

use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

pub struct TempRepo {
    pub dir: TempDir,
    /// The repository (a `repo/` directory inside `dir`, so tests have room beside it).
    pub path: PathBuf,
}

impl TempRepo {
    pub fn new() -> Self {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("repo");
        std::fs::create_dir_all(&path).unwrap();
        git(&path, &["init", "-q"]);
        git(&path, &["config", "core.autocrlf", "false"]);
        git(&path, &["config", "user.name", "test"]);
        git(&path, &["config", "user.email", "test@localhost"]);
        Self { dir, path }
    }

    /// A repo with one commit holding `files`.
    pub fn with_files(files: &[(&str, &str)]) -> Self {
        let repo = Self::new();
        for (rel, content) in files {
            repo.write(rel, content);
        }
        repo.commit("initial");
        repo
    }

    /// Writes a file in the repo (creating directories).
    pub fn write(&self, rel: &str, content: &str) -> PathBuf {
        let path = self.path.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, content).unwrap();
        path
    }

    pub fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.path.join(rel)).unwrap()
    }

    /// `git add -A && git commit -m message`.
    pub fn commit(&self, message: &str) {
        git(&self.path, &["add", "-A"]);
        git(&self.path, &["commit", "-q", "-m", message]);
    }

    pub fn git(&self, args: &[&str]) -> String {
        git(&self.path, args)
    }
}

/// `git ARGS` in `dir`; panics on failure, returns stdout.
pub fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}
