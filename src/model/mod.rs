//! The model client: one struct that talks to any OpenAI-compatible server (llama-server,
//! llama-swap, Ollama, vLLM, LM Studio, OpenRouter) and hides nothing.
//!
//! Cancelling: like every Rust future, [`ModelClient::chat`] only runs while it's polled, and
//! dropping it cancels it. Callers race it against the Ctrl+C token in `tokio::select!`; when the
//! token wins, the future is dropped and reqwest closes the connection. So the client takes no
//! token itself.

pub mod fallback;
pub mod think;
pub mod wire;

use std::time::{Duration, Instant};

use anyhow::{Context, anyhow, bail};
use reqwest::{RequestBuilder, StatusCode};
use serde_json::{Value, json};

pub use wire::{
    ChatRequest, ModelReply, ToolCall, ToolSchema, WireFunctionCall, WireMessage, WireToolCall,
};

use crate::config::{self, OrkConfig, RoleConfig, ServerConfig};
use crate::env::Env;

pub struct ModelClient {
    http: reqwest::Client,
    /// The server's base URL, without a trailing `/` (it already ends in `/v1`).
    base_url: String,
    /// The server's name in the config (`box`), for `call` events. Empty when built with `new`.
    server_name: String,
    model: String,
    api_key: Option<String>,
    /// How long to wait before each retry; one retry per entry. Tests set these to zero.
    pub retry_delays: Vec<Duration>,
}

impl ModelClient {
    /// The client for a role in the config (`grunt`): its server, its model, the key from the
    /// environment, the config's request timeout. Also returns the role's settings.
    pub fn for_role<'c>(
        config: &'c OrkConfig,
        role: &str,
        env: &Env,
    ) -> anyhow::Result<(Self, &'c RoleConfig)> {
        let role_config = config
            .roles
            .get(role)
            .with_context(|| format!("roles.{role} is missing from the config (run ork init)"))?;
        let server = config.servers.get(&role_config.server).with_context(|| {
            format!(
                "roles.{role}.server: \"{}\" is not in servers",
                role_config.server
            )
        })?;
        let timeout = Duration::from_secs(config.defaults.request_timeout_seconds);
        let mut client = Self::new(server, role_config, config::api_key(server, env), timeout)?;
        client.server_name = role_config.server.clone();
        Ok((client, role_config))
    }

    pub fn new(
        server: &ServerConfig,
        role: &RoleConfig,
        api_key: Option<String>,
        request_timeout: Duration,
    ) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(request_timeout)
            .build()
            .context("building the HTTP client")?;
        Ok(Self {
            http,
            base_url: server.url.trim_end_matches('/').to_string(),
            server_name: String::new(),
            model: role.model.clone(),
            api_key,
            retry_delays: vec![Duration::from_secs(2), Duration::from_secs(5)],
        })
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn server_name(&self) -> &str {
        &self.server_name
    }

    /// Whether an API key will be sent.
    pub fn has_key(&self) -> bool {
        self.api_key.is_some()
    }

    /// One chat completion: the whole conversation in, one reply out.
    pub async fn chat(&self, request: &ChatRequest) -> anyhow::Result<ModelReply> {
        let body = self.request_body(request);
        let url = format!("{}/chat/completions", self.base_url);
        let (raw, seconds) = self.send(|| self.http.post(&url).json(&body)).await?;
        parse_reply(&raw, seconds)
    }

    /// The ids of the models the server offers (`GET /models`).
    pub async fn list_models(&self) -> anyhow::Result<Vec<String>> {
        let url = format!("{}/models", self.base_url);
        let (raw, _) = self.send(|| self.http.get(&url)).await?;
        let body: Value =
            serde_json::from_str(&raw).context("GET /models: the reply is not JSON")?;
        let ids = body["data"].as_array().into_iter().flatten();
        Ok(ids
            .filter_map(|model| model["id"].as_str().map(String::from))
            .collect())
    }

    /// The request body, exactly as it goes over the wire.
    pub fn request_body(&self, request: &ChatRequest) -> Value {
        let mut body = json!({
            "model": self.model,
            "messages": request.messages,
            "temperature": request.temperature,
            "max_tokens": request.max_tokens,
            "stream": false,
        });
        if !request.tools.is_empty() {
            let tools: Vec<Value> = request
                .tools
                .iter()
                .map(|tool| json!({ "type": "function", "function": tool }))
                .collect();
            body["tools"] = json!(tools);
            body["tool_choice"] = json!("auto");
        }
        body
    }

    /// Sends a request, retrying what might pass on a second try, and returns the body and how
    /// long the successful attempt took.
    ///
    /// - Retried: network errors, 429 (rate limited, or llama-swap still loading a model) and 5xx.
    /// - Not retried: other 4xx (our request is wrong; it won't get better), and timeouts (the
    ///   model is just slow; a retry would double a long wait).
    async fn send(&self, request: impl Fn() -> RequestBuilder) -> anyhow::Result<(String, f64)> {
        let mut attempt = 0;
        loop {
            let started = Instant::now();
            let mut builder = request();
            if let Some(key) = &self.api_key {
                builder = builder.bearer_auth(key);
            }

            let failure = match builder.send().await {
                Ok(response) => {
                    let status = response.status();
                    let text = response.text().await;
                    match text {
                        Ok(text) if status.is_success() => {
                            return Ok((text, started.elapsed().as_secs_f64()));
                        }
                        Ok(text) if retryable(status) => {
                            anyhow!(server_error(status, &text))
                        }
                        Ok(text) => bail!(server_error(status, &text)),
                        Err(err) => anyhow!(err).context("reading the reply"),
                    }
                }
                Err(err) if err.is_timeout() => {
                    return Err(
                        anyhow!(err).context("the model took longer than request_timeout_seconds")
                    );
                }
                Err(err) => anyhow!(err).context(format!("can't reach {}", self.base_url)),
            };

            let Some(delay) = self.retry_delays.get(attempt) else {
                let tries = attempt + 1;
                return Err(failure.context(format!("gave up after {tries} tries")));
            };
            tokio::time::sleep(*delay).await;
            attempt += 1;
        }
    }
}

fn retryable(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

/// The error for a non-success status: the body is left out when it's empty.
fn server_error(status: StatusCode, text: &str) -> String {
    let body = snippet(text);
    if body.is_empty() {
        format!("the server answered {status}")
    } else {
        format!("the server answered {status}: {body}")
    }
}

/// The first 500 characters, for error messages.
fn snippet(text: &str) -> String {
    let text = text.trim();
    match text.char_indices().nth(500) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_string(),
    }
}

/// Reads a chat completion response, tolerantly: it's a `Value`, not strict structs, because
/// servers differ (llama.cpp adds `timings` and `reasoning_content`, vLLM says `reasoning`, some
/// omit `usage` or the call ids). Indexing a `Value` gives `Null` for anything missing, so an odd
/// reply degrades to defaults instead of failing the turn.
pub fn parse_reply(raw: &str, seconds: f64) -> anyhow::Result<ModelReply> {
    let body: Value = serde_json::from_str(raw)
        .with_context(|| format!("the server's reply is not JSON: {}", snippet(raw)))?;
    let choice = &body["choices"][0];
    if choice.is_null() {
        bail!("the server's reply has no choices: {}", snippet(raw));
    }
    let message = &choice["message"];
    let mut warnings = Vec::new();

    let mut tool_calls: Vec<ToolCall> = message["tool_calls"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .map(|(i, call)| ToolCall {
            // Every call needs an id: its result is tied to it. Some servers leave it out.
            id: call["id"]
                .as_str()
                .filter(|id| !id.is_empty())
                .map_or_else(|| format!("call_{}", i + 1), String::from),
            name: call["function"]["name"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            arguments: fallback::arguments_string(&call["function"]["arguments"]),
        })
        .collect();

    let mut content = message["content"].as_str().map(|text| {
        let (text, stripped) = think::strip(text);
        if stripped {
            warnings.push("think_stripped".to_string());
        }
        text
    });

    // Strip thinking first: a model may "think about" a call it then doesn't make.
    if tool_calls.is_empty()
        && let Some(text) = &content
    {
        let (calls, rest) = fallback::parse(text);
        if !calls.is_empty() {
            warnings.push("tool_calls_in_content".to_string());
            tool_calls = calls;
            content = Some(rest);
        }
    }

    let tokens_in = body["usage"]["prompt_tokens"].as_u64().unwrap_or(0);
    let tokens_out = body["usage"]["completion_tokens"].as_u64().unwrap_or(0);
    let tokens_per_second = body["timings"]["predicted_per_second"]
        .as_f64()
        .or_else(|| (tokens_out > 0 && seconds > 0.0).then(|| tokens_out as f64 / seconds));
    let reasoning = message["reasoning_content"]
        .as_str()
        .or_else(|| message["reasoning"].as_str())
        .map(String::from);

    Ok(ModelReply {
        content: content.filter(|text| !text.is_empty()),
        reasoning,
        tool_calls,
        finish_reason: choice["finish_reason"].as_str().map(String::from),
        tokens_in,
        tokens_out,
        seconds,
        tokens_per_second,
        warnings,
        raw_json: raw.to_string(),
    })
}
