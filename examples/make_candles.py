"""Synthetic EURUSD-ish hourly candles, until the fetcher exists."""
import sys

import numpy as np
import pandas as pd

n = 2000
close = 1.10 + np.cumsum(np.random.default_rng(0).normal(0, 0.0004, n))
pd.DataFrame(
    {"open": close, "high": close + 0.0005, "low": close - 0.0005,
     "close": close, "volume": 0.0},
    index=pd.date_range("2024-01-01", periods=n, freq="h"),
).to_parquet(sys.argv[1])
