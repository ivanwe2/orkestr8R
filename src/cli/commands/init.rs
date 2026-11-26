//! `ork init`: writes `ork.json` with the server and the grunt's model. The repo's file also gets
//! the build and test commands its build files imply, and `.ork/` is kept out of git.

use anyhow::{Context, bail};
use clap::Args;
use serde_json::json;

use crate::cli::{CliContext, CommandResult};
use crate::config::{self, OrkConfig};
use crate::events::OrkEvent;
use crate::git::Git;
use crate::process::ProcessRunner;

#[derive(Debug, Args)]
pub struct InitArgs {
    /// The server's OpenAI-compatible URL, ending in /v1 (e.g. http://gpu-box:9876/v1)
    #[arg(long)]
    pub url: String,
    /// The grunt's model, as the server names it
    #[arg(long)]
    pub model: String,
    /// The environment variable that holds the API key (the key itself never goes in the file).
    /// Needs --user.
    #[arg(long)]
    pub key_env: Option<String>,
    /// The model's context window, in tokens
    #[arg(long)]
    pub context_tokens: Option<u32>,
    /// Write the user's file instead of the repo's
    #[arg(long)]
    pub user: bool,
    /// Overwrite an existing file
    #[arg(long)]
    pub force: bool,
}

pub async fn run(ctx: &CliContext, args: InitArgs) -> anyhow::Result<CommandResult> {
    if args.key_env.is_some() && !args.user {
        bail!(
            "--key-env needs --user: API keys are only sent to servers your user file defines, \
             so a repo's ork.json can't send your key elsewhere"
        );
    }
    let root = config::repo_root(&ctx.cwd);
    let path = if args.user {
        config::user_path(&ctx.env)
            .context("can't find the user config directory (APPDATA or HOME isn't set)")?
    } else {
        root.as_ref()
            .context("not inside a git repository: run ork init in a repo, or use --user")?
            .join("ork.json")
    };
    if path.exists() && !args.force {
        bail!("{} exists; use --force to overwrite it", path.display());
    }

    // The commands belong to a repo: the user file has none (each repo's come from its build
    // files, or its own ork.json).
    let detect_in = root.as_deref().unwrap_or(&ctx.cwd);
    let allow_commands = if args.user {
        Vec::new()
    } else {
        config::detect_commands(detect_in)
    };
    if allow_commands.is_empty() && !args.user {
        ctx.emit(OrkEvent::Warning {
            code: "no_allow_commands".into(),
            message: format!(
                "found no build files (Cargo.toml, go.mod, .sln/.csproj, package.json scripts, pyproject.toml) in {}; add allow_commands to {} by hand",
                detect_in.display(),
                path.display()
            ),
        });
    }

    let mut server = json!({ "url": args.url });
    if let Some(key_env) = &args.key_env {
        server["key_env"] = json!(key_env);
    }
    let mut grunt = json!({ "server": "box", "model": args.model });
    if let Some(tokens) = args.context_tokens {
        grunt["context_tokens"] = json!(tokens);
    }
    let mut file = json!({
        "servers": { "box": server },
        "roles": { "grunt": grunt },
    });
    if !args.user {
        file["allow_commands"] = json!(allow_commands);
    }

    // Catch a bad --url now rather than at the first `ork run`.
    let problems = serde_json::from_value::<OrkConfig>(file.clone())?.validate();
    if !problems.is_empty() {
        bail!("not written:\n  {}", problems.join("\n  "));
    }

    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    std::fs::write(&path, serde_json::to_string_pretty(&file)? + "\n")
        .with_context(|| format!("writing {}", path.display()))?;
    if let (false, Some(root)) = (args.user, &root) {
        Git::new(ProcessRunner::new(ctx.env.clone()))
            .add_exclude(root, ".ork/")
            .await?;
    }

    ctx.say(&format!("wrote {}", path.display()));
    Ok(CommandResult::ok_with(
        json!({ "path": path.display().to_string() }),
    ))
}
