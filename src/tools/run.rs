//! `run(cmd)`: an allowed command in the worktree. A non-zero exit is still `ok`: the command ran,
//! and its exit code is information the model reads. Only a refusal, a timeout or a failure to
//! start is an error.

use serde_json::{Value, json};

use super::{ToolContext, ToolResult, arg_str};
use crate::process::{HEAD_TAIL_MAX, ProcessSpec, head_tail};

pub const DESCRIPTION: &str = "Run an allowed command in the repository (no shell: no pipes, redirects \
     or &&) and get its exit code and output.";

pub fn parameters() -> Value {
    json!({
        "type": "object",
        "properties": {
            "cmd": { "type": "string", "description": "The command line, e.g. cargo test --test it" }
        },
        "required": ["cmd"]
    })
}

pub async fn run(args: &Value, ctx: &ToolContext) -> ToolResult {
    let cmd = arg_str(args, "cmd", "");
    let argv = match ctx.policy.check(cmd) {
        Ok(argv) => argv,
        Err(refused) => return ToolResult::error(refused),
    };
    let timeout = ctx.tool_timeout();
    let spec = ProcessSpec {
        argv,
        cwd: ctx.root.clone(),
        timeout,
    };
    match ctx.runner.run(spec, &ctx.cancel).await {
        Ok(result) if result.timed_out => ToolResult::error(format!(
            "timed out after {}s and was stopped\n{}",
            timeout.as_secs(),
            head_tail(&result.output, HEAD_TAIL_MAX)
        )),
        Ok(result) => ToolResult::ok(format!(
            "exit {} in {:.1}s\n{}",
            result.exit_code,
            result.seconds,
            head_tail(&result.output, HEAD_TAIL_MAX)
        )),
        Err(err) => ToolResult::error(format!("{err:#}")),
    }
}
