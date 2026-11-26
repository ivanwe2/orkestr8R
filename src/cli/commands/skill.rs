//! `ork skill install AGENT…` and `ork skill list`: the Agent Skill, put where each agent reads
//! it (see `crate::skill` for the places).

use anyhow::bail;
use clap::builder::PossibleValuesParser;
use clap::{Args, Subcommand};
use serde_json::json;

use crate::cli::{CliContext, CommandResult};
use crate::config;
use crate::skill::{self, Agent, Scope, Target};

#[derive(Debug, Args)]
pub struct SkillArgs {
    #[command(subcommand)]
    pub command: SkillCommand,
}

#[derive(Debug, Subcommand)]
pub enum SkillCommand {
    /// Install the skill (or update it) for the agents you use
    Install(InstallArgs),
    /// Each agent: is it on this machine, is the skill there and up to date, where
    List(ListArgs),
}

#[derive(Debug, Args)]
pub struct InstallArgs {
    /// The agents: claude, opencode, codex, gemini, other (any agent that reads .agents/skills),
    /// or all
    #[arg(value_parser = PossibleValuesParser::new(["claude", "opencode", "codex", "gemini", "other", "all"]))]
    pub agents: Vec<String>,
    /// Into this repo (to commit, for everyone's agent) instead of your own folders
    #[arg(long)]
    pub repo: bool,
}

#[derive(Debug, Args)]
pub struct ListArgs {
    /// Where it would go in this repo, instead of your own folders
    #[arg(long)]
    pub repo: bool,
}

pub async fn run(ctx: &CliContext, args: SkillArgs) -> anyhow::Result<CommandResult> {
    match args.command {
        SkillCommand::Install(args) => install(ctx, args),
        SkillCommand::List(args) => list(ctx, args),
    }
}

fn targets(ctx: &CliContext, repo: bool) -> anyhow::Result<Vec<Target>> {
    let scope = if repo { Scope::Repo } else { Scope::User };
    let root = config::repo_root(&ctx.cwd);
    skill::targets(&ctx.env, &ctx.cwd, scope, root.as_deref())
}

fn install(ctx: &CliContext, args: InstallArgs) -> anyhow::Result<CommandResult> {
    let targets = targets(ctx, args.repo)?;
    let wanted: Vec<Agent> = if args.agents.iter().any(|a| a == "all") {
        Agent::ALL.to_vec()
    } else {
        args.agents
            .iter()
            .filter_map(|a| Agent::from_id(a))
            .collect()
    };
    if wanted.is_empty() {
        let found: Vec<&str> = targets
            .iter()
            .filter(|t| t.found)
            .map(|t| t.agent.id())
            .collect();
        let found = match found.as_slice() {
            [] => String::new(),
            ids => format!(" On this machine: {}.", ids.join(", ")),
        };
        bail!(
            "say which agents you use: ork skill install claude (or opencode, codex, gemini, \
             other, all).{found}"
        );
    }

    let mut done = Vec::new();
    for target in targets.iter().filter(|t| wanted.contains(&t.agent)) {
        let change = skill::install(&target.path)?;
        ctx.say(&format!(
            "✓ {:<13} {:<10} {}",
            target.agent.name(),
            change.as_str(),
            target.path.display()
        ));
        done.push(json!({
            "agent": target.agent.id(),
            "path": target.path.display().to_string(),
            "change": change.as_str(),
        }));
    }
    ctx.say("Start a new session of the agent so it reads the skill.");
    Ok(CommandResult::ok_with(json!({ "installed": done })))
}

fn list(ctx: &CliContext, args: ListArgs) -> anyhow::Result<CommandResult> {
    let targets = targets(ctx, args.repo)?;
    ctx.say(&format!("{:<14}{:<7}{:<10}PATH", "AGENT", "FOUND", "SKILL"));
    for target in &targets {
        ctx.say(&format!(
            "{:<14}{:<7}{:<10}{}",
            target.agent.name(),
            if target.found { "yes" } else { "-" },
            target.installed.as_str(),
            target.path.display()
        ));
    }
    let agents: Vec<_> = targets
        .iter()
        .map(|t| {
            json!({
                "agent": t.agent.id(),
                "name": t.agent.name(),
                "found": t.found,
                "skill": t.installed.as_str(),
                "path": t.path.display().to_string(),
            })
        })
        .collect();
    Ok(CommandResult::ok_with(json!({ "agents": agents })))
}
