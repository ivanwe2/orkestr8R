//! Secret files: what the grunt's tools never read, write or show (`.env`, private keys, package
//! registry tokens). The threat is a secret leaving the machine (a cloud model reads the repo too)
//! or landing in a transcript. So the list only grows from a repo's `ork.json`; exceptions
//! (`secrets.allow`) count only from your own config.
//!
//! It guards the tools, not commands: `run` and the checks execute programs, and a program can read
//! anything. That needs a sandbox (#20).

use anyhow::Context;
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};

/// Always secret. A pattern without `/` matches the file name at any depth.
pub const BUILT_IN: &[&str] = &[
    ".env",
    ".env.*",
    "*.pem",
    "*.key",
    "*.p12",
    "*.pfx",
    "id_rsa",
    "id_dsa",
    "id_ecdsa",
    "id_ed25519",
    ".npmrc",
    ".pypirc",
    ".netrc",
    ".pgpass",
    ".git-credentials",
    "*.tfvars",
];

/// Never secret: the templates that document a `.env` without holding one.
pub const BUILT_IN_ALLOW: &[&str] = &[".env.example", ".env.sample", ".env.template"];

#[derive(Debug, Clone)]
pub struct Secrets {
    deny: GlobSet,
    allow: GlobSet,
}

impl Secrets {
    /// The built-in lists plus the config's `secrets.deny` and `secrets.allow`.
    pub fn new(deny: &[String], allow: &[String]) -> anyhow::Result<Self> {
        let deny = BUILT_IN
            .iter()
            .copied()
            .chain(deny.iter().map(String::as_str));
        let allow = BUILT_IN_ALLOW
            .iter()
            .copied()
            .chain(allow.iter().map(String::as_str));
        Ok(Self {
            deny: set(deny).context("secrets.deny")?,
            allow: set(allow).context("secrets.allow")?,
        })
    }

    /// Just the built-in lists.
    pub fn built_in() -> Self {
        Self::new(&[], &[]).expect("the built-in patterns are valid")
    }

    /// Whether `rel` (a path relative to the repo root, with `/`) is a secret file.
    pub fn is_secret(&self, rel: &str) -> bool {
        let rel = rel.trim_start_matches("./");
        self.deny.is_match(rel) && !self.allow.is_match(rel)
    }
}

/// Checks patterns before they're used (config validation).
pub fn check_pattern(pattern: &str) -> Result<(), String> {
    glob(pattern).map(|_| ()).map_err(|err| err.to_string())
}

fn set<'a>(patterns: impl Iterator<Item = &'a str>) -> anyhow::Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        builder.add(glob(pattern).with_context(|| format!("{pattern} is not a valid glob"))?);
    }
    Ok(builder.build()?)
}

fn glob(pattern: &str) -> Result<globset::Glob, globset::Error> {
    let pattern = if pattern.contains('/') {
        pattern.to_string()
    } else {
        format!("**/{pattern}")
    };
    GlobBuilder::new(&pattern)
        .literal_separator(true)
        .case_insensitive(cfg!(windows))
        .build()
}
