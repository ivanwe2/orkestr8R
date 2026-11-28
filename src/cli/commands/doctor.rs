//! `ork doctor`: is everything in place? Config, git, the server, tool calling, the allowed
//! commands. Each check says what it found and, when it fails, what to try.

use serde::Serialize;
use serde_json::json;

use crate::cli::commands::load_config;
use crate::cli::{CliContext, CommandResult};
use crate::config::{self, LoadedConfig};
use crate::model::{ChatRequest, ModelClient, ToolSchema, WireMessage};
use crate::process;

#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub name: &'static str,
    pub ok: bool,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl Check {
    fn pass(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            ok: true,
            detail: detail.into(),
            hint: None,
        }
    }

    fn fail(name: &'static str, detail: impl Into<String>, hint: impl Into<String>) -> Self {
        Self {
            name,
            ok: false,
            detail: detail.into(),
            hint: Some(hint.into()),
        }
    }

    fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}

#[derive(Debug, clap::Args)]
pub struct DoctorArgs {
    /// The role whose server and tool calling to check (default: grunt)
    #[arg(long, default_value = "grunt")]
    pub role: String,
}

pub async fn run(ctx: &CliContext, args: DoctorArgs) -> anyhow::Result<CommandResult> {
    let mut checks = Vec::new();

    let loaded = load_config(ctx);
    let usable = check_config(&loaded, &mut checks);
    checks.push(check_git(ctx).await);

    match usable.map(|config| ModelClient::for_role(config, &args.role, &ctx.env)) {
        Some(Ok((mut client, role))) => {
            // Diagnose, don't wait: a down server should show at once, not after the backoff.
            client.retry_delays.clear();
            checks.push(check_server(&client).await);
            checks
                .push(check_tool_calling(&client, role.temperature, role.max_output_tokens).await);
        }
        Some(Err(err)) => {
            checks.push(Check::fail(
                "server",
                format!("{err:#}"),
                "fix the config first",
            ));
            checks.push(Check::fail(
                "tool_calling",
                "skipped",
                "fix the config first",
            ));
        }
        None => {
            checks.push(Check::fail(
                "server",
                "skipped: no usable config",
                "fix the config first",
            ));
            checks.push(Check::fail(
                "tool_calling",
                "skipped: no usable config",
                "fix the config first",
            ));
        }
    }
    if let Ok(loaded) = &loaded {
        checks.push(check_commands(
            ctx,
            &loaded.config.allow_commands,
            &loaded.detected,
        ));
    }

    for check in &checks {
        let mark = if check.ok { "✓" } else { "✗" };
        ctx.say(&format!("{mark} {:<13} {}", check.name, check.detail));
        if let Some(hint) = &check.hint {
            ctx.say(&format!("  {:<13} hint: {hint}", ""));
        }
    }

    let failed = checks.iter().filter(|check| !check.ok).count();
    let data = json!({ "checks": checks });
    Ok(if failed == 0 {
        CommandResult::ok_with(data)
    } else {
        CommandResult::error(format!("{failed} of {} checks failed", checks.len())).with_data(data)
    })
}

/// The config check; returns the config when the server checks can use it.
fn check_config<'a>(
    loaded: &'a anyhow::Result<LoadedConfig>,
    checks: &mut Vec<Check>,
) -> Option<&'a config::OrkConfig> {
    match loaded {
        Err(err) => {
            checks.push(Check::fail(
                "config",
                format!("{err:#}"),
                "fix the file, or run ork init --force",
            ));
            None
        }
        Ok(loaded) if loaded.sources.is_empty() => {
            checks.push(Check::fail(
                "config",
                "no ork.json found",
                "run ork init --url URL --model MODEL (add --user for every repo)",
            ));
            None
        }
        Ok(loaded) => {
            let from: Vec<String> = loaded
                .sources
                .iter()
                .map(|p| p.display().to_string())
                .collect();
            let problems = loaded.config.validate();
            if problems.is_empty() {
                let mut check = Check::pass("config", from.join(" + "));
                if !loaded.warnings.is_empty() {
                    check = check.with_hint(loaded.warnings.join("; "));
                }
                checks.push(check);
                Some(&loaded.config)
            } else {
                checks.push(Check::fail(
                    "config",
                    problems.join("; "),
                    format!("edit {}", from.join(" or ")),
                ));
                None
            }
        }
    }
}

async fn check_git(ctx: &CliContext) -> Check {
    let version = tokio::process::Command::new("git")
        .arg("--version")
        .output()
        .await;
    let version = match version {
        Ok(output) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        }
        _ => return Check::fail("git", "git didn't run", "install git and put it on PATH"),
    };
    match config::repo_root(&ctx.cwd) {
        Some(root) => Check::pass("git", format!("{version} · repo {}", root.display())),
        None => Check::fail(
            "git",
            format!("{version} · not inside a repository"),
            "run ork in a git repo",
        ),
    }
}

async fn check_server(client: &ModelClient) -> Check {
    match client.list_models().await {
        Ok(models) if models.iter().any(|m| m == client.model()) => Check::pass(
            "server",
            format!(
                "{} · {} models, {} among them",
                client.base_url(),
                models.len(),
                client.model()
            ),
        ),
        Ok(models) => Check::pass(
            "server",
            format!("{} · {} models", client.base_url(), models.len()),
        )
        .with_hint(format!(
            "{} isn't in the server's list ({}); the request may still work, e.g. through an alias",
            client.model(),
            models.join(", ")
        )),
        Err(err) => Check::fail(
            "server",
            format!("{err:#}"),
            "is the server running, and is the url right (it ends in /v1)?",
        ),
    }
}

/// The one request that proves the model can drive the loop: given one tool, does it call it?
async fn check_tool_calling(client: &ModelClient, temperature: f64, max_tokens: u32) -> Check {
    let request = ChatRequest {
        messages: vec![WireMessage::user("Call the ping tool with value \"pong\".")],
        tools: vec![ToolSchema {
            name: "ping".into(),
            description: "Answers a ping.".into(),
            parameters: json!({
                "type": "object",
                "properties": { "value": { "type": "string" } },
                "required": ["value"]
            }),
        }],
        temperature,
        max_tokens,
    };
    const HINT: &str = "the model replied with text instead of a tool call; for llama-server, \
                        check the chat template / --jinja, or try another model";
    match client.chat(&request).await {
        Ok(reply) if reply.tool_calls.iter().any(|call| call.name == "ping") => {
            let mut detail = format!("{} called ping", client.model());
            if let Some(tps) = reply.tokens_per_second {
                detail += &format!(" · {tps:.0} t/s");
            }
            let check = Check::pass("tool_calling", detail);
            if reply.warnings.iter().any(|w| w == "tool_calls_in_content") {
                check.with_hint("the call came as text and was recovered; the chat template doesn't parse tool calls")
            } else {
                check
            }
        }
        Ok(reply) => {
            let said = reply.content.unwrap_or_default();
            let said: String = said.chars().take(80).collect();
            Check::fail(
                "tool_calling",
                format!("no tool call; it said: {said}"),
                HINT,
            )
        }
        Err(err) => {
            let message = format!("{err:#}");
            let hint = if message.contains("the server answered 5") {
                "the server failed while answering: the model may have crashed or run out of \
                 memory; look at the server's logs (llama-swap: GET /logs)"
            } else {
                "fix the server check first"
            };
            Check::fail("tool_calling", message, hint)
        }
    }
}

fn check_commands(ctx: &CliContext, allow: &[String], detected: &[String]) -> Check {
    if allow.is_empty() {
        return Check::pass("commands", "none allowed").with_hint(
            "orders need at least one check command: none found in this repo's build files; add allow_commands to its ork.json (e.g. \"npm test\")",
        );
    }
    let mut missing: Vec<&str> = Vec::new();
    for program in allow
        .iter()
        .filter_map(|entry| entry.split_whitespace().next())
    {
        if !missing.contains(&program) && process::find(program, &ctx.env, &ctx.cwd).is_none() {
            missing.push(program);
        }
    }
    if missing.is_empty() {
        let check = Check::pass("commands", allow.join(", "));
        if detected.is_empty() {
            check
        } else {
            check.with_hint(format!(
                "from this repo's build files: {} (list allow_commands in its ork.json to choose)",
                detected.join(", ")
            ))
        }
    } else {
        Check::fail(
            "commands",
            format!("not on PATH: {}", missing.join(", ")),
            "install them, or remove them from allow_commands",
        )
    }
}
