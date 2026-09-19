//! quantrig API. Everything is a client of this — the browser, the MCP server,
//! a future TUI. Nothing else touches the engine.

mod sandbox;

use axum::extract::Path as UrlPath;
use axum::http::StatusCode;
use axum::response::Html;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::PathBuf;
use tower_http::services::{ServeDir, ServeFile};

/// Built SPA. Empty in dev — `npm run dev` serves it and proxies the API here.
fn ui_dir() -> PathBuf {
    std::env::var("QUANTRIG_UI")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("ui/dist"))
}

fn runs_dir() -> PathBuf {
    std::env::var("QUANTRIG_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("data"))
        .join("runs")
}

#[derive(Deserialize)]
struct RunRequest {
    code: String,
    candles: String,
    cash: f64,
    spread: f64,
    commission: f64,
}

type ApiError = (StatusCode, Json<Value>);

fn bad(status: StatusCode, msg: impl ToString) -> ApiError {
    (status, Json(json!({ "error": msg.to_string() })))
}

async fn run(Json(req): Json<RunRequest>) -> Result<Json<Value>, ApiError> {
    let id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
        .to_string();
    let dir = runs_dir().join(&id);
    std::fs::create_dir_all(&dir).map_err(|e| bad(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let strategy = dir.join("strategy.py");
    std::fs::write(&strategy, &req.code).map_err(|e| bad(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let candles = PathBuf::from(&req.candles);
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

async fn report(UrlPath(id): UrlPath<String>) -> Result<Html<String>, ApiError> {
    // Run ids are our own nanosecond timestamps; anything else is a traversal attempt.
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
        return Err(bad(StatusCode::BAD_REQUEST, "bad run id"));
    }
    std::fs::read_to_string(runs_dir().join(id).join("report.html"))
        .map(Html)
        .map_err(|e| bad(StatusCode::NOT_FOUND, e))
}

#[tokio::main]
async fn main() {
    // Client routes fall through to index.html; the API owns /run and /report.
    let spa = ServeDir::new(ui_dir()).fallback(ServeFile::new(ui_dir().join("index.html")));
    let app = Router::new()
        .route("/run", post(run))
        .route("/report/{id}", get(report))
        .fallback_service(spa);

    // ponytail: loopback only, no auth yet. Password gate before this leaves localhost.
    let addr = std::env::var("QUANTRIG_ADDR").unwrap_or_else(|_| "127.0.0.1:9000".into());
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    println!("quantrig on http://{addr}");
    axum::serve(listener, app).await.unwrap();
}
