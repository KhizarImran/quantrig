//! Calls London Strategic Edge. This side of the boundary holds the API key and
//! is allowed on the network — which is exactly why no strategy code runs here.

use crate::store;
use std::error::Error;
use std::path::Path;
use std::process::Command;

fn root() -> std::path::PathBuf {
    std::env::var("QUANTRIG_ROOT")
        .unwrap_or_else(|_| env!("CARGO_MANIFEST_DIR").into())
        .into()
}

fn run(args: &[&str]) -> Result<String, Box<dyn Error>> {
    let key = store::lse_key().ok_or("no London Strategic Edge API key set — add one in Settings")?;
    let python = format!("{}/bin/python3", crate::sandbox::python_prefix());
    let out = Command::new(python)
        .arg(root().join("fetcher/fetch.py"))
        .args(args)
        .env("LSE_API_KEY", key)
        .output()?;
    if !out.status.success() {
        // Log the whole thing: with retries the last line hides the earlier
        // attempts, and `docker compose logs` is where this gets diagnosed.
        let err = String::from_utf8_lossy(&out.stderr);
        eprintln!("fetcher {args:?} failed:\n{err}");
        let summary: Vec<&str> = err.lines().rev().take(3).collect();
        return Err(summary.into_iter().rev().collect::<Vec<_>>().join(" / ").into());
    }
    Ok(String::from_utf8(out.stdout)?)
}

/// Every FX pair the vault carries, as JSON.
///
/// The upstream catalog is a multi-megabyte single GET that truncates often, so
/// the answer is cached on disk. The pair list changes about never.
pub fn catalog() -> Result<String, Box<dyn Error>> {
    let cache = store::data_dir().join("pairs.json");
    if let Ok(cached) = std::fs::read_to_string(&cache) {
        if cached.len() > 2 {
            return Ok(cached);
        }
    }
    std::fs::create_dir_all(store::data_dir())?;
    run(&["catalog", cache.to_str().ok_or("bad data dir")?])
}

/// Downloads candles to `out`, returning a JSON summary (rows, start, end).
pub fn download(
    symbol: &str,
    timeframe: &str,
    start: &str,
    end: &str,
    out: &Path,
) -> Result<String, Box<dyn Error>> {
    run(&["download", symbol, timeframe, start, end, out.to_str().unwrap()])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs the fetcher's own check, so the retry logic can't rot silently.
    #[test]
    fn fetcher_recovers_from_truncated_reads() {
        let out = Command::new(format!("{}/bin/python3", crate::sandbox::python_prefix()))
            .arg(root().join("fetcher/fetch.py"))
            .arg("selftest")
            .output()
            .expect("run fetch.py");
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    }
}
