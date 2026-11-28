use orkestr8r::config::{RoleConfig, ServerConfig};
use orkestr8r::model::{ChatRequest, ModelClient, ToolSchema, WireMessage, fallback, think};
use serde_json::{Value, json};
use std::time::Duration;

use crate::support::fake_server::{FakeModelServer, Script, unreachable_url};

fn read_file_schema() -> ToolSchema {
    ToolSchema {
        name: "read_file".into(),
        description: "Read a file, with line numbers".into(),
        parameters: json!({
            "type": "object",
            "properties": { "path": { "type": "string" } },
            "required": ["path"]
        }),
    }
}

fn request(with_tools: bool) -> ChatRequest {
    ChatRequest {
        messages: vec![
            WireMessage::system("You are a coding agent."),
            WireMessage::user("Add Sum to src/calc.rs"),
        ],
        tools: if with_tools {
            vec![read_file_schema()]
        } else {
            vec![]
        },
        temperature: 0.2,
        max_tokens: 512,
    }
}

#[tokio::test]
async fn tool_calls_are_parsed_and_the_raw_reply_is_kept() {
    let server = FakeModelServer::start(
        Script::new().tool_call("read_file", json!({ "path": "src/calc.rs" })),
    )
    .await;

    let reply = server.client().chat(&request(true)).await.unwrap();

    assert_eq!(reply.tool_calls.len(), 1);
    let call = &reply.tool_calls[0];
    assert_eq!(
        (call.id.as_str(), call.name.as_str()),
        ("call_1", "read_file")
    );
    // arguments stay the raw string the model wrote
    assert_eq!(call.arguments, r#"{"path":"src/calc.rs"}"#);
    assert_eq!(reply.finish_reason.as_deref(), Some("tool_calls"));
    assert_eq!(reply.content, None);
    assert_eq!((reply.tokens_in, reply.tokens_out), (100, 20));
    assert!(reply.raw_json.contains("chatcmpl-fake"));
    let raw: Value = serde_json::from_str(&reply.raw_json).unwrap();
    assert_eq!(raw["usage"]["total_tokens"], 120);
}

#[tokio::test]
async fn the_request_has_the_openai_shape() {
    let server = FakeModelServer::start(Script::new().text("ok").text("ok")).await;
    let client = server.client();

    client.chat(&request(true)).await.unwrap();
    client.chat(&request(false)).await.unwrap();

    let requests = server.requests();
    let with_tools = &requests[0];
    assert_eq!(with_tools["model"], "fake-model");
    assert_eq!(with_tools["stream"], false);
    assert_eq!(with_tools["temperature"], 0.2);
    assert_eq!(with_tools["max_tokens"], 512);
    assert_eq!(
        with_tools["messages"][0],
        json!({ "role": "system", "content": "You are a coding agent." })
    );
    assert_eq!(with_tools["tools"][0]["type"], "function");
    assert_eq!(with_tools["tools"][0]["function"]["name"], "read_file");
    assert_eq!(
        with_tools["tools"][0]["function"]["parameters"]["required"],
        json!(["path"])
    );
    assert_eq!(with_tools["tool_choice"], "auto");

    let without = requests[1].as_object().unwrap();
    assert!(!without.contains_key("tools") && !without.contains_key("tool_choice"));
}

#[tokio::test]
async fn an_assistant_message_with_only_tool_calls_sends_null_content() {
    let server = FakeModelServer::start(
        Script::new()
            .tool_call("read_file", json!({ "path": "a" }))
            .text("ok"),
    )
    .await;
    let client = server.client();
    let mut req = request(true);

    let reply = client.chat(&req).await.unwrap();
    req.messages.push(reply.to_message());
    req.messages
        .push(WireMessage::tool("call_1", "1 │ fn main() {}"));
    client.chat(&req).await.unwrap();

    let messages = &server.requests()[1]["messages"];
    assert_eq!(messages[2]["role"], "assistant");
    assert_eq!(messages[2]["content"], Value::Null);
    assert_eq!(messages[2]["tool_calls"][0]["id"], "call_1");
    assert_eq!(
        messages[2]["tool_calls"][0]["function"]["arguments"],
        r#"{"path":"a"}"#
    );
    assert_eq!(
        messages[3],
        json!({ "role": "tool", "content": "1 │ fn main() {}", "tool_call_id": "call_1" })
    );
}

#[test]
fn think_stripping_cases() {
    assert_eq!(think::strip("  hello  "), ("hello".into(), false));
    assert_eq!(
        think::strip("<think>hmm</think>Hello"),
        ("Hello".into(), true)
    );
    assert_eq!(
        think::strip("a<think>1</think>b<think>2</think>c"),
        ("abc".into(), true)
    );
    // an orphan close: the template opened the block in the prompt
    assert_eq!(
        think::strip("thinking aloud\n</think>\n\nAnswer"),
        ("Answer".into(), true)
    );
    // an unclosed open: cut off mid-thought
    assert_eq!(
        think::strip("Answer <think>and then I was cut o"),
        ("Answer".into(), true)
    );
    assert_eq!(
        think::strip("<think>ünïcödé…</think>→ ok"),
        ("→ ok".into(), true)
    );
}

#[tokio::test]
async fn think_blocks_are_stripped_from_the_content() {
    let server =
        FakeModelServer::start(Script::new().text("<think>Let me see…</think>\nHello")).await;

    let reply = server.client().chat(&request(false)).await.unwrap();

    assert_eq!(reply.content.as_deref(), Some("Hello"));
    assert_eq!(reply.warnings, ["think_stripped"]);
    assert!(
        reply.raw_json.contains("Let me see"),
        "the transcript keeps it"
    );
}

#[tokio::test]
async fn reasoning_content_stays_out_of_the_content() {
    let body = json!({
        "choices": [{
            "message": { "role": "assistant", "content": "Hi", "reasoning_content": "The user greets me." },
            "finish_reason": "stop"
        }],
        "usage": { "prompt_tokens": 10, "completion_tokens": 3 },
        "timings": { "predicted_per_second": 51.7 }
    });
    let server = FakeModelServer::start(Script::new().raw(&body.to_string())).await;

    let reply = server.client().chat(&request(false)).await.unwrap();

    assert_eq!(reply.content.as_deref(), Some("Hi"));
    assert_eq!(reply.reasoning.as_deref(), Some("The user greets me."));
    assert_eq!(reply.to_message().content.as_deref(), Some("Hi"));
    // llama.cpp's own speed figure wins over our estimate
    assert_eq!(reply.tokens_per_second, Some(51.7));
    assert_eq!(reply.cost, None, "a local server sends no cost");
}

#[tokio::test]
async fn a_cost_in_the_usage_is_kept() {
    // OpenRouter's shape: the price of the request, in US dollars, next to the token counts.
    let body = json!({
        "choices": [{ "message": { "role": "assistant", "content": "Hi" }, "finish_reason": "stop" }],
        "usage": { "prompt_tokens": 1000, "completion_tokens": 20, "cost": 0.00042 }
    });
    let server = FakeModelServer::start(Script::new().raw(&body.to_string())).await;

    let reply = server.client().chat(&request(false)).await.unwrap();

    assert_eq!(reply.cost, Some(0.00042));
}

#[test]
fn the_fallback_parses_tool_call_blocks() {
    let content = "Let me look.\n<tool_call>\n{\"name\": \"read_file\", \"arguments\": {\"path\": \"a.rs\"}}\n</tool_call>\n\
                   <tool_call>{\"name\": \"grep\", \"arguments\": \"{\\\"pattern\\\": \\\"Sum\\\"}\"}</tool_call>\n\
                   <tool_call>not json</tool_call>";

    let (calls, rest) = fallback::parse(content);

    assert_eq!(calls.len(), 2);
    assert_eq!(
        (calls[0].id.as_str(), calls[0].name.as_str()),
        ("fallback_1", "read_file")
    );
    assert_eq!(calls[0].arguments, r#"{"path":"a.rs"}"#);
    assert_eq!(
        (calls[1].id.as_str(), calls[1].arguments.as_str()),
        ("fallback_2", r#"{"pattern": "Sum"}"#)
    );
    // the two calls are gone (their newlines stay); the block that isn't a call is kept
    assert_eq!(rest, "Let me look.\n\n\n<tool_call>not json</tool_call>");
}

#[tokio::test]
async fn tool_calls_written_into_the_text_become_tool_calls() {
    let text = "<think>I should read it</think>Let me look.\n<tool_call>{\"name\": \"read_file\", \"arguments\": {\"path\": \"a.rs\"}}</tool_call>";
    let server = FakeModelServer::start(Script::new().text(text)).await;

    let reply = server.client().chat(&request(true)).await.unwrap();

    assert_eq!(reply.tool_calls.len(), 1);
    assert_eq!(reply.tool_calls[0].id, "fallback_1");
    assert_eq!(reply.content.as_deref(), Some("Let me look."));
    assert_eq!(reply.warnings, ["think_stripped", "tool_calls_in_content"]);
}

#[tokio::test]
async fn malformed_arguments_are_kept_as_sent() {
    let server =
        FakeModelServer::start(Script::new().malformed_args("read_file", r#"{"path": "a.rs""#))
            .await;

    let reply = server.client().chat(&request(true)).await.unwrap();

    assert_eq!(reply.tool_calls[0].arguments, r#"{"path": "a.rs""#);
}

#[tokio::test]
async fn a_500_is_retried() {
    let server = FakeModelServer::start(Script::new().status(500).text("ok")).await;

    let reply = server.client().chat(&request(false)).await.unwrap();

    assert_eq!(reply.content.as_deref(), Some("ok"));
    assert_eq!(server.requests().len(), 2);
}

#[tokio::test]
async fn retries_run_out_after_two() {
    let server = FakeModelServer::start(
        Script::new()
            .status(503)
            .status(429)
            .status(502)
            .text("too late"),
    )
    .await;

    let err = server.client().chat(&request(false)).await.unwrap_err();

    assert_eq!(server.requests().len(), 3);
    let message = format!("{err:#}");
    assert!(
        message.contains("gave up after 3 tries") && message.contains("502"),
        "{message}"
    );
}

#[tokio::test]
async fn a_400_fails_at_once_with_the_body() {
    let server = FakeModelServer::start(Script::new().status(400).text("never")).await;

    let err = server.client().chat(&request(false)).await.unwrap_err();

    assert_eq!(server.requests().len(), 1, "a 4xx is not retried");
    let message = format!("{err:#}");
    assert!(
        message.contains("400") && message.contains("error 400"),
        "{message}"
    );
}

#[tokio::test]
async fn the_auth_header_is_sent_only_with_a_key() {
    let server = FakeModelServer::start(Script::new().text("a").text("b")).await;

    server.client().chat(&request(false)).await.unwrap();
    server
        .client_with_key(Some("s3cret"))
        .chat(&request(false))
        .await
        .unwrap();

    let auth: Vec<_> = server
        .recorded()
        .into_iter()
        .map(|r| r.authorization)
        .collect();
    assert_eq!(auth, [None, Some("Bearer s3cret".to_string())]);
}

#[tokio::test]
async fn list_models_returns_the_ids() {
    let server = FakeModelServer::with_models(Script::new(), &["qwen-27b", "gemma-12b"]).await;

    let models = server.client().list_models().await.unwrap();

    assert_eq!(models, ["qwen-27b", "gemma-12b"]);
}

#[tokio::test]
async fn an_unreachable_server_is_an_error() {
    let server = ServerConfig {
        url: unreachable_url().await,
        key_env: None,
    };
    let role = RoleConfig {
        model: "m".into(),
        ..Default::default()
    };
    let mut client = ModelClient::new(&server, &role, None, Duration::from_secs(5)).unwrap();
    client.retry_delays = vec![Duration::ZERO; 2];

    let err = client.chat(&request(false)).await.unwrap_err();

    let message = format!("{err:#}");
    assert!(message.contains("can't reach"), "{message}");
}

#[tokio::test]
async fn a_reply_without_choices_is_an_error() {
    let server =
        FakeModelServer::start(Script::new().raw(r#"{"error": "model not loaded"}"#)).await;

    let err = server.client().chat(&request(false)).await.unwrap_err();

    assert!(format!("{err:#}").contains("no choices"), "{err:#}");
}

fn client_for(url: &str, thinking: Option<bool>, extra_body: Option<Value>) -> ModelClient {
    let server = ServerConfig {
        url: url.into(),
        ..Default::default()
    };
    let role = RoleConfig {
        model: "m".into(),
        thinking,
        extra_body,
        ..Default::default()
    };
    ModelClient::new(&server, &role, None, Duration::from_secs(5)).unwrap()
}

#[test]
fn the_thinking_switch_speaks_each_servers_language() {
    let local = |thinking| {
        client_for("http://127.0.0.1:8080/v1", thinking, None).request_body(&request(false))
    };
    assert_eq!(
        local(Some(false))["chat_template_kwargs"],
        json!({ "enable_thinking": false })
    );
    assert_eq!(
        local(Some(true))["chat_template_kwargs"],
        json!({ "enable_thinking": true })
    );
    let unset = local(None);
    assert!(unset.get("chat_template_kwargs").is_none() && unset.get("reasoning").is_none());

    let router = |thinking| {
        client_for("https://openrouter.ai/api/v1", thinking, None).request_body(&request(false))
    };
    assert_eq!(
        router(Some(false))["reasoning"],
        json!({ "effort": "none" })
    );
    assert_eq!(router(Some(true))["reasoning"], json!({ "enabled": true }));
    assert!(router(Some(false)).get("chat_template_kwargs").is_none());
}

#[test]
fn extra_body_is_merged_last_and_wins() {
    let client = client_for(
        "http://127.0.0.1:8080/v1",
        Some(false),
        Some(json!({ "chat_template_kwargs": { "enable_thinking": true, "x": 1 }, "top_k": 20 })),
    );
    let body = client.request_body(&request(false));
    assert_eq!(body["top_k"], 20);
    assert_eq!(
        body["chat_template_kwargs"],
        json!({ "enable_thinking": true, "x": 1 })
    );
    assert_eq!(body["model"], "m", "the rest of the body stands");
}
