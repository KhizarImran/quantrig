//! The chat agent, talking to OpenCode Go.
//!
//! Go is an OpenAI-compatible gateway: `Authorization: Bearer <key>` against
//! /chat/completions. The key stays on this side — the browser never sees it.
//!
//! The tool loop runs here rather than in the browser so a turn is one request:
//! ask, run whatever tools the model calls, ask again, until it answers.

use crate::{backtests, chatgpt, store};
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
result you have not run. Pass a stable project name to run_backtest for the strategy \
family (for example Martingale), reusing it across pairs, timeframes and variants \
so the user can browse those runs together in Backtest history.
When asked about previous backtests or to compare existing results, first use \
list_backtests and read_backtest to inspect saved runs, including manual runs. \
Do not rerun strategies merely to retrieve existing results. read_backtest returns \
summary metrics by default; request code: \"yes\" only when inspecting the original strategy source is necessary. Older archived runs \
may lack saved statistics or settings; explain missing information rather than inventing it.";

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
        "name": "list_backtests",
        "description": "List saved Backtest history, newest first, including manual and agent runs. Returns run IDs, strategy projects, datasets, status and available returns. Use read_backtest for full results.",
        "parameters": {"type": "object", "properties": {}}
      }},
      {"type": "function", "function": {
        "name": "read_backtest",
        "description": "Read a saved backtest's ten summary metrics: total_return_pct, final_cash (final balance), num_trades, win_rate_pct, profit_factor, max_drawdown_pct, sharpe_ratio, avg_pnl, best_trade and worst_trade. Includes run identity and errors. Code is omitted unless code is explicitly yes. Missing archived metrics are null. Does not rerun the strategy.",
        "parameters": {"type": "object", "required": ["id"], "properties": {
          "id": {"type": "string", "description": "Run ID returned by list_backtests"},
          "code": {"type": "string", "enum": ["yes", "no"], "description": "Optional; only yes includes the original strategy source. Omit to save tokens."}}}
      }},
      {"type": "function", "function": {
        "name": "run_backtest",
        "description": "Run a saved strategy against a dataset and return its stats.",
        "parameters": {"type": "object", "required": ["strategy", "dataset"], "properties": {
          "strategy": {"type": "string"},
          "dataset": {"type": "string", "description": "e.g. EUR_USD@1h"},
          "project": {"type": "string", "description": "Strategy family/project, e.g. Martingale. Reuse across pairs and timeframes."},
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
        "list_backtests" => json!(backtests::list()).to_string(),
        "read_backtest" => match backtests::load(&s("id")) {
            Ok(run) => {
                let metrics: serde_json::Map<String, Value> = [
                    "total_return_pct", "final_cash", "num_trades", "win_rate_pct",
                    "profit_factor", "max_drawdown_pct", "sharpe_ratio", "avg_pnl",
                    "best_trade", "worst_trade",
                ].into_iter().map(|key| (key.into(), run.stats.as_ref()
                    .and_then(|stats| stats.get(key)).cloned().unwrap_or(Value::Null))).collect();
                let mut result = json!({"id": run.id, "project": run.project,
                    "strategy": run.strategy, "dataset": run.dataset,
                    "stats": metrics, "error": run.error});
                if s("code") == "yes" { result["code"] = json!(run.code); }
                result.to_string()
            },
            Err(e) => format!("error: {e}"),
        },
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
            let Some(_candles) = store::dataset_path(&s("dataset")).filter(|p| p.exists()) else {
                return "error: no such dataset — call list_datasets".into();
            };
            let code = match std::fs::read_to_string(strategy) {
                Ok(code) => code,
                Err(e) => return format!("error: {e}"),
            };
            match backtests::execute(code, s("strategy"), s("dataset"), s("project"),
                backtests::Config { cash: n("cash", 10000.0), spread: n("spread", 0.0001), commission: n("commission", 0.0) }, "agent") {
                Ok(run) => json!({"id": run.id, "project": run.project, "stats": run.stats, "error": run.error}).to_string(),
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
    response_items: Vec<Value>,
    streamed_items: std::collections::BTreeMap<u64, Value>,
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
        if !self.response_items.is_empty() { message["response_items"] = json!(self.response_items); }
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
    let mut go_messages = messages.to_vec();
    for m in &mut go_messages { if let Some(obj) = m.as_object_mut() { obj.remove("response_items"); } }
    let mut res = http
        .post(format!("{BASE}/chat/completions"))
        .bearer_auth(key)
        // Go routes and caches per conversation; a stable id per chat is asked for.
        .header("x-opencode-session", session)
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .json(&json!({
            "model": model, "messages": go_messages, "tools": tools(), "stream": true
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

/// Translate persisted chat-completions history into stateless Responses items.
fn response_input(messages: &[Value]) -> Vec<Value> {
    let mut input = Vec::new();
    for m in messages {
        match m["role"].as_str() {
            Some("system") => {},
            Some("tool") => input.push(json!({"type": "function_call_output", "call_id": m["tool_call_id"], "output": m["content"]})),
            Some("assistant") => {
                if let Some(items) = m["response_items"].as_array() {
                    input.extend(items.clone());
                } else {
                    if let Some(text) = m["content"].as_str().filter(|s| !s.is_empty()) {
                        input.push(json!({"role": "assistant", "content": text}));
                    }
                    for call in m["tool_calls"].as_array().into_iter().flatten() {
                        input.push(json!({"type": "function_call", "call_id": call["id"],
                            "namespace": "quantrig", "name": call["function"]["name"],
                            "arguments": history_arguments(&call["function"]["arguments"])}));
                    }
                }
            }
            Some("user") => input.push(json!({"role": "user", "content": m["content"]})),
            _ => {},
        }
    }
    input
}
fn response_body(model: &str, messages: &[Value]) -> Value {
    let functions: Vec<Value> = tools().as_array().unwrap().iter().map(|tool| {
        let mut f = tool["function"].clone();
        f["type"] = json!("function"); f["strict"] = json!(false); f
    }).collect();
    json!({"model": model, "instructions": SYSTEM, "input": response_input(messages),
        "store": false, "stream": true, "include": ["reasoning.encrypted_content"],
        "tools": [{"type": "namespace", "name": "quantrig", "description": "Saved strategies, market datasets, and sandboxed backtests", "tools": functions}]})
}
fn response_error(error: &Value) -> String {
    match error["code"].as_str().unwrap_or("unknown_error") {
        "subscription_sharing_usage_limit_exceeded" => "ChatGPT plan usage limit reached. Check usage in ChatGPT Settings or choose another provider.".into(),
        "subscription_sharing_usage_unavailable" => "ChatGPT plan usage is unavailable for this account. Check plan permissions in ChatGPT Settings.".into(),
        "subscription_sharing_unsupported_capability" => "ChatGPT does not support a requested capability for this model or plan.".into(),
        "invalid_api_key" | "token_expired" => "ChatGPT session is no longer valid. Sign in again in Settings.".into(),
        _ => format!("ChatGPT request failed: {}", error["message"].as_str().unwrap_or("unknown error")),
    }
}
/// Only the terminal completed event authorizes executing tool calls.
async fn response_event(acc: &mut Accumulator, chunk: &Value, out: &tokio::sync::mpsc::Sender<Value>) -> Result<bool, Box<dyn Error + Send + Sync>> {
    match chunk["type"].as_str().unwrap_or("") {
        "response.output_text.delta" | "response.reasoning_summary_text.delta" => {
            let text = chunk["delta"].as_str().unwrap_or("");
            let kind = if chunk["type"] == "response.output_text.delta" { acc.content.push_str(text); "text" } else { "reasoning" };
            out.send(event(kind, json!({"delta": text}))).await.map_err(|_| "Chat reader disconnected")?;
        }
        "response.output_item.done" => {
            let index = chunk["output_index"].as_u64().ok_or("ChatGPT output item had no index")?;
            let item = chunk["item"].as_object().ok_or("ChatGPT output item was invalid")?;
            acc.streamed_items.insert(index, Value::Object(item.clone()));
        }
        "response.completed" => {
            if chunk["response"]["status"] != "completed" { return Err("ChatGPT response did not complete".into()); }
            // Some subscription streams send completed items separately and omit
            // them from the terminal response. Preserve those items in stream order.
            acc.response_items = match chunk["response"]["output"].as_array() {
                Some(items) if !items.is_empty() => items.clone(),
                _ => acc.streamed_items.values().cloned().collect(),
            };
            for item in &mut acc.response_items {
                if item["type"] == "function_call" {
                    if item["namespace"].as_str().is_some_and(|s| s != "quantrig") { return Err("ChatGPT returned an unknown tool namespace".into()); }
                    let id = item["call_id"].as_str().ok_or("ChatGPT tool call had no ID")?.to_string();
                    let name = item["name"].as_str().ok_or("ChatGPT tool call had no name")?.to_string();
                    let raw = item["arguments"].as_str().ok_or("ChatGPT tool call had no arguments")?.to_string();
                    acc.calls.push((id, name, raw.clone()));
                    item["arguments"] = json!(history_arguments(&json!(raw)));
                }
            }
            if acc.content.is_empty() {
                let text: String = acc.response_items.iter().filter(|item| item["type"] == "message")
                    .flat_map(|item| item["content"].as_array().into_iter().flatten())
                    .filter_map(|part| part["text"].as_str().or_else(|| part["refusal"].as_str())).collect();
                if !text.is_empty() {
                    acc.content = text.clone();
                    out.send(event("text", json!({"delta": text}))).await.map_err(|_| "Chat reader disconnected")?;
                }
            }
            if acc.content.is_empty() && acc.calls.is_empty() {
                return Err("ChatGPT completed without an answer or a tool call. Please retry.".into());
            }
            return Ok(true);
        }
        "response.failed" => return Err(response_error(&chunk["response"]["error"]).into()),
        "response.incomplete" => return Err("ChatGPT response was incomplete. Try again.".into()),
        "error" => return Err(response_error(chunk).into()),
        _ => {},
    }
    Ok(false)
}
async fn responses_once(http: &reqwest::Client, model: &str, messages: &[Value], out: &tokio::sync::mpsc::Sender<Value>) -> Result<Accumulator, Box<dyn Error + Send + Sync>> {
    let key = chatgpt::access_token().await?;
    let res = http.post("https://api.openai.com/v1/responses").bearer_auth(&key)
        .json(&response_body(model, messages)).send().await?;
    if !res.status().is_success() {
        let status = res.status();
        if status == reqwest::StatusCode::UNAUTHORIZED { chatgpt::invalidate(&key).await; }
        let body = res.json::<Value>().await.unwrap_or_else(|_| json!({}));
        return Err(format!("ChatGPT {status}: {}", response_error(&body["error"])).into());
    }
    read_response(res, out).await
}
async fn read_response(mut res: reqwest::Response, out: &tokio::sync::mpsc::Sender<Value>) -> Result<Accumulator, Box<dyn Error + Send + Sync>> {
    let mut acc = Accumulator::default();
    let mut buffer = Vec::new();
    let mut data = String::new();
    while let Some(bytes) = res.chunk().await? {
        buffer.extend_from_slice(&bytes);
        while let Some(nl) = buffer.iter().position(|b| *b == b'\n') {
            let line = std::str::from_utf8(&buffer[..nl])?.trim_end_matches('\r').to_string();
            buffer.drain(..=nl);
            if line.is_empty() {
                if !data.is_empty() {
                    let chunk: Value = serde_json::from_str(data.trim())?;
                    data.clear();
                    if response_event(&mut acc, &chunk, out).await? { return Ok(acc); }
                }
            } else if let Some(payload) = line.strip_prefix("data:") {
                if !data.is_empty() { data.push('\n'); }
                data.push_str(payload.trim_start());
            }
        }
    }
    Err("ChatGPT stream ended before response.completed. No tools were executed.".into())
}

/// One chat turn, streamed: ask the model, run any tools it calls, repeat until
/// it answers. Events go to `out`; the browser rebuilds the transcript from them.
pub async fn turn(
    provider: String,
    model: String,
    session: String,
    history: Vec<Value>,
    out: tokio::sync::mpsc::Sender<Value>,
) {
    let key = if provider == "chatgpt" { String::new() } else {
        match store::opencode_key() {
            Some(key) => key,
            None => { let _ = out.send(event("error", json!({"error": "Add an OpenCode Go key in Settings."}))).await; return; }
        }
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
        if out.is_closed() { return; }
        let result = if provider == "chatgpt" {
            responses_once(&http, &model, &messages, &out).await
        } else { stream_once(&http, &key, &model, &session, &messages, &out).await };
        let acc = match result {
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
    fn stateless_responses_preserve_reasoning_tool_calls_and_outputs() {
        let reasoning = json!({"type": "reasoning", "id": "rs_test", "summary": [], "encrypted_content": "opaque"});
        let call = json!({"type": "function_call", "namespace": "quantrig", "call_id": "call_test", "name": "list_datasets", "arguments": "{}"});
        let messages = vec![json!({"role": "system", "content": SYSTEM}), json!({"role": "user", "content": "List datasets"}),
            json!({"role": "assistant", "content": "", "response_items": [reasoning.clone(), call.clone()]}),
            json!({"role": "tool", "tool_call_id": "call_test", "content": "[\"EUR_USD@1h\"]"})];
        let body = response_body("account-model", &messages);
        assert_eq!(body["store"], false); assert_eq!(body["stream"], true);
        assert_eq!(body["input"][1], reasoning); assert_eq!(body["input"][2], call);
        assert_eq!(body["input"][3]["type"], "function_call_output");
        assert_eq!(body["tools"][0]["type"], "namespace");
        assert_eq!(body["tools"][0]["tools"].as_array().unwrap().len(), 7);
        for forbidden in ["previous_response_id", "temperature", "max_output_tokens", "conversation"] { assert!(body.get(forbidden).is_none()); }
        let legacy = response_input(&[json!({"role": "assistant", "tool_calls": [{"id": "old-call", "function": {"name": "list_datasets", "arguments": "{bad"}}]})]);
        assert_eq!(legacy[0]["arguments"], "{}");
    }
    #[tokio::test]
    async fn only_completed_responses_release_calls_and_preserve_raw_validation() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let mut acc = Accumulator::default();
        assert!(!response_event(&mut acc, &json!({"type": "response.output_text.delta", "delta": "Hello 🌍"}), &tx).await.unwrap());
        assert_eq!(rx.recv().await.unwrap()["delta"], "Hello 🌍");
        assert!(!response_event(&mut acc, &json!({"type": "response.function_call_arguments.done", "arguments": "{}"}), &tx).await.unwrap());
        assert!(acc.calls.is_empty());
        let completed = json!({"type": "response.completed", "response": {"status": "completed", "output": [{"type": "function_call", "namespace": "quantrig", "call_id": "call_test", "name": "list_datasets", "arguments": "{bad"}]}});
        assert!(response_event(&mut acc, &completed, &tx).await.unwrap());
        assert_eq!(acc.calls[0].2, "{bad");
        assert_eq!(acc.message()["response_items"][0]["arguments"], "{}");
        for failed in [json!({"type": "response.failed", "response": {"error": {"code": "subscription_sharing_usage_limit_exceeded"}}}),
            json!({"type": "response.incomplete"}), json!({"type": "error", "message": "failed"})] {
            assert!(response_event(&mut Accumulator::default(), &failed, &tx).await.is_err());
        }
    }
    #[tokio::test]
    async fn completed_output_items_survive_an_empty_terminal_output() {
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        let mut acc = Accumulator::default();
        let reasoning = json!({"type": "reasoning", "id": "rs_test", "summary": [], "encrypted_content": "opaque"});
        let call = json!({"type": "function_call", "namespace": "quantrig", "call_id": "call_test", "name": "list_datasets", "arguments": "{}"});
        // Completion items may precede the terminal event, and must not execute early.
        for (index, item) in [(1, call.clone()), (0, reasoning.clone())] {
            assert!(!response_event(&mut acc, &json!({"type": "response.output_item.done", "output_index": index, "item": item}), &tx).await.unwrap());
            assert!(acc.calls.is_empty());
        }
        assert!(response_event(&mut acc, &json!({"type": "response.completed", "response": {"status": "completed", "output": []}}), &tx).await.unwrap());
        assert_eq!(acc.calls, vec![("call_test".into(), "list_datasets".into(), "{}".into())]);
        assert_eq!(acc.response_items, vec![reasoning.clone(), call.clone()]);
        let history = vec![acc.message(), json!({"role": "tool", "tool_call_id": "call_test", "content": "[]"})];
        assert_eq!(response_input(&history), vec![reasoning, call, json!({"type": "function_call_output", "call_id": "call_test", "output": "[]"})]);
        // A full terminal output must not duplicate the previously streamed call.
        let mut acc = Accumulator::default();
        let item = json!({"type": "function_call", "call_id": "call_two", "name": "list_datasets", "arguments": "{}"});
        response_event(&mut acc, &json!({"type": "response.output_item.done", "output_index": 0, "item": item}), &tx).await.unwrap();
        response_event(&mut acc, &json!({"type": "response.completed", "response": {"status": "completed", "output": [item]}}), &tx).await.unwrap();
        assert_eq!(acc.calls.len(), 1);
    }

    #[tokio::test]
    #[ignore = "requires loopback socket permission"]
    async fn responses_transport_handles_split_utf8_and_rejects_interrupted_streams() {
        use axum::{body::Body, routing::get, Router};
        async fn mocked(stream: String, tx: &tokio::sync::mpsc::Sender<Value>) -> Result<Accumulator, Box<dyn Error + Send + Sync>> {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
            let addr = listener.local_addr()?;
            let bytes: Vec<Vec<u8>> = stream.into_bytes().into_iter().map(|b| vec![b]).collect();
            let app = Router::new().route("/", get(move || { let bytes = bytes.clone(); async move { Body::from_stream(tokio_stream::iter(bytes.into_iter().map(Ok::<_, std::io::Error>))) } }));
            let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
            let result = read_response(reqwest::get(format!("http://{addr}/")).await?, tx).await;
            server.abort(); result
        }
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let stream = format!("data: {}\r\n\r\ndata: {}\n\n", json!({"type": "response.output_text.delta", "delta": "hello 🌍"}), json!({"type": "response.completed", "response": {"status": "completed", "output": []}}));
        let acc = mocked(stream, &tx).await.unwrap(); assert_eq!(acc.content, "hello 🌍"); assert_eq!(rx.recv().await.unwrap()["delta"], "hello 🌍");
        assert!(mocked(format!("data: {}\n\n", json!({"type": "response.output_item.added", "item": {"type": "function_call"}})), &tx).await.is_err());
        assert!(mocked(format!("data: {}\n\n", json!({"type": "response.failed", "response": {"error": {"code": "subscription_sharing_usage_limit_exceeded"}}})), &tx).await.err().unwrap().to_string().contains("usage limit"));
    }

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
    fn tools_read_saved_backtests_without_rerunning() {
        let (_guard, dir) = scratch("history");
        std::fs::create_dir_all(dir.join("backtests")).unwrap();
        let record = json!({
            "id": "123", "project": "Martingale", "strategy": "gold",
            "dataset": "XAU_USD@15m", "created": 123, "source": "manual",
            "code": "original strategy", "config": {"cash": 10000.0, "spread": 0.1, "commission": 0.0},
            "stats": {"total_return_pct": 12.5}, "error": null, "report_available": false
        });
        std::fs::write(dir.join("backtests/123.json"), record.to_string()).unwrap();
        let list: Value = serde_json::from_str(&call_tool("list_backtests", &json!({}))).unwrap();
        assert_eq!(list[0]["id"], "123");
        assert_eq!(list[0]["project"], "Martingale");
        assert_eq!(list[0]["return_pct"], 12.5);
        let read: Value = serde_json::from_str(&call_tool("read_backtest", &json!({"id": "123"}))).unwrap();
        assert_eq!(read["stats"]["total_return_pct"], 12.5);
        assert_eq!(read["stats"].as_object().unwrap().len(), 10);
        assert!(read["stats"]["final_cash"].is_null());
        assert!(read.get("code").is_none());
        assert!(read.get("config").is_none());
        for code in ["no", "YES", "true"] {
            let read: Value = serde_json::from_str(&call_tool("read_backtest", &json!({"id": "123", "code": code}))).unwrap();
            assert!(read.get("code").is_none());
        }
        let with_code: Value = serde_json::from_str(&call_tool("read_backtest", &json!({"id": "123", "code": "yes"}))).unwrap();
        assert_eq!(with_code["code"], record["code"]);
        assert_eq!(with_code["stats"], read["stats"]);
        assert!(!store::runs_dir().exists());
        std::fs::remove_dir_all(&dir).unwrap();
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
            ("read_backtest", json!({"id": "../escape"})),
            ("read_backtest", json!({"id": "123"})),
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
            ["list_strategies", "read_strategy", "write_strategy", "list_datasets", "list_backtests", "read_backtest", "run_backtest"]
        );
    }
}
