//! `ork models`: which model does each role, on which server; `--test` sends each a tiny request.

use clap::Args;
use serde::Serialize;
use serde_json::json;

use crate::cli::commands::load_config;
use crate::cli::{CliContext, CommandResult};
use crate::model::{ChatRequest, ModelClient, WireMessage};

#[derive(Debug, Args)]
pub struct ModelsArgs {
    /// Send each role's model a tiny request and report the speed
    #[arg(long)]
    pub test: bool,
}

#[derive(Debug, Serialize)]
struct RoleLine {
    role: String,
    /// The role an order runs on when it names none.
    default: bool,
    server: String,
    url: String,
    model: String,
    key_set: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    test: Option<TestResult>,
}

#[derive(Debug, Serialize)]
struct TestResult {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    seconds: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tokens_in: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tokens_out: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tokens_per_second: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

pub async fn run(ctx: &CliContext, args: ModelsArgs) -> anyhow::Result<CommandResult> {
    let loaded = load_config(ctx)?;
    let config = &loaded.config;
    let mut lines = Vec::new();

    for name in config.roles.keys() {
        let (mut client, role) = ModelClient::for_role(config, name, &ctx.env)?;
        client.retry_delays.clear();
        let test = if args.test {
            Some(test_role(&client, role.temperature).await)
        } else {
            None
        };

        let key = match config
            .servers
            .get(&role.server)
            .and_then(|s| s.key_env.as_deref())
        {
            Some(var) if client.has_key() => format!("key: {var} (set)"),
            Some(var) => format!("key: {var} (NOT set)"),
            None => "no key".to_string(),
        };
        let default = name == crate::orders::DEFAULT_ROLE;
        let mut human = format!(
            "{name}{}  {}  {}  {}  {key}",
            if default { " (default)" } else { "" },
            role.server,
            client.base_url(),
            role.model
        );
        match role.thinking {
            Some(true) => human += "  thinking on",
            Some(false) => human += "  thinking off",
            None => {}
        }
        match &test {
            Some(TestResult {
                ok: true,
                seconds: Some(s),
                tokens_in: Some(i),
                tokens_out: Some(o),
                tokens_per_second,
                ..
            }) => {
                human += &format!("  ok {s:.1}s {i}→{o} tok");
                if let Some(tps) = tokens_per_second {
                    human += &format!(" {tps:.0} t/s");
                }
            }
            Some(TestResult {
                error: Some(error), ..
            }) => human += &format!("  failed: {error}"),
            _ => {}
        }
        ctx.say(&human);

        lines.push(RoleLine {
            role: name.clone(),
            default,
            server: role.server.clone(),
            url: client.base_url().to_string(),
            model: role.model.clone(),
            key_set: client.has_key(),
            thinking: role.thinking,
            test,
        });
    }

    let failed = lines
        .iter()
        .filter(|line| line.test.as_ref().is_some_and(|t| !t.ok))
        .count();
    let data = json!({ "roles": lines });
    Ok(if failed == 0 {
        CommandResult::ok_with(data)
    } else {
        CommandResult::error(format!("{failed} role(s) failed the test")).with_data(data)
    })
}

async fn test_role(client: &ModelClient, temperature: f64) -> TestResult {
    let request = ChatRequest {
        messages: vec![WireMessage::user("Reply with OK.")],
        tools: vec![],
        temperature,
        max_tokens: 256,
    };
    match client.chat(&request).await {
        Ok(reply) => TestResult {
            ok: true,
            seconds: Some(reply.seconds),
            tokens_in: Some(reply.tokens_in),
            tokens_out: Some(reply.tokens_out),
            tokens_per_second: reply.tokens_per_second,
            error: None,
        },
        Err(err) => TestResult {
            ok: false,
            seconds: None,
            tokens_in: None,
            tokens_out: None,
            tokens_per_second: None,
            error: Some(format!("{err:#}")),
        },
    }
}
