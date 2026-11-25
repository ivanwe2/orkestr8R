//! `ork.json`: which model does each role, on which server, which commands may run, and the
//! default limits. Loaded from the built-in defaults, then the user file, then the repo file, each
//! overriding the one before.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::env::Env;

/// The whole config. `#[serde(default)]` everywhere, so a file only needs the keys it changes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OrkConfig {
    pub servers: BTreeMap<String, ServerConfig>,
    pub roles: BTreeMap<String, RoleConfig>,
    pub allow_commands: Vec<String>,
    pub defaults: DefaultsConfig,
}

/// An OpenAI-compatible server. `url` ends in `/v1`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub url: String,
    /// The name of the environment variable that holds the API key. Keys never live in the file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_env: Option<String>,
}

/// A role (`grunt` in v0.1): which model on which server, and how to call it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RoleConfig {
    pub server: String,
    pub model: String,
    pub context_tokens: u32,
    pub temperature: f64,
    pub max_output_tokens: u32,
}

impl Default for RoleConfig {
    fn default() -> Self {
        Self {
            server: String::new(),
            model: String::new(),
            context_tokens: 32_768,
            temperature: 0.2,
            max_output_tokens: 8_192,
        }
    }
}

/// Limits for an order that doesn't set its own.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DefaultsConfig {
    pub max_turns: u32,
    pub max_repairs: u32,
    pub max_minutes: u32,
    pub tool_timeout_seconds: u64,
    pub check_timeout_seconds: u64,
    pub request_timeout_seconds: u64,
}

impl Default for DefaultsConfig {
    fn default() -> Self {
        Self {
            max_turns: 40,
            max_repairs: 2,
            max_minutes: 20,
            tool_timeout_seconds: 300,
            check_timeout_seconds: 600,
            request_timeout_seconds: 600,
        }
    }
}

/// The merged config, and the files it came from (in the order they were applied).
#[derive(Debug, Clone)]
pub struct LoadedConfig {
    pub config: OrkConfig,
    pub sources: Vec<PathBuf>,
    /// Things people should know (commands emit them as `warning` events), e.g. an API key
    /// ignored because the repo's file changed its server.
    pub warnings: Vec<String>,
}

/// Loads the config for a command run in `cwd`.
///
/// Files that don't exist are skipped. `ORK_CONFIG`, when set, replaces the user and repo files
/// (and must exist). Bad JSON is an error; bad values are not (see [`OrkConfig::validate`]).
pub fn load(cwd: &Path, env: &Env) -> anyhow::Result<LoadedConfig> {
    // (file, trusted). The user file and ORK_CONFIG are the user's own choice; the repo's file
    // comes with whatever was cloned.
    let files: Vec<(PathBuf, bool)> = match env.get("ORK_CONFIG") {
        Some(path) => {
            let path = cwd.join(path);
            if !path.is_file() {
                bail!(
                    "ORK_CONFIG points to {}, which doesn't exist",
                    path.display()
                );
            }
            vec![(path, true)]
        }
        None => {
            let user = user_path(env).filter(|path| path.is_file());
            let repo = repo_root(cwd)
                .map(|root| root.join("ork.json"))
                .filter(|path| path.is_file());
            let user = user.map(|path| (path, true));
            user.into_iter()
                .chain(repo.map(|path| (path, false)))
                .collect()
        }
    };

    // Merge as JSON first, deserialize once: a key the user file sets survives a repo file that
    // doesn't mention it, and the defaults fill whatever nobody set.
    let mut merged = Value::Object(Default::default());
    let mut trusted = Value::Object(Default::default());
    for (file, is_trusted) in &files {
        let text =
            std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
        let value: Value = serde_json::from_str(&text)
            .with_context(|| format!("{} is not valid JSON", file.display()))?;
        if *is_trusted {
            merge(&mut trusted, value.clone());
        }
        merge(&mut merged, value);
    }
    let mut config: OrkConfig =
        serde_json::from_value(merged).context("the config has a value of the wrong type")?;
    let trusted: OrkConfig = serde_json::from_value(trusted).unwrap_or_default();
    let warnings = drop_untrusted_keys(&mut config, &trusted);

    Ok(LoadedConfig {
        config,
        sources: files.into_iter().map(|(path, _)| path).collect(),
        warnings,
    })
}

/// An API key is only sent to a server whose `url` and `key_env` the user's own file (or
/// `ORK_CONFIG`) sets exactly. Otherwise a cloned repo's `ork.json` could point a server at its
/// own host, or name another variable (`AWS_SECRET_ACCESS_KEY`) as the key, and `ork` would send
/// it as a Bearer token. Returns a warning per key dropped.
fn drop_untrusted_keys(config: &mut OrkConfig, trusted: &OrkConfig) -> Vec<String> {
    let mut warnings = Vec::new();
    for (name, server) in &mut config.servers {
        if server.key_env.is_none() {
            continue;
        }
        let as_trusted = trusted
            .servers
            .get(name)
            .is_some_and(|own| own.url == server.url && own.key_env == server.key_env);
        if !as_trusted {
            server.key_env = None;
            warnings.push(format!(
                "servers.{name}.key_env: ignored, because the repo's ork.json changes this server; \
                 API keys are only sent to servers your user file defines (ork init --user)"
            ));
        }
    }
    warnings
}

/// Deep-merges `overlay` into `base`: objects merge key by key, anything else (arrays included)
/// is replaced.
pub fn merge(base: &mut Value, overlay: Value) {
    match (base, overlay) {
        (Value::Object(base), Value::Object(overlay)) => {
            for (key, value) in overlay {
                match base.get_mut(&key) {
                    Some(existing) => merge(existing, value),
                    None => {
                        base.insert(key, value);
                    }
                }
            }
        }
        (base, overlay) => *base = overlay,
    }
}

/// The user's file: `%APPDATA%\orkestr8r\ork.json` on Windows, else
/// `$XDG_CONFIG_HOME/orkestr8r/ork.json` or `$HOME/.config/orkestr8r/ork.json`.
pub fn user_path(env: &Env) -> Option<PathBuf> {
    let dir = if cfg!(windows) {
        PathBuf::from(env.get("APPDATA")?)
    } else if let Some(xdg) = env.get("XDG_CONFIG_HOME").filter(|dir| !dir.is_empty()) {
        PathBuf::from(xdg)
    } else {
        PathBuf::from(env.get("HOME")?).join(".config")
    };
    Some(dir.join("orkestr8r").join("ork.json"))
}

/// The nearest directory, from `cwd` up, with a `.git` entry (a directory, or a file in a
/// worktree).
pub fn repo_root(cwd: &Path) -> Option<PathBuf> {
    cwd.ancestors()
        .find(|dir| dir.join(".git").exists())
        .map(Path::to_path_buf)
}

impl OrkConfig {
    /// Every problem, each starting with its JSON path. Empty when the config is usable.
    pub fn validate(&self) -> Vec<String> {
        let mut problems = Vec::new();

        if !self.roles.contains_key("grunt") {
            problems.push(
                "roles.grunt: missing; it says which model does the work (run `ork init`)".into(),
            );
        }
        for (name, role) in &self.roles {
            let path = format!("roles.{name}");
            if role.server.is_empty() {
                problems.push(format!("{path}.server: missing"));
            } else if !self.servers.contains_key(&role.server) {
                problems.push(format!(
                    "{path}.server: \"{}\" is not in servers",
                    role.server
                ));
            }
            if role.model.is_empty() {
                problems.push(format!("{path}.model: missing"));
            }
            positive(
                &mut problems,
                &format!("{path}.context_tokens"),
                role.context_tokens.into(),
            );
            positive(
                &mut problems,
                &format!("{path}.max_output_tokens"),
                role.max_output_tokens.into(),
            );
            if role.temperature.is_nan() || role.temperature < 0.0 {
                problems.push(format!("{path}.temperature: must be 0 or more"));
            }
        }
        for (name, server) in &self.servers {
            if !is_http_url(&server.url) {
                problems.push(format!(
                    "servers.{name}.url: \"{}\" is not an http:// or https:// URL",
                    server.url
                ));
            }
        }

        let d = &self.defaults;
        positive(&mut problems, "defaults.max_turns", d.max_turns.into());
        positive(&mut problems, "defaults.max_minutes", d.max_minutes.into());
        positive(
            &mut problems,
            "defaults.tool_timeout_seconds",
            d.tool_timeout_seconds,
        );
        positive(
            &mut problems,
            "defaults.check_timeout_seconds",
            d.check_timeout_seconds,
        );
        positive(
            &mut problems,
            "defaults.request_timeout_seconds",
            d.request_timeout_seconds,
        );
        // max_repairs may be 0: no repair turns.

        problems
    }
}

/// The API key for `server`: the value of its `key_env` variable, when both are set.
pub fn api_key(server: &ServerConfig, env: &Env) -> Option<String> {
    let name = server.key_env.as_deref()?;
    env.get(name)
        .filter(|key| !key.is_empty())
        .map(String::from)
}

fn positive(problems: &mut Vec<String>, path: &str, value: u64) {
    if value == 0 {
        problems.push(format!("{path}: must be more than 0"));
    }
}

fn is_http_url(url: &str) -> bool {
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"));
    rest.is_some_and(|rest| !rest.is_empty() && !rest.starts_with('/'))
}
