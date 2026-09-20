//! Where the API keeps its own state: settings and downloaded candles.
//!
//! Nothing here is ever bound into the sandbox.

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

fn settings_path() -> PathBuf {
    data_dir().join("settings.json")
}

/// The LSE key, or None if it has never been set.
///
/// ponytail: a 0600 file, the same protection a .env gets. Envelope encryption
/// (DECISIONS.md) lands when this stops being a loopback-only single-user box.
pub fn api_key() -> Option<String> {
    let raw = std::fs::read_to_string(settings_path()).ok()?;
    let key = raw
        .split("\"lse_api_key\"")
        .nth(1)?
        .split('"')
        .nth(1)?
        .to_string();
    (!key.is_empty()).then_some(key)
}

pub fn set_api_key(key: &str) -> Result<(), Box<dyn Error>> {
    if key.contains(['"', '\\']) {
        return Err("api key contains quoting characters".into());
    }
    std::fs::create_dir_all(data_dir())?;
    let path = settings_path();
    std::fs::write(&path, format!("{{\"lse_api_key\": \"{key}\"}}\n"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// A downloaded dataset, named `EUR_USD@1h`. The name is also the file stem, so
/// it has to stay free of separators — it arrives from the browser.
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
}
