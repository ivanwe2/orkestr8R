//! A fake OpenAI-compatible model server: replies from a script, records every request.
//!
//! ```ignore
//! let server = FakeModelServer::start(
//!     Script::new()
//!         .tool_call("read_file", json!({ "path": "src/calc.rs" }))
//!         .tool_call("finish", json!({ "summary": "done" })),
//! )
//! .await;
//! let client = server.client();
//! ```

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use orkestr8r::config::{RoleConfig, ServerConfig};
use orkestr8r::model::ModelClient;
use serde_json::{Value, json};
use tokio::task::JoinHandle;

/// What the server answers, one step per chat request, in order.
#[derive(Debug, Clone, Default)]
pub struct Script {
    steps: Vec<Step>,
    calls: usize,
}

#[derive(Debug, Clone)]
enum Step {
    Json(Value),
    Raw(String),
    Status(u16),
}

impl Script {
    pub fn new() -> Self {
        Self::default()
    }

    /// A reply with one tool call.
    pub fn tool_call(self, name: &str, args: Value) -> Self {
        self.tool_calls(vec![(name, args)])
    }

    /// A reply with several tool calls (ids `call_1`, `call_2`, … across the whole script).
    pub fn tool_calls(mut self, calls: Vec<(&str, Value)>) -> Self {
        let calls = calls
            .into_iter()
            .map(|(name, args)| self.call(name, args.to_string()))
            .collect();
        self.reply(Value::Null, calls, "tool_calls")
    }

    /// A reply with one tool call whose `arguments` string is `raw`, as is (broken JSON, say).
    pub fn malformed_args(mut self, name: &str, raw: &str) -> Self {
        let call = self.call(name, raw.to_string());
        self.reply(Value::Null, vec![call], "tool_calls")
    }

    /// A text reply, no tool calls.
    pub fn text(self, text: &str) -> Self {
        self.reply(json!(text), vec![], "stop")
    }

    /// A reply with exactly this body (status 200).
    pub fn raw(mut self, body: &str) -> Self {
        self.steps.push(Step::Raw(body.to_string()));
        self
    }

    /// An error status, with the body `error {code}`.
    pub fn status(mut self, code: u16) -> Self {
        self.steps.push(Step::Status(code));
        self
    }

    fn call(&mut self, name: &str, arguments: String) -> Value {
        self.calls += 1;
        json!({
            "id": format!("call_{}", self.calls),
            "type": "function",
            "function": { "name": name, "arguments": arguments },
        })
    }

    fn reply(mut self, content: Value, calls: Vec<Value>, finish_reason: &str) -> Self {
        let mut message = json!({ "role": "assistant", "content": content });
        if !calls.is_empty() {
            message["tool_calls"] = json!(calls);
        }
        self.steps.push(Step::Json(json!({
            "id": "chatcmpl-fake",
            "object": "chat.completion",
            "model": "fake-model",
            "choices": [{ "index": 0, "message": message, "finish_reason": finish_reason }],
            "usage": { "prompt_tokens": 100, "completion_tokens": 20, "total_tokens": 120 },
        })));
        self
    }
}

/// One request the server received.
#[derive(Debug, Clone)]
pub struct Recorded {
    pub path: String,
    pub authorization: Option<String>,
    pub body: Value,
}

struct Shared {
    steps: Mutex<VecDeque<Step>>,
    models: Vec<String>,
    recorded: Mutex<Vec<Recorded>>,
}

impl Shared {
    fn record(&self, path: &str, headers: &HeaderMap, body: &[u8]) {
        let authorization = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .map(String::from);
        let body = serde_json::from_slice(body).unwrap_or(Value::Null);
        self.recorded.lock().unwrap().push(Recorded {
            path: path.into(),
            authorization,
            body,
        });
    }
}

pub struct FakeModelServer {
    base_url: String,
    shared: Arc<Shared>,
    task: JoinHandle<()>,
}

impl FakeModelServer {
    /// Starts on 127.0.0.1 on a free port, offering the model `fake-model`.
    pub async fn start(script: Script) -> Self {
        Self::with_models(script, &["fake-model"]).await
    }

    pub async fn with_models(script: Script, models: &[&str]) -> Self {
        let shared = Arc::new(Shared {
            steps: Mutex::new(script.steps.into()),
            models: models.iter().map(|m| m.to_string()).collect(),
            recorded: Mutex::new(Vec::new()),
        });
        let app = Router::new()
            .route("/v1/chat/completions", post(chat))
            .route("/v1/models", get(list_models))
            .with_state(shared.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            base_url: format!("http://127.0.0.1:{port}/v1"),
            shared,
            task,
        }
    }

    /// `http://127.0.0.1:{port}/v1`
    pub fn base_url(&self) -> String {
        self.base_url.clone()
    }

    pub fn server_config(&self) -> ServerConfig {
        ServerConfig {
            url: self.base_url(),
            key_env: None,
        }
    }

    /// A client for `fake-model` on this server, with no waiting between retries.
    pub fn client(&self) -> ModelClient {
        self.client_with_key(None)
    }

    pub fn client_with_key(&self, key: Option<&str>) -> ModelClient {
        let role = RoleConfig {
            server: "box".into(),
            model: "fake-model".into(),
            ..Default::default()
        };
        let mut client = ModelClient::new(
            &self.server_config(),
            &role,
            key.map(String::from),
            Duration::from_secs(30),
        )
        .unwrap();
        client.retry_delays = vec![Duration::ZERO; 2];
        client
    }

    /// The body of every chat request, in order.
    pub fn requests(&self) -> Vec<Value> {
        self.recorded()
            .into_iter()
            .filter(|r| r.path.ends_with("/chat/completions"))
            .map(|r| r.body)
            .collect()
    }

    /// Every request, chat or not.
    pub fn recorded(&self) -> Vec<Recorded> {
        self.shared.recorded.lock().unwrap().clone()
    }
}

impl Drop for FakeModelServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// A URL where nothing listens: a port that was free a moment ago.
pub async fn unreachable_url() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    format!("http://127.0.0.1:{port}/v1")
}

async fn chat(State(shared): State<Arc<Shared>>, headers: HeaderMap, body: Bytes) -> Response {
    shared.record("/v1/chat/completions", &headers, &body);
    let step = shared.steps.lock().unwrap().pop_front();
    match step {
        Some(Step::Json(reply)) => axum::Json(reply).into_response(),
        Some(Step::Raw(body)) => {
            ([(header::CONTENT_TYPE, "application/json")], body).into_response()
        }
        Some(Step::Status(code)) => {
            let status = StatusCode::from_u16(code).unwrap();
            (status, format!("error {code}")).into_response()
        }
        None => (StatusCode::INTERNAL_SERVER_ERROR, "script exhausted").into_response(),
    }
}

async fn list_models(State(shared): State<Arc<Shared>>, headers: HeaderMap) -> Response {
    shared.record("/v1/models", &headers, b"");
    let data: Vec<Value> = shared
        .models
        .iter()
        .map(|id| json!({ "id": id, "object": "model" }))
        .collect();
    axum::Json(json!({ "object": "list", "data": data })).into_response()
}
