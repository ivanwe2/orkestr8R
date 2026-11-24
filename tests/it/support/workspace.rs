//! A temp git repo plus an `ORK_CONFIG` file pointing at a (fake) server: the setting for
//! end-to-end command tests.

use std::path::{Path, PathBuf};
use std::process::Command;

use orkestr8r::env::Env;
use serde_json::{Value, json};
use tempfile::TempDir;

pub struct Workspace {
    pub dir: TempDir,
    /// The repo, `git init`ed, with an identity and `core.autocrlf false`.
    pub repo: PathBuf,
    pub config_path: PathBuf,
    /// The real environment (so PATH finds git) with `ORK_CONFIG` set.
    pub env: Env,
}

impl Workspace {
    /// A repo and a config whose grunt is `fake-model` on `url`, allowing `allow`.
    pub fn new(url: &str, allow: &[&str]) -> Self {
        let config = json!({
            "servers": { "box": { "url": url } },
            "roles": { "grunt": { "server": "box", "model": "fake-model" } },
            "allow_commands": allow,
        });
        Self::with_config(config)
    }

    pub fn with_config(config: Value) -> Self {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q"]);
        git(&repo, &["config", "core.autocrlf", "false"]);
        git(&repo, &["config", "user.name", "test"]);
        git(&repo, &["config", "user.email", "test@localhost"]);

        let config_path = dir.path().join("ork.json");
        std::fs::write(&config_path, serde_json::to_string_pretty(&config).unwrap()).unwrap();
        let env = Env::from_process().with("ORK_CONFIG", config_path.display().to_string());
        Self {
            dir,
            repo,
            config_path,
            env,
        }
    }

    /// Writes a file in the repo (creating directories).
    pub fn write(&self, rel: &str, content: &str) -> PathBuf {
        let path = self.repo.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, content).unwrap();
        path
    }

    /// `git add -A && git commit -m message` in the repo.
    pub fn commit(&self, message: &str) {
        git(&self.repo, &["add", "-A"]);
        git(&self.repo, &["commit", "-q", "-m", message]);
    }
}

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

/// The stdout of a `--json` command, one parsed event per line.
pub fn json_lines(stdout: &str) -> Vec<Value> {
    stdout
        .lines()
        .map(|line| serde_json::from_str(line).unwrap_or_else(|e| panic!("not JSON ({e}): {line}")))
        .collect()
}
