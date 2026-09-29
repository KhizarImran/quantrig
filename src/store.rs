//! Where the API keeps its own state: settings, strategies, downloaded candles,
//! chat conversations.
//!
//! Nothing here is ever bound into the sandbox.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::error::Error;
use std::path::PathBuf;

pub fn data_dir() -> PathBuf {
    std::env::var("QUANTRIG_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("data"))
}

pub fn runs_dir() -> PathBuf {
    data_dir().join("runs")
}

pub fn candles_dir() -> PathBuf {
    data_dir().join("candles")
}

pub fn strategies_dir() -> PathBuf {
    data_dir().join("strategies")
}

pub fn conversations_dir() -> PathBuf {
    data_dir().join("conversations")
}

fn settings_path() -> PathBuf {
    data_dir().join("settings.json")
}

/// Credentials, stored 0600 on this machine.
///
/// ponytail: a 0600 file, the same protection a .env gets. Envelope encryption
/// (DECISIONS.md) lands when this stops being a loopback-only single-user box.
#[derive(Default, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lse_api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opencode_api_key: Option<String>,
}

pub fn settings() -> Settings {
    std::fs::read_to_string(settings_path())
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub fn lse_key() -> Option<String> {
    settings().lse_api_key.filter(|k| !k.is_empty())
}

pub fn opencode_key() -> Option<String> {
    settings().opencode_api_key.filter(|k| !k.is_empty())
}

/// Writes one key without disturbing the others.
pub fn set_key(which: &str, key: &str) -> Result<(), Box<dyn Error>> {
    let mut current = settings();
    match which {
        "lse_api_key" => current.lse_api_key = Some(key.to_string()),
        "opencode_api_key" => current.opencode_api_key = Some(key.to_string()),
        other => return Err(format!("unknown setting {other}").into()),
    }
    std::fs::create_dir_all(data_dir())?;
    let path = settings_path();
    std::fs::write(&path, serde_json::to_string_pretty(&current)?)?;
    // Best effort: a bind mount from a non-POSIX host may not support it.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// A downloaded dataset, named `EUR_USD@1h`. The name is also the file stem, so
/// it has to stay free of separators — it arrives from the browser or an agent.
pub fn dataset_name(symbol: &str, timeframe: &str) -> String {
    format!("{}@{}", symbol.replace('/', "_"), timeframe)
}

pub fn is_safe_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() < 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '@' | '-' | '.'))
        && !name.contains("..")
}

pub fn dataset_path(name: &str) -> Option<PathBuf> {
    is_safe_name(name).then(|| candles_dir().join(format!("{name}.parquet")))
}

/// Strategy files the agent writes. Same name rules — an LLM picks these.
pub fn strategy_path(name: &str) -> Option<PathBuf> {
    let stem = name.strip_suffix(".py").unwrap_or(name);
    is_safe_name(stem).then(|| strategies_dir().join(format!("{stem}.py")))
}

pub fn list_strategies() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(strategies_dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "py"))
        .filter_map(|e| e.path().file_stem().map(|s| s.to_string_lossy().to_string()))
        .collect();
    names.sort();
    names
}

/// One chat, keyed by the browser's session id (a UUID, which passes the same
/// name rules). The transcript is the browser's to shape, so it is kept opaque.
pub fn conversation_path(id: &str) -> Option<PathBuf> {
    is_safe_name(id).then(|| conversations_dir().join(format!("{id}.json")))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Writes a conversation, stamping its id and time. Returns what was stored.
pub fn save_conversation(id: &str, mut body: Value) -> Result<Value, Box<dyn Error>> {
    let path = conversation_path(id).ok_or("bad conversation id")?;
    let obj = body.as_object_mut().ok_or("conversation must be a JSON object")?;
    obj.insert("id".into(), json!(id));
    obj.insert("updated".into(), json!(now_ms()));
    std::fs::create_dir_all(conversations_dir())?;
    std::fs::write(path, serde_json::to_string(&body)?)?;
    Ok(body)
}

pub fn load_conversation(id: &str) -> Option<Value> {
    let raw = std::fs::read_to_string(conversation_path(id)?).ok()?;
    serde_json::from_str(&raw).ok()
}

pub fn delete_conversation(id: &str) -> Result<(), Box<dyn Error>> {
    let path = conversation_path(id).ok_or("bad conversation id")?;
    std::fs::remove_file(path)?;
    Ok(())
}

/// Summaries for the sidebar, newest first — not the transcripts.
/// ponytail: reads every file per call. Fine until someone has thousands of chats.
pub fn list_conversations() -> Vec<Value> {
    let mut out: Vec<Value> = std::fs::read_dir(conversations_dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .filter_map(|raw| serde_json::from_str::<Value>(&raw).ok())
        .map(|c| json!({ "id": c["id"], "title": c["title"], "updated": c["updated"] }))
        .collect();
    out.sort_by_key(|c| std::cmp::Reverse(c["updated"].as_u64().unwrap_or(0)));
    out
}

/// QUANTRIG_DATA is process-global, so tests that repoint it must not overlap.
#[cfg(test)]
pub(crate) static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_traversal_in_dataset_names() {
        assert!(dataset_path("../../etc/passwd").is_none());
        assert!(dataset_path("..").is_none());
        assert!(dataset_path("a/b").is_none());
        assert!(dataset_path("").is_none());
        assert!(dataset_path("EUR_USD@1h").is_some());
    }

    #[test]
    fn dataset_name_flattens_the_pair() {
        assert_eq!(dataset_name("EUR/USD", "1h"), "EUR_USD@1h");
    }

    /// An LLM picks these names, so the same rules have to hold.
    #[test]
    fn strategy_names_are_confined_to_the_strategies_dir() {
        // Reads strategies_dir(), so it must not overlap a test repointing QUANTRIG_DATA.
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        assert!(strategy_path("../../../etc/passwd").is_none());
        assert!(strategy_path("a/b.py").is_none());
        assert!(strategy_path("").is_none());
        assert_eq!(
            strategy_path("sma_cross.py").unwrap().file_name(),
            strategy_path("sma_cross").unwrap().file_name(),
            "the .py suffix is optional, not a second file"
        );
        assert!(strategy_path("sma_cross").unwrap().ends_with("sma_cross.py"));
    }

    #[test]
    fn settings_keeps_other_keys_when_one_is_written() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("qr-settings-{}", std::process::id()));
        unsafe { std::env::set_var("QUANTRIG_DATA", &dir) };
        let _ = std::fs::remove_dir_all(&dir);

        set_key("lse_api_key", "lse-1").unwrap();
        set_key("opencode_api_key", "oc-1").unwrap();
        assert_eq!(lse_key().as_deref(), Some("lse-1"), "writing one key dropped the other");
        assert_eq!(opencode_key().as_deref(), Some("oc-1"));
        assert!(set_key("nonsense", "x").is_err());

        let _ = std::fs::remove_dir_all(&dir);
        unsafe { std::env::remove_var("QUANTRIG_DATA") };
    }

    #[test]
    fn conversation_ids_are_confined_to_their_dir() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        assert!(conversation_path("../settings").is_none());
        assert!(conversation_path("a/b").is_none());
        assert!(conversation_path("").is_none());
        assert!(conversation_path("0b6c1d2e-9f1a-4c3b-8d7e-123456789abc").is_some());
        assert!(save_conversation("../x", json!({})).is_err());
        assert!(delete_conversation("..").is_err());
    }

    #[test]
    fn conversations_round_trip_and_list_newest_first() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("qr-convos-{}", std::process::id()));
        unsafe { std::env::set_var("QUANTRIG_DATA", &dir) };
        let _ = std::fs::remove_dir_all(&dir);

        assert!(list_conversations().is_empty(), "no dir yet is an empty list, not an error");
        let body = json!({"title": "first", "parts": [{"kind": "user", "text": "hi"}]});
        save_conversation("a", body).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        save_conversation("b", json!({"title": "second"})).unwrap();
        assert!(save_conversation("c", json!([1, 2])).is_err(), "only objects are stored");

        let loaded = load_conversation("a").unwrap();
        assert_eq!(loaded["id"], "a");
        assert_eq!(loaded["parts"][0]["text"], "hi");
        assert!(loaded["updated"].as_u64().unwrap() > 0);

        let list = list_conversations();
        let ids: Vec<_> = list.iter().map(|c| c["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["b", "a"]);
        assert!(list[0].get("parts").is_none(), "the list carries summaries only");

        delete_conversation("a").unwrap();
        assert!(load_conversation("a").is_none());
        assert!(delete_conversation("a").is_err());

        let _ = std::fs::remove_dir_all(&dir);
        unsafe { std::env::remove_var("QUANTRIG_DATA") };
    }
}
