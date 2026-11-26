//! `ork ask`: a one-shot, read-only answer about some files from the local model. One request, no
//! loop, no tools: the chief uses it instead of reading a big file itself.

use std::path::PathBuf;

use anyhow::bail;
use clap::Args;
use serde_json::json;

use crate::cli::commands::load_config;
use crate::cli::{CliContext, CommandResult};
use crate::config;
use crate::events::OrkEvent;
use crate::model::{ChatRequest, ModelClient, WireMessage};
use crate::text;

const SYSTEM: &str =
    "Answer the question about the files below. Be concise. Cite file:line where it helps.";
const MAX_LINES_PER_FILE: usize = 400;
const MAX_TOTAL_CHARS: usize = 48_000;

#[derive(Debug, Args)]
pub struct AskArgs {
    /// The question
    pub question: String,
    /// A file to ask about, relative to where you are or to the repo root (repeat for more)
    #[arg(long = "path", required = true)]
    pub paths: Vec<PathBuf>,
}

pub async fn run(ctx: &CliContext, args: AskArgs) -> anyhow::Result<CommandResult> {
    // Like `ork run --path`: relative to where you are, else to the repo root.
    let root = config::repo_root(&ctx.cwd);
    let mut files = Vec::new();
    let mut missing = Vec::new();
    for path in &args.paths {
        let here = ctx.cwd.join(path);
        let found = if here.is_file() {
            Some(here)
        } else {
            root.as_ref()
                .map(|root| root.join(path))
                .filter(|file| file.is_file())
        };
        match found {
            Some(file) => files.push((path.display().to_string().replace('\\', "/"), file)),
            None => missing.push(path.display().to_string()),
        }
    }
    if !missing.is_empty() {
        bail!("not found: {}", missing.join(", "));
    }

    let loaded = load_config(ctx)?;
    let (client, role) = ModelClient::for_role(&loaded.config, "grunt", &ctx.env)?;

    let (prompt, truncated) = render(&args.question, &files)?;
    if !truncated.is_empty() {
        ctx.emit(OrkEvent::Warning {
            code: "truncated".into(),
            message: format!("cut to fit: {}", truncated.join(", ")),
        });
    }

    let request = ChatRequest {
        messages: vec![WireMessage::system(SYSTEM), WireMessage::user(prompt)],
        tools: vec![],
        temperature: role.temperature,
        max_tokens: role.max_output_tokens,
    };
    let reply = tokio::select! {
        reply = client.chat(&request) => reply?,
        _ = ctx.cancel.cancelled() => bail!("cancelled"),
    };
    ctx.emit(OrkEvent::Call {
        turn: 1,
        model: client.model().to_string(),
        server: client.server_name().to_string(),
        tokens_in: reply.tokens_in,
        tokens_out: reply.tokens_out,
        seconds: reply.seconds,
        tokens_per_second: reply.tokens_per_second,
        cost: reply.cost,
    });

    let answer = reply.content.unwrap_or_default();
    ctx.say(&answer);
    Ok(CommandResult::ok_with(json!({
        "answer": answer,
        "model": client.model(),
        "tokens_in": reply.tokens_in,
        "tokens_out": reply.tokens_out,
        "seconds": reply.seconds,
    })))
}

/// The user message: the question, then each file with numbered lines, within the caps. Returns
/// the files that were cut.
/// The question, then each file numbered under its name, cut to fit.
fn render(question: &str, files: &[(String, PathBuf)]) -> anyhow::Result<(String, Vec<String>)> {
    let mut prompt = format!("{question}\n");
    let mut truncated = Vec::new();

    for (name, file) in files {
        let name = name.clone();
        let room = MAX_TOTAL_CHARS.saturating_sub(prompt.chars().count());
        if room == 0 {
            truncated.push(name);
            continue;
        }
        let bytes = std::fs::read(file)?;
        let content = String::from_utf8_lossy(&bytes);
        let (numbered, cut_lines) = text::numbered(&content, 1, MAX_LINES_PER_FILE);
        let (block, cut_chars) = text::cut(&numbered, room);
        if cut_lines || cut_chars {
            truncated.push(name.clone());
        }
        let newline = if block.ends_with('\n') { "" } else { "\n" };
        prompt.push_str(&format!("\n## {name}\n```\n{block}{newline}```\n"));
    }
    Ok((prompt, truncated))
}
