//! Runs untrusted, LLM-written strategy code.
//!
//! Under bubblewrap with no network and a cleared environment: this process holds
//! the credentials, the child must never see them. `/out` is the only writable
//! path, and it is how results come back.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Repo root, so we can find `runner/run.py`. The image sets QUANTRIG_ROOT.
fn root() -> PathBuf {
    std::env::var("QUANTRIG_ROOT")
        .unwrap_or_else(|_| env!("CARGO_MANIFEST_DIR").into())
        .into()
}

/// Python install to bind in: a dev venv, or the image's /usr.
pub fn python_prefix() -> String {
    std::env::var("QUANTRIG_PYTHON_PREFIX").unwrap_or_else(|_| "/usr".into())
}

/// Runs `strategy` over `candles`, writing any report into `out`. Returns JSON stats.
pub fn run_backtest(
    strategy: &Path,
    candles: &Path,
    out: &Path,
    config: &str,
) -> Result<String, Box<dyn Error>> {
    let runner = root().join("runner/run.py").canonicalize()?;
    let strategy = strategy.canonicalize()?;
    let candles = candles.canonicalize()?;
    let out = out.canonicalize()?;
    let prefix = python_prefix();

    let mut cmd = Command::new("bwrap");
    cmd.args(["--unshare-all", "--die-with-parent", "--clearenv"])
        .args(["--setenv", "PATH", "/usr/bin"])
        .args(["--setenv", "HOME", "/tmp"])
        // /etc is deliberately not bound, so there is no ld.so.cache to consult.
        // Debian's python keeps libpython in <prefix>/lib, which is not a default
        // linker path — point the linker straight at it instead of exposing /etc.
        .args(["--setenv", "LD_LIBRARY_PATH", &format!("{prefix}/lib:{prefix}/lib64")])
        .args(["--ro-bind", "/usr", "/usr"])
        .args(["--symlink", "usr/lib", "/lib"])
        .args(["--symlink", "usr/lib64", "/lib64"])
        .args(["--symlink", "usr/bin", "/bin"])
        // No --proc: mounting a fresh procfs inside a userns needs a fully
        // visible /proc, which Docker masks. Nothing in the backtest path reads
        // /proc, so not mounting it keeps the container's defaults intact and
        // makes the sandbox tighter rather than looser.
        .args(["--dev", "/dev"])
        .args(["--tmpfs", "/tmp"]);
    if prefix != "/usr" {
        cmd.args(["--ro-bind", &prefix, &prefix]);
    }
    let out_proc = cmd
        .args(["--ro-bind", runner.to_str().unwrap(), "/q/run.py"])
        .args(["--ro-bind", strategy.to_str().unwrap(), "/q/strategy.py"])
        .args(["--ro-bind", candles.to_str().unwrap(), "/q/candles.parquet"])
        .args(["--bind", out.to_str().unwrap(), "/out"])
        .args(["--chdir", "/q"])
        .arg(format!("{prefix}/bin/python3"))
        .args(["/q/run.py", config])
        .output()?;

    if !out_proc.status.success() {
        let stderr = String::from_utf8_lossy(&out_proc.stderr);
        if stderr.contains("Failed to make / slave") {
            return Err("the sandbox could not start: a mount operation was denied by \
                        the container security policy. Check Docker's AppArmor profile \
                        and, inside Proxmox LXC, the host's AppArmor denials and nesting \
                        configuration. The strategy has not run."
                .into());
        }
        // Distinguish "the sandbox could not start" from "the strategy is wrong";
        // they look identical in the UI otherwise, and the fix is completely different.
        if stderr.contains("create new namespace") {
            return Err("the sandbox could not start: bubblewrap needs user namespaces. \
                        In Docker, add `security_opt: [seccomp=unconfined]` to the \
                        service (see compose.yml) and recreate the container."
                .into());
        }
        return Err(format!("strategy failed: {stderr}").into());
    }
    Ok(String::from_utf8(out_proc.stdout)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const GEN_CANDLES: &str = r#"
import sys, numpy as np, pandas as pd
n = 500
rng = np.random.default_rng(0)
close = 1.10 + np.cumsum(rng.normal(0, 0.0004, n))
pd.DataFrame({"open": close, "high": close + 0.0005, "low": close - 0.0005,
              "close": close, "volume": 0.0},
             index=pd.date_range("2024-01-01", periods=n, freq="h")).to_parquet(sys.argv[1])
"#;

    /// Scratch dir holding a synthetic EURUSD-ish parquet plus per-test strategies.
    fn workdir() -> PathBuf {
        let dir = std::env::temp_dir().join("quantrig-test");
        fs::create_dir_all(dir.join("out")).unwrap();
        let candles = dir.join("candles.parquet");
        if !candles.exists() {
            let out = Command::new(format!("{}/bin/python3", python_prefix()))
                .args(["-c", GEN_CANDLES])
                .arg(&candles)
                .output()
                .unwrap();
            assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        }
        dir
    }

    fn run(dir: &Path, name: &str, body: &str) -> Result<String, Box<dyn Error>> {
        let path = dir.join(name);
        fs::write(&path, body).unwrap();
        run_backtest(&path, &dir.join("candles.parquet"), &dir.join("out"), "{}")
    }

    #[test]
    fn runs_a_strategy_and_returns_stats() {
        let dir = workdir();
        let json = run(&dir, "buyer.py", concat!(
            "from backtestingfx import Strategy\n",
            "class Buyer(Strategy):\n",
            "    def next(self):\n",
            "        self.close_all()\n",
            "        self.buy(lot_size=0.1)\n",
        )).unwrap();
        assert!(json.contains("\"num_trades\""), "{json}");
        assert!(!json.contains("\"num_trades\": 0"), "no trades placed: {json}");
    }

    #[test]
    fn sandbox_has_no_network() {
        let dir = workdir();
        let err = run(&dir, "phone_home.py", concat!(
            "import socket\n",
            "socket.create_connection((\"1.1.1.1\", 53), timeout=5)\n",
            "from backtestingfx import Strategy\n",
            "class X(Strategy):\n",
            "    def next(self): pass\n",
        )).unwrap_err().to_string();
        assert!(err.contains("unreachable") || err.contains("Errno"),
                "sandbox reached the network: {err}");
    }

    #[test]
    fn sandbox_has_no_secrets() {
        let dir = workdir();
        unsafe { std::env::set_var("OANDA_API_KEY", "leaked-if-you-see-this") };
        run(&dir, "snoop.py", concat!(
            "import os\n",
            "assert not [k for k in os.environ if 'KEY' in k or 'TOKEN' in k], dict(os.environ)\n",
            "from backtestingfx import Strategy\n",
            "class X(Strategy):\n",
            "    def next(self): pass\n",
        )).unwrap();
    }

    /// The linker has no ld.so.cache in here, so the prefix must arrive by env.
    #[test]
    fn sandbox_points_the_linker_at_the_python_prefix() {
        let dir = workdir();
        run(&dir, "libs.py", &format!(concat!(
            "import os\n",
            "assert os.environ['LD_LIBRARY_PATH'] == '{0}/lib:{0}/lib64', os.environ.get('LD_LIBRARY_PATH')\n",
            "from backtestingfx import Strategy\n",
            "class X(Strategy):\n",
            "    def next(self): pass\n",
        ), python_prefix())).unwrap();
    }

    #[test]
    fn sandbox_cannot_write_outside_out() {
        let dir = workdir();
        let err = run(&dir, "vandal.py", concat!(
            "open('/q/strategy.py', 'w').write('owned')\n",
            "from backtestingfx import Strategy\n",
            "class X(Strategy):\n",
            "    def next(self): pass\n",
        )).unwrap_err().to_string();
        assert!(err.contains("Read-only"), "sandbox wrote to /q: {err}");
    }
}
