//! The chat agent, talking to OpenCode Go.
//!
//! Go is an OpenAI-compatible gateway: `Authorization: Bearer <key>` against
//! /chat/completions. The key stays on this side — the browser never sees it.
//!
//! The tool loop runs here rather than in the browser so a turn is one request:
//! ask, run whatever tools the model calls, ask again, until it answers.

use crate::{sandbox, store};
use serde_json::{json, Value};
use std::error::Error;

const BASE: &str = "https://opencode.ai/zen/go/v1";
/// Go asks clients to identify themselves rather than look like a bare HTTP library.
const USER_AGENT: &str = concat!("quantrig/", env!("CARGO_PKG_VERSION"));
/// Enough for write → run → read → explain, with room to retry a broken strategy.
const MAX_STEPS: usize = 8;

const SYSTEM: &str = "\
You are a quantitative trading assistant inside quantrig. You write FX strategies \
for the `backtestingfx` Python library and test them against downloaded candles.

A strategy subclasses `Strategy` and implements `next()`, called once per bar:
  from backtestingfx import Strategy
  class MyStrategy(Strategy):
      def init(self):        # optional, once before the loop; self._bars is every Bar
          ...
      def next(self):        # self._bar is the current bar; self.positions is open trades
          self.buy(lot_size=0.1, stop_loss=None, take_profit=None)
          self.sell(lot_size=0.1)
          self.close_all()
          self.close_position(id); self.close_partial(id, lots); self.update_sl(id, price)
Bar fields: open, high, low, close, volume, timestamp (unix int).
Exactly one Strategy subclass per file. Precompute indicators in init() — next() is \
called per bar and plain Python there is slow.

The file runs sandboxed: no network, no filesystem, no imports beyond backtestingfx \
and the standard library. Do not read files or call APIs.

Work by writing a strategy, running it, and reading the numbers back. When a run \
disappoints, say what the numbers indicate before changing anything. Never claim a \
result you have not run.";

fn tools() -> Value {
    json!([
      {"type": "function", "function": {
        "name": "list_strategies",
        "description": "List saved strategy names.",
        "parameters": {"type": "object", "properties": {}}
      }},
      {"type": "function", "function": {
        "name": "read_strategy",
        "description": "Read a saved strategy's Python source.",
        "parameters": {"type": "object", "required": ["name"], "properties": {
          "name": {"type": "string", "description": "Strategy name, without .py"}}}
      }},
      {"type": "function", "function": {
        "name": "write_strategy",
        "description": "Create or overwrite a strategy file.",
        "parameters": {"type": "object", "required": ["name", "code"], "properties": {
          "name": {"type": "string", "description": "Letters, digits, _ - . only"},
          "code": {"type": "string", "description": "Complete Python source"}}}
      }},
      {"type": "function", "function": {
        "name": "list_datasets",
        "description": "List downloaded candle datasets a backtest can run against.",
        "parameters": {"type": "object", "properties": {}}
      }},
      {"type": "function", "function": {
        "name": "run_backtest",
        "description": "Run a saved strategy against a dataset and return its stats.",
        "parameters": {"type": "object", "required": ["strategy", "dataset"], "properties": {
          "strategy": {"type": "string"},
          "dataset": {"type": "string", "description": "e.g. EUR_USD@1h"},
          "cash": {"type": "number", "default": 10000},
          "spread": {"type": "number", "default": 0.0001},
          "commission": {"type": "number", "default": 0.0}}}
      }}
    ])
}

/// Runs one tool call. Errors come back as text: the model should see what broke
/// and fix it, not have the whole turn fail.
fn call_tool(name: &str, args: &Value) -> String {
    let s = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let n = |k: &str, d: f64| args.get(k).and_then(Value::as_f64).unwrap_or(d);

    match name {
        "list_strategies" => json!(store::list_strategies()).to_string(),
        "list_datasets" => {
            let names: Vec<String> = std::fs::read_dir(store::candles_dir())
                .into_iter()
                .flatten()
                .flatten()
                .filter(|e| e.path().extension().is_some_and(|x| x == "parquet"))
                .filter_map(|e| e.path().file_stem().map(|s| s.to_string_lossy().to_string()))
                .collect();
            json!(names).to_string()
        }
        "read_strategy" => match store::strategy_path(&s("name")) {
            Some(p) => std::fs::read_to_string(p).unwrap_or_else(|e| format!("error: {e}")),
            None => "error: bad strategy name".into(),
        },
        "write_strategy" => match store::strategy_path(&s("name")) {
            Some(p) => {
                let code = s("code");
                if code.trim().is_empty() {
                    return "error: code was empty".into();
                }
                match std::fs::create_dir_all(store::strategies_dir())
                    .and_then(|_| std::fs::write(&p, &code))
                {
                    Ok(()) => format!("wrote {} ({} bytes)", p.display(), code.len()),
                    Err(e) => format!("error: {e}"),
                }
            }
            None => "error: bad strategy name".into(),
        },
        "run_backtest" => {
            let Some(strategy) = store::strategy_path(&s("strategy")).filter(|p| p.exists())
            else {
                return "error: no such strategy — write it first".into();
            };
            let Some(candles) = store::dataset_path(&s("dataset")).filter(|p| p.exists()) else {
                return "error: no such dataset — call list_datasets".into();
            };
            let config = json!({
                "cash": n("cash", 10000.0), "spread": n("spread", 0.0001),
                "commission": n("commission", 0.0), "plot": false
            })
            .to_string();
            let dir = store::runs_dir().join("agent");
            if let Err(e) = std::fs::create_dir_all(&dir) {
                return format!("error: {e}");
            }
            match sandbox::run_backtest(&strategy, &candles, &dir, &config) {
                Ok(stats) => stats,
                Err(e) => format!("the strategy failed: {e}"),
            }
        }
        other => format!("error: unknown tool {other}"),
    }
}

/// What the browser receives while a turn runs.
fn event(kind: &str, body: Value) -> Value {
    let mut out = json!({ "type": kind });
    if let (Some(o), Some(b)) = (out.as_object_mut(), body.as_object()) {
        o.extend(b.clone());
    }
    out
}

/// Providers require object-valued tool arguments even when a model emitted
/// malformed JSON. Keep history valid; execution still validates the raw call.
fn history_arguments(args: &Value) -> String {
    let parsed = match args {
        Value::String(raw) => serde_json::from_str::<Value>(raw).ok(),
        Value::Object(_) => Some(args.clone()),
        _ => None,
    };
    parsed.filter(Value::is_object).unwrap_or_else(|| json!({})).to_string()
}

fn normalize_history(messages: &mut [Value]) {
    for message in messages {
        if let Some(calls) = message["tool_calls"].as_array_mut() {
            for call in calls {
                if let Some(function) = call["function"].as_object_mut() {
                    let args = history_arguments(function.get("arguments").unwrap_or(&Value::Null));
                    function.insert("arguments".into(), Value::String(args));
                }
            }
        }
    }
}

/// Accumulates one streamed assistant message: text, reasoning, and tool calls
/// whose arguments arrive a fragment at a time, keyed by index.
#[derive(Default)]
struct Accumulator {
    content: String,
    reasoning: String,
    calls: Vec<(String, String, String)>, // id, name, arguments
}

impl Accumulator {
    fn take_delta(&mut self, delta: &Value, out: &tokio::sync::mpsc::Sender<Value>) {
        if let Some(text) = delta["content"].as_str().filter(|t| !t.is_empty()) {
            self.content.push_str(text);
            let _ = out.try_send(event("text", json!({ "delta": text })));
        }
        // Several Go models expose chain-of-thought under this field.
        if let Some(text) = delta["reasoning_content"]
            .as_str()
            .or_else(|| delta["reasoning"].as_str())
            .filter(|t| !t.is_empty())
        {
            self.reasoning.push_str(text);
            let _ = out.try_send(event("reasoning", json!({ "delta": text })));
        }
        for call in delta["tool_calls"].as_array().into_iter().flatten() {
            let index = call["index"].as_u64().unwrap_or(0) as usize;
            while self.calls.len() <= index {
                self.calls.push((String::new(), String::new(), String::new()));
            }
            let slot = &mut self.calls[index];
            if let Some(id) = call["id"].as_str() {
                slot.0 = id.to_string();
            }
            if let Some(name) = call["function"]["name"].as_str() {
                slot.1.push_str(name);
            }
            if let Some(args) = call["function"]["arguments"].as_str() {
                slot.2.push_str(args);
            }
        }
    }

    /// The assistant message to append to history, in the shape the API expects back.
    fn message(&self) -> Value {
        let mut message = json!({ "role": "assistant", "content": self.content });
        if !self.calls.is_empty() {
            message["tool_calls"] = Value::Array(
                self.calls
                    .iter()
                    .map(|(id, name, args)| {
                        json!({"id": id, "type": "function",
                               "function": {"name": name, "arguments": history_arguments(&json!(args))}})
                    })
                    .collect(),
            );
        }
        message
    }
}

/// One streamed request. Returns the assembled assistant message.
async fn stream_once(
    http: &reqwest::Client,
    key: &str,
    model: &str,
    session: &str,
    messages: &[Value],
    out: &tokio::sync::mpsc::Sender<Value>,
) -> Result<Accumulator, Box<dyn Error + Send + Sync>> {
    let mut res = http
        .post(format!("{BASE}/chat/completions"))
        .bearer_auth(key)
        // Go routes and caches per conversation; a stable id per chat is asked for.
        .header("x-opencode-session", session)
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .json(&json!({
            "model": model, "messages": messages, "tools": tools(), "stream": true
        }))
        .send()
        .await?;
    if !res.status().is_success() {
        let status = res.status();
        return Err(format!("OpenCode Go {status}: {}", res.text().await?).into());
    }

    let mut acc = Accumulator::default();
    let mut buffer = Vec::new();
    // SSE: `data:` lines, one JSON chunk each, terminated by [DONE]. Chunks can
    // be split across reads, so only complete lines are parsed.
    while let Some(bytes) = res.chunk().await? {
        buffer.extend_from_slice(&bytes);
        while let Some(nl) = buffer.iter().position(|b| *b == b'\n') {
            // Decode complete lines so UTF-8 characters split across network
            // chunks cannot corrupt JSON or strategy source.
            let line = std::str::from_utf8(&buffer[..nl])?.trim().to_string();
            buffer.drain(..=nl);
            let Some(payload) = line.strip_prefix("data:") else { continue };
            let payload = payload.trim();
            if payload.is_empty() || payload == "[DONE]" {
                continue;
            }
            let chunk = serde_json::from_str::<Value>(payload)?;
            if let Some(delta) = chunk["choices"].get(0).map(|c| &c["delta"]) {
                acc.take_delta(delta, out);
            }
        }
    }
    Ok(acc)
}

/// One chat turn, streamed: ask the model, run any tools it calls, repeat until
/// it answers. Events go to `out`; the browser rebuilds the transcript from them.
pub async fn turn(
    model: String,
    session: String,
    history: Vec<Value>,
    out: tokio::sync::mpsc::Sender<Value>,
) {
    let Some(key) = store::opencode_key() else {
        let _ = out
            .send(event("error", json!({"error": "no OpenCode Go API key set — add one in Settings"})))
            .await;
        return;
    };
    if !is_session_id(&session) {
        let _ = out.send(event("error", json!({"error": "bad session id"}))).await;
        return;
    }

    let http = reqwest::Client::new();
    let mut messages = vec![json!({"role": "system", "content": SYSTEM})];
    messages.extend(history);
    // Older saved chats may already contain the malformed call that failed.
    normalize_history(&mut messages);

    for _ in 0..MAX_STEPS {
        let acc = match stream_once(&http, &key, &model, &session, &messages, &out).await {
            Ok(acc) => acc,
            Err(e) => {
                let _ = out.send(event("error", json!({"error": e.to_string()}))).await;
                return;
            }
        };

        let message = acc.message();
        messages.push(message.clone());
        let _ = out.send(event("message", json!({"message": message}))).await;

        if acc.calls.is_empty() {
            let _ = out.send(event("done", json!({}))).await;
            return;
        }

        for (id, name, raw) in &acc.calls {
            let _ = out
                .send(event("tool", json!({"id": id, "name": name, "arguments": raw})))
                .await;
            // Arguments arrive as a JSON string, and a model can emit broken JSON.
            let args: Value = serde_json::from_str(raw).unwrap_or(Value::Null);
            let tool = name.clone();
            let result = tokio::task::spawn_blocking(move || match args {
                Value::Object(_) => call_tool(&tool, &args),
                _ => "error: arguments were not valid JSON".to_string(),
            })
            .await
            .unwrap_or_else(|e| format!("error: {e}"));

            let reply = json!({
                "role": "tool", "tool_call_id": id, "name": name, "content": result,
            });
            messages.push(reply.clone());
            let _ = out.send(event("tool_result", json!({"message": reply}))).await;
        }
    }
    let _ = out
        .send(event("error", json!({"error": format!("gave up after {MAX_STEPS} steps")})))
        .await;
}

/// Session ids come from the browser and go straight into a header.
fn is_session_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// The public model list — no key needed, so the picker works before setup.
pub async fn models() -> Result<Value, Box<dyn Error>> {
    Ok(reqwest::get(format!("{BASE}/models")).await?.json().await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_history_always_contains_json_object_arguments() {
        for raw in ["", "{broken", "null", "[]", "42"] {
            let acc = Accumulator {
                calls: vec![("call-1".into(), "run_backtest".into(), raw.into())],
                ..Default::default()
            };
            let message = acc.message();
            assert_eq!(message["tool_calls"][0]["function"]["arguments"], "{}");
            // Raw malformed arguments must remain invalid for tool execution.
            assert_eq!(acc.calls[0].2, raw);
        }
        let mut history = vec![json!({"role": "assistant", "tool_calls": [
            {"function": {"name": "run_backtest", "arguments": "{broken"}},
            {"function": {"name": "run_backtest", "arguments": {"cash": 10000}}},
            {"function": {"name": "run_backtest", "arguments": "{\"cash\":10000}"}}
        ]})];
        normalize_history(&mut history);
        for (index, expected) in [json!({}), json!({"cash": 10000}), json!({"cash": 10000})]
            .iter().enumerate()
        {
            let raw = history[0]["tool_calls"][index]["function"]["arguments"].as_str().unwrap();
            assert_eq!(&serde_json::from_str::<Value>(raw).unwrap(), expected);
        }
    }

    /// Repoints QUANTRIG_DATA, so it holds store::ENV_LOCK for the whole test.
    fn scratch(name: &str) -> (std::sync::MutexGuard<'static, ()>, std::path::PathBuf) {
        let guard = store::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("qr-agent-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        unsafe { std::env::set_var("QUANTRIG_DATA", &dir) };
        (guard, dir)
    }

    /// The tools an LLM drives, exercised the way it drives them: by name, with
    /// JSON arguments that may be wrong.
    #[test]
    fn tools_round_trip_a_strategy() {
        let (_guard, dir) = scratch("roundtrip");

        let wrote = call_tool("write_strategy", &json!({"name": "mine", "code": "x = 1\n"}));
        assert!(wrote.starts_with("wrote"), "{wrote}");
        assert_eq!(call_tool("list_strategies", &json!({})), r#"["mine"]"#);
        assert_eq!(call_tool("read_strategy", &json!({"name": "mine"})), "x = 1\n");

        let _ = std::fs::remove_dir_all(&dir);
        unsafe { std::env::remove_var("QUANTRIG_DATA") };
    }

    /// Bad input must come back as text the model can act on, never a panic.
    #[test]
    fn tools_report_errors_as_text() {
        let (_guard, dir) = scratch("errors");

        for (tool, args) in [
            ("write_strategy", json!({"name": "../escape", "code": "x"})),
            ("write_strategy", json!({"name": "ok", "code": "   "})),
            ("read_strategy", json!({"name": "missing"})),
            ("run_backtest", json!({"strategy": "nope", "dataset": "nope"})),
            ("wat", json!({})),
        ] {
            let out = call_tool(tool, &args);
            assert!(out.starts_with("error:"), "{tool} {args} gave {out}");
        }

        // A tool called with no arguments at all must not panic.
        assert!(call_tool("read_strategy", &json!({})).starts_with("error:"));

        let _ = std::fs::remove_dir_all(&dir);
        unsafe { std::env::remove_var("QUANTRIG_DATA") };
    }

    /// The schema is what the model reads; a malformed one silently disables a tool.
    #[test]
    fn every_tool_has_a_schema() {
        let listed: Vec<String> = tools()
            .as_array()
            .unwrap()
            .iter()
            .map(|t| {
                let f = &t["function"];
                assert!(f["description"].is_string(), "{f} has no description");
                assert!(f["parameters"]["type"] == "object", "{f} has no object schema");
                f["name"].as_str().unwrap().to_string()
            })
            .collect();
        assert_eq!(
            listed,
            ["list_strategies", "read_strategy", "write_strategy", "list_datasets", "run_backtest"]
        );
    }
}
