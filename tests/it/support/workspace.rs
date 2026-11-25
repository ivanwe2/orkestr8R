//! A temp git repo plus an `ORK_CONFIG` file pointing at a (fake) server: the setting for
//! end-to-end command tests.

use std::path::PathBuf;

use orkestr8r::env::Env;
use serde_json::{Value, json};

use super::temp_repo::TempRepo;

pub struct Workspace {
    pub temp: TempRepo,
    /// The repo (the same as `temp.path`).
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
        let temp = TempRepo::new();
        let config_path = temp.dir.path().join("ork.json");
        std::fs::write(&config_path, serde_json::to_string_pretty(&config).unwrap()).unwrap();
        let env = Env::from_process().with("ORK_CONFIG", config_path.display().to_string());
        Self {
            repo: temp.path.clone(),
            temp,
            config_path,
            env,
        }
    }

    /// The temp directory around the repo.
    pub fn dir(&self) -> &std::path::Path {
        self.temp.dir.path()
    }

    /// Writes a file in the repo (creating directories).
    pub fn write(&self, rel: &str, content: &str) -> PathBuf {
        self.temp.write(rel, content)
    }

    /// `git add -A && git commit -m message` in the repo.
    pub fn commit(&self, message: &str) {
        self.temp.commit(message);
    }
}

/// The stdout of a `--json` command, one parsed event per line.
pub fn json_lines(stdout: &str) -> Vec<Value> {
    stdout
        .lines()
        .map(|line| serde_json::from_str(line).unwrap_or_else(|e| panic!("not JSON ({e}): {line}")))
        .collect()
}
