//! The Agent Skill, built into `ork`. `ork skill install` writes it where a coding agent looks for
//! skills, so the agent learns when to delegate without anyone copying files. The text is
//! `skills/orkestr8r/SKILL.md`, compiled in: the skill always matches the `ork` that runs it.
//!
//! Where each agent looks (each reads `<dir>/orkestr8r/SKILL.md`):
//!
//! | Agent | For you | For one repo |
//! |---|---|---|
//! | Claude Code | `~/.claude/skills` (`$CLAUDE_CONFIG_DIR/skills`) | `.claude/skills` |
//! | OpenCode | `~/.config/opencode/skills` (`$XDG_CONFIG_HOME/opencode/skills`) | `.opencode/skills` |
//! | Codex | `~/.agents/skills` | `.agents/skills` |
//! | Gemini CLI | `~/.gemini/skills` | `.gemini/skills` |
//! | any other | `~/.agents/skills`, the shared location | `.agents/skills` |

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};

use crate::env::Env;

pub const NAME: &str = "orkestr8r";

/// The skill's text.
pub const SKILL: &str = include_str!("../skills/orkestr8r/SKILL.md");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Agent {
    Claude,
    OpenCode,
    Codex,
    Gemini,
    /// Any agent that reads the shared `.agents/skills`.
    Other,
}

impl Agent {
    pub const ALL: [Agent; 5] = [
        Agent::Claude,
        Agent::OpenCode,
        Agent::Codex,
        Agent::Gemini,
        Agent::Other,
    ];

    /// What `ork skill install` takes.
    pub fn id(self) -> &'static str {
        match self {
            Agent::Claude => "claude",
            Agent::OpenCode => "opencode",
            Agent::Codex => "codex",
            Agent::Gemini => "gemini",
            Agent::Other => "other",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Agent::Claude => "Claude Code",
            Agent::OpenCode => "OpenCode",
            Agent::Codex => "Codex",
            Agent::Gemini => "Gemini CLI",
            Agent::Other => "Other agents",
        }
    }

    pub fn from_id(id: &str) -> Option<Agent> {
        Agent::ALL
            .into_iter()
            .find(|agent| agent.id().eq_ignore_ascii_case(id))
    }

    /// The agent's program, looked for on PATH.
    fn program(self) -> Option<&'static str> {
        match self {
            Agent::Claude => Some("claude"),
            Agent::OpenCode => Some("opencode"),
            Agent::Codex => Some("codex"),
            Agent::Gemini => Some("gemini"),
            Agent::Other => None,
        }
    }
}

/// For you (every repo), or for one repo (commit it, and everyone's agent gets it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    User,
    Repo,
}

/// What's on disk where an agent would read the skill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Installed {
    No,
    /// The same text as this `ork`'s.
    Current,
    /// Another text: an older `ork`'s, or edited.
    Outdated,
}

impl Installed {
    pub fn as_str(self) -> &'static str {
        match self {
            Installed::No => "missing",
            Installed::Current => "current",
            Installed::Outdated => "outdated",
        }
    }
}

/// One agent, here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub agent: Agent,
    /// The `SKILL.md` it reads.
    pub path: PathBuf,
    /// Its program is on PATH, or its config directory exists.
    pub found: bool,
    pub installed: Installed,
}

/// What an install did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Installed,
    Updated,
    /// It was there already, the same.
    Unchanged,
}

impl Change {
    pub fn as_str(self) -> &'static str {
        match self {
            Change::Installed => "installed",
            Change::Updated => "updated",
            Change::Unchanged => "unchanged",
        }
    }
}

/// Every agent: where its skill goes in `scope`, whether it's on this machine, whether the skill
/// is there. `repo` is the repo root, needed for [`Scope::Repo`].
pub fn targets(
    env: &Env,
    cwd: &Path,
    scope: Scope,
    repo: Option<&Path>,
) -> anyhow::Result<Vec<Target>> {
    let home = home(env);
    Agent::ALL
        .into_iter()
        .map(|agent| {
            let dir = match scope {
                Scope::User => user_dir(agent, env, home.as_deref())
                    .context("can't find your home directory (USERPROFILE or HOME isn't set)")?,
                Scope::Repo => repo
                    .context("not inside a git repository")?
                    .join(repo_dir(agent)),
            };
            let path = dir.join(NAME).join("SKILL.md");
            Ok(Target {
                agent,
                found: found(agent, env, cwd, home.as_deref()),
                installed: installed(&path),
                path,
            })
        })
        .collect()
}

/// Writes the skill to `path`. `repo` is the repo root when `path` is in one ([`Scope::Repo`]).
pub fn install(path: &Path, repo: Option<&Path>) -> anyhow::Result<Change> {
    if let Some(root) = repo {
        no_links_below(root, path)?;
    }
    let change = match installed(path) {
        Installed::Current => return Ok(Change::Unchanged),
        Installed::Outdated => Change::Updated,
        Installed::No => Change::Installed,
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    std::fs::write(path, SKILL).with_context(|| format!("writing {}", path.display()))?;
    Ok(change)
}

/// A repo's folders come with whatever was cloned, and a cloned `.claude/skills` (or the
/// `SKILL.md` in it) could be a link into your home: writing through it would overwrite one of
/// your files. So nothing below the root may be a link. Your own folders may be (dotfiles).
fn no_links_below(root: &Path, path: &Path) -> anyhow::Result<()> {
    let rel = path
        .strip_prefix(root)
        .with_context(|| format!("{} is outside {}", path.display(), root.display()))?;
    let mut at = root.to_path_buf();
    for part in rel.components() {
        at.push(part);
        match std::fs::symlink_metadata(&at) {
            Ok(meta) if meta.file_type().is_symlink() => {
                bail!("{} is a link; not writing through it", at.display())
            }
            Ok(_) => {}
            // The rest doesn't exist yet: install creates it.
            Err(_) => break,
        }
    }
    Ok(())
}

/// Line endings aside: git may have checked the file out with CRLF.
fn installed(path: &Path) -> Installed {
    match std::fs::read_to_string(path) {
        Ok(text) if text.replace("\r\n", "\n") == SKILL.replace("\r\n", "\n") => Installed::Current,
        Ok(_) => Installed::Outdated,
        Err(_) if path.exists() => Installed::Outdated,
        Err(_) => Installed::No,
    }
}

fn home(env: &Env) -> Option<PathBuf> {
    let name = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    env.get(name)
        .or_else(|| env.get("HOME"))
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
}

fn set(env: &Env, name: &str) -> Option<PathBuf> {
    env.get(name)
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
}

/// The agent's own directory: there means it has run on this machine.
fn config_dir(agent: Agent, env: &Env, home: Option<&Path>) -> Option<PathBuf> {
    match agent {
        Agent::Claude => set(env, "CLAUDE_CONFIG_DIR").or_else(|| Some(home?.join(".claude"))),
        Agent::OpenCode => set(env, "XDG_CONFIG_HOME")
            .or_else(|| Some(home?.join(".config")))
            .map(|dir| dir.join("opencode")),
        Agent::Codex => set(env, "CODEX_HOME").or_else(|| Some(home?.join(".codex"))),
        Agent::Gemini => Some(home?.join(".gemini")),
        Agent::Other => None,
    }
}

fn user_dir(agent: Agent, env: &Env, home: Option<&Path>) -> Option<PathBuf> {
    match agent {
        Agent::Claude | Agent::OpenCode | Agent::Gemini => {
            Some(config_dir(agent, env, home)?.join("skills"))
        }
        Agent::Codex | Agent::Other => Some(home?.join(".agents").join("skills")),
    }
}

fn repo_dir(agent: Agent) -> &'static str {
    match agent {
        Agent::Claude => ".claude/skills",
        Agent::OpenCode => ".opencode/skills",
        Agent::Codex | Agent::Other => ".agents/skills",
        Agent::Gemini => ".gemini/skills",
    }
}

fn found(agent: Agent, env: &Env, cwd: &Path, home: Option<&Path>) -> bool {
    config_dir(agent, env, home).is_some_and(|dir| dir.is_dir())
        || agent
            .program()
            .and_then(|program| crate::process::find::find(program, env, cwd))
            .is_some()
}
