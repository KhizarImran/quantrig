"""Runs one agent-written strategy. Sandboxed: no network, no secrets, /q read-only.

Everything here is untrusted-adjacent — it shares a process with LLM-written code,
so it must never be handed a credential. /out is the only writable path.
"""
import importlib.util
import json
import sys

import pandas as pd
from backtestingfx import Backtest, Strategy


def load_strategy(path):
    spec = importlib.util.spec_from_file_location("strategy", path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    found = [o for o in vars(mod).values()
             if isinstance(o, type) and issubclass(o, Strategy) and o is not Strategy]
    if len(found) != 1:
        raise SystemExit(f"expected exactly one Strategy subclass, found {len(found)}")
    return found[0]


cfg = json.loads(sys.argv[1]) if len(sys.argv) > 1 else {}
df = pd.read_parquet("/q/candles.parquet")
bt = Backtest(df, load_strategy("/q/strategy.py"),
              cash=cfg.get("cash", 10000.0),
              spread=cfg.get("spread", 0.0001),
              commission=cfg.get("commission", 0.0),
              quote_to_account=cfg.get("quote_to_account", 1.0))
stats = bt.run()

if cfg.get("plot"):
    bt.plot("/out/report.html")

# ponytail: scalars only. equity_curve/trades when the Results screen needs them.
FIELDS = ("initial_cash", "final_cash", "total_return_pct", "num_trades", "num_wins",
          "win_rate_pct", "avg_pnl", "best_trade", "worst_trade", "profit_factor",
          "max_drawdown_pct", "sharpe_ratio")
print(json.dumps({f: getattr(stats, f) for f in FIELDS}))
