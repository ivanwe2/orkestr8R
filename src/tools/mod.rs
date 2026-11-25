//! The grunt's tools: what the model can do in its worktree.
//!
//! A tool never fails with a Rust error: whatever goes wrong comes back as a [`ToolResult`] with
//! `ok: false` and a message the model can act on ("not found; read_file the region again"). The
//! model reads it as the tool's result and tries again; an error would just end the run.
//!
//! The set of tools is closed, so it's an `enum` and dispatch is a `match`: adding a tool means
//! adding a variant, and the compiler points at every `match` that doesn't handle it yet.

pub mod args;
pub mod edit_file;
pub mod grep;
pub mod guard;
pub mod list_dir;
pub mod read_file;
pub mod run;
pub mod write_file;

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

pub use guard::PathGuard;

use crate::config::OrkConfig;
use crate::env::Env;
use crate::git::Git;
use crate::model::ToolSchema;
use crate::orders::{Order, OrderMode};
use crate::process::{CommandPolicy, ProcessRunner};

/// What a tool hands back: always text for the model, and whether it worked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolResult {
    pub ok: bool,
    pub output: String,
}

impl ToolResult {
    pub fn ok(output: impl Into<String>) -> Self {
        Self {
            ok: true,
            output: output.into(),
        }
    }

    pub fn error(output: impl Into<String>) -> Self {
        Self {
            ok: false,
            output: output.into(),
        }
    }
}

/// Everything a tool may use, built once per job.
#[derive(Debug, Clone)]
pub struct ToolContext {
    /// The worktree.
    pub root: PathBuf,
    pub order: Order,
    pub guard: PathGuard,
    pub runner: ProcessRunner,
    pub policy: CommandPolicy,
    pub config: OrkConfig,
    pub cancel: CancellationToken,
}

impl ToolContext {
    pub fn new(
        root: &Path,
        order: &Order,
        config: &OrkConfig,
        env: &Env,
        cancel: CancellationToken,
    ) -> anyhow::Result<Self> {
        let guard = PathGuard::new(root, &order.write_scope)?;
        Ok(Self {
            root: guard.root().to_path_buf(),
            guard,
            order: order.clone(),
            runner: ProcessRunner::new(env.clone()),
            policy: CommandPolicy::new(&config.allow_commands),
            config: config.clone(),
            cancel,
        })
    }

    pub fn git(&self) -> Git {
        Git::new(self.runner.clone())
    }

    pub fn tool_timeout(&self) -> Duration {
        Duration::from_secs(self.config.defaults.tool_timeout_seconds)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    ReadFile,
    ListDir,
    Grep,
    WriteFile,
    EditFile,
    Run,
    Finish,
}

impl Tool {
    pub const ALL: [Tool; 7] = [
        Tool::ReadFile,
        Tool::ListDir,
        Tool::Grep,
        Tool::WriteFile,
        Tool::EditFile,
        Tool::Run,
        Tool::Finish,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Tool::ReadFile => "read_file",
            Tool::ListDir => "list_dir",
            Tool::Grep => "grep",
            Tool::WriteFile => "write_file",
            Tool::EditFile => "edit_file",
            Tool::Run => "run",
            Tool::Finish => "finish",
        }
    }

    pub fn from_name(name: &str) -> Option<Tool> {
        Tool::ALL.into_iter().find(|tool| tool.name() == name)
    }

    /// Whether it can change the worktree (`readonly` orders don't get these).
    pub fn writes(self) -> bool {
        matches!(self, Tool::WriteFile | Tool::EditFile | Tool::Run)
    }

    pub fn schema(self) -> ToolSchema {
        let (description, parameters) = match self {
            Tool::ReadFile => (read_file::DESCRIPTION, read_file::parameters()),
            Tool::ListDir => (list_dir::DESCRIPTION, list_dir::parameters()),
            Tool::Grep => (grep::DESCRIPTION, grep::parameters()),
            Tool::WriteFile => (write_file::DESCRIPTION, write_file::parameters()),
            Tool::EditFile => (edit_file::DESCRIPTION, edit_file::parameters()),
            Tool::Run => (run::DESCRIPTION, run::parameters()),
            Tool::Finish => (FINISH_DESCRIPTION, finish_parameters()),
        };
        ToolSchema {
            name: self.name().into(),
            description: description.into(),
            parameters,
        }
    }

    /// Runs the tool with arguments that already passed [`args::validate`].
    pub async fn run(self, args: &Value, ctx: &ToolContext) -> ToolResult {
        match self {
            Tool::ReadFile => read_file::run(args, ctx),
            Tool::ListDir => list_dir::run(args, ctx).await,
            Tool::Grep => grep::run(args, ctx).await,
            Tool::WriteFile => write_file::run(args, ctx),
            Tool::EditFile => edit_file::run(args, ctx),
            Tool::Run => run::run(args, ctx).await,
            Tool::Finish => ToolResult::error("finish is handled by the loop"),
        }
    }
}

const FINISH_DESCRIPTION: &str = "Call when the goal is met, with a short summary of what you did. orkestr8R then \
     runs the checks; if they fail you get the output and can fix it. Also call it when something \
     blocks you, with the question in open_questions.";

fn finish_parameters() -> Value {
    json!({
        "type": "object",
        "properties": {
            "summary": { "type": "string", "description": "What you did, in a few lines" },
            "open_questions": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Questions for whoever gave the order, if something blocked you"
            }
        },
        "required": ["summary"]
    })
}

/// The tools an order gets.
#[derive(Debug, Clone)]
pub struct ToolRegistry {
    tools: Vec<Tool>,
}

impl ToolRegistry {
    /// `write`: every tool. `readonly`: only the ones that can't change anything.
    pub fn for_mode(mode: OrderMode) -> Self {
        let tools = match mode {
            OrderMode::Write => Tool::ALL.to_vec(),
            OrderMode::Readonly => Tool::ALL
                .into_iter()
                .filter(|tool| !tool.writes())
                .collect(),
        };
        Self { tools }
    }

    pub fn tools(&self) -> &[Tool] {
        &self.tools
    }

    pub fn get(&self, name: &str) -> Option<Tool> {
        self.tools.iter().copied().find(|tool| tool.name() == name)
    }

    /// What the model is offered.
    pub fn schemas(&self) -> Vec<ToolSchema> {
        self.tools.iter().map(|tool| tool.schema()).collect()
    }

    /// Looks the tool up, checks the arguments, runs it. Every failure is a result for the model.
    pub async fn call(&self, name: &str, args_json: &str, ctx: &ToolContext) -> ToolResult {
        match self.prepare(name, args_json) {
            Ok((tool, args)) => tool.run(&args, ctx).await,
            Err(problem) => ToolResult::error(problem),
        }
    }

    /// The tool and its checked arguments, or why the call is malformed (an unknown tool, or
    /// arguments that don't fit its schema). The loop's guards count malformed calls apart from
    /// tools that ran and failed.
    pub fn prepare(&self, name: &str, args_json: &str) -> Result<(Tool, Value), String> {
        let Some(tool) = self.get(name) else {
            let names: Vec<&str> = self.tools.iter().map(|tool| tool.name()).collect();
            return Err(format!(
                "unknown tool `{name}`; the tools are: {}",
                names.join(", ")
            ));
        };
        let args = args::validate(args_json, &tool.schema().parameters)?;
        Ok((tool, args))
    }
}

/// A string argument (validated already), or `default`.
pub(crate) fn arg_str<'a>(args: &'a Value, name: &str, default: &'a str) -> &'a str {
    args[name].as_str().unwrap_or(default)
}

pub(crate) fn arg_usize(args: &Value, name: &str) -> Option<usize> {
    args[name].as_i64().map(|n| n.max(0) as usize)
}
