//! quantrig API. Everything is a client of this — the browser, the MCP server,
//! a future TUI. Nothing else touches the engine.

mod agent;
mod fetcher;
mod sandbox;
mod store;

use axum::extract::Path as UrlPath;
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::Html;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use std::convert::Infallible;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt;
use tower_http::services::{ServeDir, ServeFile};

/// Built SPA. Empty in dev — `npm run dev` serves it and proxies the API here.
fn ui_dir() -> std::path::PathBuf {
    std::env::var("QUANTRIG_UI")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("ui/dist"))
}

type ApiError = (StatusCode, Json<Value>);

fn bad(status: StatusCode, msg: impl ToString) -> ApiError {
    (status, Json(json!({ "error": msg.to_string() })))
}

// ---- settings ----

#[derive(Deserialize)]
struct KeyRequest {
    #[serde(default)]
    lse_api_key: Option<String>,
    #[serde(default)]
    opencode_api_key: Option<String>,
}

/// Which keys are set — never the keys themselves. They only travel inward.
async fn get_settings() -> Json<Value> {
    Json(json!({
        "lse_api_key_set": store::lse_key().is_some(),
        "opencode_api_key_set": store::opencode_key().is_some(),
    }))
}

async fn put_settings(Json(req): Json<KeyRequest>) -> Result<Json<Value>, ApiError> {
    for (name, value) in [
        ("lse_api_key", req.lse_api_key),
        ("opencode_api_key", req.opencode_api_key),
    ] {
        let Some(key) = value else { continue };
        let key = key.trim();
        if key.is_empty() {
            return Err(bad(StatusCode::BAD_REQUEST, format!("{name} is empty")));
        }
        store::set_key(name, key).map_err(|e| bad(StatusCode::BAD_REQUEST, e))?;
    }
    Ok(get_settings().await.0).map(Json)
}

// ---- market data ----

/// 409 rather than 502: a missing key is the operator's to fix, not the vault's.
fn require_key() -> Result<(), ApiError> {
    store::lse_key()
        .map(|_| ())
        .ok_or_else(|| bad(StatusCode::CONFLICT, "no London Strategic Edge API key set — add one in Settings"))
}

async fn pairs() -> Result<Json<Value>, ApiError> {
    require_key()?;
    let raw = tokio::task::spawn_blocking(|| fetcher::catalog().map_err(|e| e.to_string()))
        .await
        .map_err(|e| bad(StatusCode::INTERNAL_SERVER_ERROR, e))?
        .map_err(|e| bad(StatusCode::BAD_GATEWAY, e))?;
    Ok(Json(serde_json::from_str(&raw).map_err(|e| bad(StatusCode::BAD_GATEWAY, e))?))
}

/// Datasets already on disk, newest first.
async fn list_datasets() -> Json<Value> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(store::candles_dir()) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "parquet") {
                continue;
            }
            let name = path.file_stem().unwrap().to_string_lossy().to_string();
            let meta = std::fs::read_to_string(path.with_extension("json"))
                .ok()
                .and_then(|s| serde_json::from_str::<Value>(&s).ok())
                .unwrap_or_else(|| json!({}));
            out.push(json!({
                "name": name,
                "bytes": entry.metadata().map(|m| m.len()).unwrap_or(0),
                "rows": meta.get("rows").cloned().unwrap_or(Value::Null),
                "start": meta.get("start").cloned().unwrap_or(Value::Null),
                "end": meta.get("end").cloned().unwrap_or(Value::Null),
            }));
        }
    }
    Json(Value::Array(out))
}

#[derive(Deserialize)]
struct DownloadRequest {
    symbol: String,
    timeframe: String,
    #[serde(default)]
    start: String,
    #[serde(default)]
    end: String,
}

async fn download(Json(req): Json<DownloadRequest>) -> Result<Json<Value>, ApiError> {
    require_key()?;
    let name = store::dataset_name(&req.symbol, &req.timeframe);
    let path = store::dataset_path(&name)
        .ok_or_else(|| bad(StatusCode::BAD_REQUEST, "bad symbol or timeframe"))?;
    std::fs::create_dir_all(store::candles_dir())
        .map_err(|e| bad(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let summary = {
        let path = path.clone();
        tokio::task::spawn_blocking(move || {
            fetcher::download(&req.symbol, &req.timeframe, &req.start, &req.end, &path)
                .map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| bad(StatusCode::INTERNAL_SERVER_ERROR, e))?
        .map_err(|e| bad(StatusCode::BAD_GATEWAY, e))?
    };

    // Sidecar so the dataset list can show span and row count without opening parquet.
    let _ = std::fs::write(path.with_extension("json"), &summary);
    let summary: Value =
        serde_json::from_str(&summary).map_err(|e| bad(StatusCode::BAD_GATEWAY, e))?;
    Ok(Json(json!({ "name": name, "summary": summary })))
}

// ---- backtests ----

#[derive(Deserialize)]
struct RunRequest {
    code: String,
    dataset: String,
    cash: f64,
    spread: f64,
    commission: f64,
}

async fn run(Json(req): Json<RunRequest>) -> Result<Json<Value>, ApiError> {
    let candles = store::dataset_path(&req.dataset)
        .filter(|p| p.exists())
        .ok_or_else(|| bad(StatusCode::BAD_REQUEST, "unknown dataset — download it first"))?;

    let id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
        .to_string();
    let dir = store::runs_dir().join(&id);
    std::fs::create_dir_all(&dir).map_err(|e| bad(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let strategy = dir.join("strategy.py");
    std::fs::write(&strategy, &req.code).map_err(|e| bad(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let config = json!({
        "cash": req.cash, "spread": req.spread, "commission": req.commission, "plot": true
    })
    .to_string();

    // Blocking: bwrap + a full backtest. ponytail: one run at a time is fine until
    // someone complains — the queue lands with concurrent users.
    let stats = tokio::task::spawn_blocking(move || {
        // Box<dyn Error> isn't Send; the message is all the caller needs.
        sandbox::run_backtest(&strategy, &candles, &dir, &config).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| bad(StatusCode::INTERNAL_SERVER_ERROR, e))?
    .map_err(|e| bad(StatusCode::BAD_REQUEST, e))?;

    let stats: Value = serde_json::from_str(&stats).map_err(|e| bad(StatusCode::BAD_GATEWAY, e))?;
    Ok(Json(json!({ "id": id, "stats": stats })))
}

// ---- strategies ----

#[derive(Deserialize)]
struct StrategyRequest {
    name: String,
    code: String,
}

async fn list_strategies() -> Json<Value> {
    Json(json!(store::list_strategies()))
}

async fn get_strategy(UrlPath(name): UrlPath<String>) -> Result<Json<Value>, ApiError> {
    let path = store::strategy_path(&name)
        .ok_or_else(|| bad(StatusCode::BAD_REQUEST, "bad strategy name"))?;
    let code = std::fs::read_to_string(path).map_err(|e| bad(StatusCode::NOT_FOUND, e))?;
    Ok(Json(json!({ "name": name, "code": code })))
}

async fn put_strategy(Json(req): Json<StrategyRequest>) -> Result<Json<Value>, ApiError> {
    let path = store::strategy_path(&req.name)
        .ok_or_else(|| bad(StatusCode::BAD_REQUEST, "bad strategy name"))?;
    std::fs::create_dir_all(store::strategies_dir())
        .and_then(|_| std::fs::write(&path, &req.code))
        .map_err(|e| bad(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(json!({ "name": req.name })))
}

// ---- agent ----

#[derive(Deserialize)]
struct ChatRequest {
    model: String,
    /// Stable for the life of one conversation — Go routes and caches on it.
    session: String,
    messages: Vec<Value>,
}

/// Server-sent events for one turn: text and reasoning deltas, tool calls and
/// their results, then done. The tool loop runs here, not in the browser.
async fn chat(
    Json(req): Json<ChatRequest>,
) -> Result<Sse<impl futures_core::Stream<Item = Result<Event, Infallible>>>, ApiError> {
    if store::opencode_key().is_none() {
        return Err(bad(
            StatusCode::CONFLICT,
            "no OpenCode Go API key set — add one in Settings",
        ));
    }
    // Bounded: a slow reader should slow the turn, not grow memory without limit.
    let (tx, rx) = tokio::sync::mpsc::channel(256);
    tokio::spawn(agent::turn(req.model, req.session, req.messages, tx));

    let stream = ReceiverStream::new(rx)
        .map(|event| Ok(Event::default().data(event.to_string())));
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

async fn models() -> Result<Json<Value>, ApiError> {
    agent::models()
        .await
        .map(Json)
        .map_err(|e| bad(StatusCode::BAD_GATEWAY, e))
}

async fn report(UrlPath(id): UrlPath<String>) -> Result<Html<String>, ApiError> {
    // Run ids are our own nanosecond timestamps; anything else is a traversal attempt.
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
        return Err(bad(StatusCode::BAD_REQUEST, "bad run id"));
    }
    std::fs::read_to_string(store::runs_dir().join(id).join("report.html"))
        .map(Html)
        .map_err(|e| bad(StatusCode::NOT_FOUND, e))
}

#[tokio::main]
async fn main() {
    // Client routes fall through to index.html; the API owns /api and /report.
    let spa = ServeDir::new(ui_dir()).fallback(ServeFile::new(ui_dir().join("index.html")));
    let app = Router::new()
        .route("/api/settings", get(get_settings).put(put_settings))
        .route("/api/pairs", get(pairs))
        .route("/api/datasets", get(list_datasets).post(download))
        .route("/api/run", post(run))
        .route("/api/strategies", get(list_strategies).put(put_strategy))
        .route("/api/strategies/{name}", get(get_strategy))
        .route("/api/models", get(models))
        .route("/api/chat", post(chat))
        .route("/report/{id}", get(report))
        .fallback_service(spa);

    // ponytail: loopback only, no auth yet. Password gate before this leaves localhost.
    let addr = std::env::var("QUANTRIG_ADDR").unwrap_or_else(|_| "127.0.0.1:9000".into());
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    println!("quantrig on http://{addr}");
    axum::serve(listener, app).await.unwrap();
}
