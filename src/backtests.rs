//! Durable run records live outside the directory writable by strategy code.
use crate::{sandbox, store};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{error::Error, path::PathBuf};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Clone, Serialize, Deserialize)]
pub struct Config {
    pub cash: f64,
    pub spread: f64,
    pub commission: f64,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Run {
    pub id: String,
    pub project: String,
    pub strategy: String,
    pub dataset: String,
    pub created: u64,
    pub source: String,
    pub code: String,
    pub config: Option<Config>,
    pub stats: Option<Value>,
    pub error: Option<String>,
    pub report_available: bool,
}

fn history_dir() -> PathBuf { store::data_dir().join("backtests") }

pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 32 && id.bytes().all(|b| b.is_ascii_digit())
}

pub fn project_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.chars().count() > 80 || name.chars().any(char::is_control) {
        return Err("project must be at most 80 characters without control characters".into());
    }
    Ok(if name.is_empty() { "Ungrouped".into() } else { name.into() })
}

fn save(run: &Run) -> Result<()> {
    if !valid_id(&run.id) { return Err("bad run id".into()); }
    std::fs::create_dir_all(history_dir())?;
    let path = history_dir().join(format!("{}.json", run.id));
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, serde_json::to_vec(run)?)?;
    std::fs::rename(temp, path)?;
    Ok(())
}

pub fn load(id: &str) -> Result<Run> {
    if !valid_id(id) { return Err("bad run id".into()); }
    let path = history_dir().join(format!("{id}.json"));
    if path.exists() {
        return Ok(serde_json::from_slice(&std::fs::read(path)?)?);
    }
    // Older manual runs retained source/report but never saved settings or stats.
    let dir = store::runs_dir().join(id);
    if !dir.join("report.html").is_file() { return Err("no such backtest".into()); }
    Ok(Run {
        id: id.into(), project: "Ungrouped".into(), strategy: "Archived strategy".into(),
        dataset: String::new(), created: id.parse::<u128>().unwrap_or(0).checked_div(1_000_000).unwrap_or(0) as u64,
        source: "legacy".into(), code: std::fs::read_to_string(dir.join("strategy.py"))?,
        config: None, stats: None, error: None, report_available: true,
    })
}

pub fn list() -> Vec<Value> {
    let mut ids = std::collections::BTreeSet::new();
    for entry in std::fs::read_dir(history_dir()).into_iter().flatten().flatten() {
        if entry.path().extension().is_some_and(|e| e == "json") {
            if let Some(id) = entry.path().file_stem().and_then(|s| s.to_str()) { ids.insert(id.to_string()); }
        }
    }
    for entry in std::fs::read_dir(store::runs_dir()).into_iter().flatten().flatten() {
        let id = entry.file_name().to_string_lossy().to_string();
        if valid_id(&id) && entry.path().join("report.html").is_file() { ids.insert(id); }
    }
    let mut runs: Vec<_> = ids.into_iter().filter_map(|id| load(&id).ok()).map(|run| json!({
        "id": run.id, "project": run.project, "strategy": run.strategy,
        "dataset": run.dataset, "created": run.created, "source": run.source,
        "status": if run.error.is_some() { "failed" } else { "completed" },
        "return_pct": run.stats.as_ref().and_then(|s| s.get("total_return_pct")),
    })).collect();
    runs.sort_by_key(|run| std::cmp::Reverse(run["created"].as_u64().unwrap_or(0)));
    runs
}

pub fn move_project(id: &str, project: &str) -> Result<Run> {
    let mut run = load(id)?;
    run.project = project_name(project)?;
    save(&run)?;
    Ok(run)
}

/// Remove both the record and its artifacts so legacy discovery cannot bring it back.
pub fn delete(id: &str) -> Result<()> {
    load(id)?; // Validate the id and require an existing saved/legacy run.
    let dir = store::runs_dir().join(id);
    match std::fs::remove_dir_all(dir) {
        Ok(()) => (),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(e) => return Err(e.into()),
    }
    match std::fs::remove_file(history_dir().join(format!("{id}.json"))) {
        Ok(()) => (),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(e) => return Err(e.into()),
    }
    Ok(())
}

pub fn delete_many(ids: Vec<String>) -> Result<Value> {
    // Use the exact IDs confirmed by the user; concurrent new runs stay intact.
    if ids.is_empty() || ids.iter().any(|id| !valid_id(id)) {
        return Err("provide valid run ids".into());
    }
    let mut deleted = Vec::new();
    let mut failed = Vec::new();
    for id in ids.into_iter().collect::<std::collections::BTreeSet<_>>() {
        match delete(&id) {
            Ok(()) => deleted.push(id),
            Err(e) => failed.push(json!({"id": id, "error": e.to_string()})),
        }
    }
    Ok(json!({"deleted": deleted, "failed": failed}))
}

pub fn execute(code: String, strategy: String, dataset: String, project: String, config: Config, source: &str) -> Result<Run> {
    let project = project_name(&project)?;
    if !config.cash.is_finite() || config.cash <= 0.0 || !config.spread.is_finite()
        || config.spread < 0.0 || !config.commission.is_finite() || config.commission < 0.0 {
        return Err("cash must be positive; spread and commission must be non-negative finite numbers".into());
    }
    let candles = store::dataset_path(&dataset).filter(|p| p.is_file()).ok_or("unknown dataset — download it first")?;
    let (id, dir) = loop {
        let id = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos().to_string();
        let dir = store::runs_dir().join(&id);
        std::fs::create_dir_all(store::runs_dir())?;
        match std::fs::create_dir(&dir) {
            Ok(()) => break (id, dir),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    };
    let strategy_path = dir.join("strategy.py");
    std::fs::write(&strategy_path, &code)?;
    let runner_config = json!({"cash": config.cash, "spread": config.spread, "commission": config.commission, "plot": true});
    let outcome = sandbox::run_backtest(&strategy_path, &candles, &dir, &runner_config.to_string())
        .and_then(|raw| Ok(serde_json::from_str::<Value>(&raw)?));
    let (stats, error) = match outcome { Ok(stats) => (Some(stats), None), Err(e) => (None, Some(e.to_string())) };
    let run = Run {
        created: id.parse::<u128>()?.checked_div(1_000_000).unwrap_or(0) as u64,
        id, project, strategy: if strategy.trim().is_empty() { "Untitled strategy".into() } else { strategy },
        dataset, source: source.into(), code, config: Some(config), stats, error,
        report_available: dir.join("report.html").is_file(),
    };
    save(&run)?;
    Ok(run)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_preserves_variants_settings_failures_and_legacy_reports() {
        let _guard = store::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let root = std::env::temp_dir().join(format!("qr-history-{}", std::process::id()));
        let previous = std::env::var_os("QUANTRIG_DATA");
        unsafe { std::env::set_var("QUANTRIG_DATA", &root) };
        std::fs::create_dir_all(&root).unwrap();

        let original = Run {
            id: "1000000".into(), project: "Martingale".into(), strategy: "Martingale v1".into(),
            dataset: "EUR_USD@1h".into(), created: 1, source: "agent".into(),
            code: "# original source".into(), config: Some(Config { cash: 5000.0, spread: 0.001, commission: 0.1 }),
            stats: Some(json!({"total_return_pct": 12.5})), error: None, report_available: true,
        };
        save(&original).unwrap();
        let mut variant = original.clone();
        variant.id = "2000000".into();
        variant.created = 2;
        variant.dataset = "GBP_USD@4h".into();
        variant.code = "# independent variant".into();
        variant.stats = None;
        variant.error = Some("strategy failed".into());
        variant.report_available = false;
        save(&variant).unwrap();
        assert_eq!(load(&original.id).unwrap().code, original.code);
        assert_eq!(load(&original.id).unwrap().config.unwrap().cash, 5000.0);
        let history = list();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0]["dataset"], "GBP_USD@4h");
        assert_eq!(history[0]["status"], "failed");
        assert!(history.iter().all(|run| run["project"] == "Martingale"));
        assert!(history[0].get("code").is_none());

        let moved = move_project(&original.id, " Mean reversion ").unwrap();
        assert_eq!(moved.project, "Mean reversion");
        assert_eq!(moved.stats.unwrap()["total_return_pct"], 12.5);
        assert_eq!(load(&variant.id).unwrap().project, "Martingale");

        let legacy_dir = store::runs_dir().join("3000000");
        std::fs::create_dir_all(&legacy_dir).unwrap();
        std::fs::write(legacy_dir.join("strategy.py"), "# old source").unwrap();
        std::fs::write(legacy_dir.join("report.html"), "<html>old report</html>").unwrap();
        let legacy = load("3000000").unwrap();
        assert_eq!(legacy.project, "Ungrouped");
        assert!(legacy.stats.is_none() && legacy.config.is_none());
        assert_eq!(list().len(), 3);
        move_project("3000000", "Martingale").unwrap();
        assert_eq!(list().len(), 3, "migration must not duplicate the legacy run");
        assert_eq!(load("3000000").unwrap().project, "Martingale");
        for id in ["../settings", "agent", "", "1/2"] { assert!(load(id).is_err()); }
        assert_eq!(project_name(" ").unwrap(), "Ungrouped");
        assert!(project_name("bad\nname").is_err());
        assert!(project_name(&"x".repeat(81)).is_err());
        assert!(execute(String::new(), String::new(), "EUR_USD@1h".into(), "Martingale".into(),
            Config { cash: -1.0, spread: 0.0, commission: 0.0 }, "manual").is_err());

        assert!(delete_many(vec![original.id.clone(), "../settings".into()]).is_err());
        assert_eq!(list().len(), 3, "invalid bulk input must not delete any runs");
        std::fs::create_dir_all(store::strategies_dir()).unwrap();
        std::fs::create_dir_all(store::candles_dir()).unwrap();
        let saved_strategy = store::strategies_dir().join("martingale.py");
        let candles = store::candles_dir().join("EUR_USD@1h.parquet");
        std::fs::write(&saved_strategy, "# keep saved strategy").unwrap();
        std::fs::write(&candles, "keep market data").unwrap();
        let removed = delete_many(vec![variant.id.clone(), "3000000".into(), variant.id.clone()]).unwrap();
        assert_eq!(removed["deleted"].as_array().unwrap().len(), 2);
        assert!(removed["failed"].as_array().unwrap().is_empty());
        assert!(!legacy_dir.exists());
        assert!(load("3000000").is_err(), "deleted legacy reports must not reappear");
        assert_eq!(list().len(), 1, "other projects must remain untouched");
        assert!(saved_strategy.exists() && candles.exists());
        let original_dir = store::runs_dir().join(&original.id);
        std::fs::create_dir_all(&original_dir).unwrap();
        std::fs::write(original_dir.join("report.html"), "saved report").unwrap();
        let partial = delete_many(vec![original.id.clone(), "9999999".into()]).unwrap();
        assert_eq!(partial["deleted"], json!([original.id]));
        assert_eq!(partial["failed"].as_array().unwrap().len(), 1);
        assert!(!original_dir.exists());
        assert!(list().is_empty());

        std::fs::remove_dir_all(&root).unwrap();
        unsafe {
            match previous { Some(value) => std::env::set_var("QUANTRIG_DATA", value), None => std::env::remove_var("QUANTRIG_DATA") }
        }
    }
}
