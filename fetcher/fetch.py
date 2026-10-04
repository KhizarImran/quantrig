"""Market data fetcher. Holds the API key and is allowed on the network.

This is the opposite side of the boundary from runner/run.py: it sees the
credential and never sees strategy code. Invoked by the API, never by a strategy.

    fetch.py catalog <out.json>
    fetch.py download <symbol> <timeframe> <start> <end> <out.parquet>
    fetch.py selftest
"""
import http.client
import json
import os
import pathlib
import sys
import time

import pandas as pd
from lse import LSE, LSEError

# What backtestingfx needs out of whatever the vault hands back.
COLUMNS = ["open", "high", "low", "close", "volume"]

ATTEMPTS = 3
# The client defaults to 60s, which the multi-megabyte catalog read overruns.
REST_TIMEOUT = 300
# The vault caps a candles call at 5000 rows; that is also a comfortable page.
PAGE = 5000
# ponytail: ~2.5M bars. Raise it when someone actually wants minute data since 2009.
MAX_PAGES = 500


def retry(call, what: str):
    """Large vault reads drop mid-transfer. Try again; downloads resume."""
    last = None
    for attempt in range(1, ATTEMPTS + 1):
        try:
            return call()
        except LSEError as e:
            # A bad key or a missing symbol will not fix itself; only transport does.
            if e.status in (400, 401, 403, 404):
                raise
            last = e
        except (OSError, EOFError, http.client.HTTPException) as e:
            # IncompleteRead is an HTTPException, NOT an OSError — a chunked
            # response cut short lands here, and it is the common failure.
            last = e
        print(f"{what} attempt {attempt}/{ATTEMPTS} failed: {last}", file=sys.stderr)
        if attempt < ATTEMPTS:
            time.sleep(2 * attempt)
    raise SystemExit(f"{what} failed after {ATTEMPTS} attempts: {last}")


def normalise(df: pd.DataFrame) -> pd.DataFrame:
    """Vault parquet -> OHLCV frame on a DatetimeIndex."""
    df = df.rename(columns={c: c.lower() for c in df.columns})
    stamp = next((c for c in ("timestamp", "ts", "time", "date") if c in df.columns), None)
    if stamp is None:
        raise SystemExit(f"no timestamp column in {list(df.columns)}")
    # to_datetime on a column gives a Series; tz_localize there would target the
    # frame's index, not these values. Build the index explicitly.
    df.index = pd.DatetimeIndex(pd.to_datetime(df[stamp], utc=True, format="mixed")).tz_localize(None)
    missing = [c for c in COLUMNS if c not in df.columns and c != "volume"]
    if missing:
        raise SystemExit(f"missing OHLC columns {missing} in {list(df.columns)}")
    if "volume" not in df.columns:
        df["volume"] = 0.0
    df = df[COLUMNS].astype(float).sort_index()
    # A NaN close would poison the backtest silently; FX candles legitimately
    # carry no consolidated volume, so that one defaults instead of dropping.
    df["volume"] = df["volume"].fillna(0.0)
    return df.dropna(subset=["open", "high", "low", "close"])


def research_instruments(rows: list) -> list:
    """Keep FX, commodities and indices; deduplicate vault dataset rows."""
    categories = {"forex", "fx", "commodity", "commodities", "index", "indices", "indexes"}
    instruments = {}
    for row in rows:
        symbol = row.get("symbol", "")
        category = str(row.get("category", "")).strip().lower()
        if symbol and category in categories:
            instruments.setdefault(symbol, {
                "symbol": symbol,
                "name": row.get("name") or row.get("display_name") or symbol,
            })
    return sorted(instruments.values(), key=lambda instrument: instrument["symbol"])


def catalog(client: LSE, out: str) -> None:
    """catalog() pulls the whole vault index (22,000+ rows, several MB) and
    filters client-side, so this is one big GET that truncates often. Retry it,
    then cache the instruments supported by the picker."""
    rows = retry(lambda: client.catalog(), "catalog")
    pairs = research_instruments(rows)
    if not pairs:
        raise SystemExit("catalog returned no FX, commodity or index instruments")
    pathlib.Path(out).write_text(json.dumps(pairs))
    print(json.dumps(pairs))


def fetch_chunks(client: LSE, symbol: str, timeframe: str, start: str, end: str) -> list:
    """Page the candles endpoint instead of pulling one big export file.

    Each call returns at most PAGE bars (~0.5 MB), so a dropped transfer costs
    one page rather than the whole history — which is what kept truncating at
    4 MB. It also spends no export-job budget.
    """
    rows: list = []
    cursor = start or "2009-01-01"
    last = None
    for page in range(1, MAX_PAGES + 1):
        batch = retry(
            lambda c=cursor: client.candles(symbol, timeframe, start=c, end=end or None,
                                            limit=PAGE, order="asc"),
            f"{symbol} {timeframe} page {page} from {cursor}",
        )
        # start is inclusive, so the first row of each page repeats the last one.
        if last is not None:
            batch = [r for r in batch if r["timestamp"] > last]
        if not batch:
            break
        rows.extend(batch)
        last = batch[-1]["timestamp"]
        cursor = last
        print(f"{symbol} {timeframe}: {len(rows)} bars to {last}", file=sys.stderr)
        if len(batch) < PAGE - 1:  # short page means we reached the end
            break
    else:
        raise SystemExit(
            f"{symbol} {timeframe} exceeded {MAX_PAGES} pages; narrow the date range"
        )
    return rows


def download(client: LSE, symbol: str, timeframe: str, start: str, end: str, out: str) -> None:
    rows = fetch_chunks(client, symbol, timeframe, start, end)
    if not rows:
        raise SystemExit(f"{symbol} {timeframe} returned no candles for that range")
    df = normalise(pd.DataFrame(rows))
    df.to_parquet(out)
    print(json.dumps({
        "rows": len(df),
        "start": df.index[0].isoformat(),
        "end": df.index[-1].isoformat(),
    }))


def selftest() -> None:
    """The bug this guards: IncompleteRead is an HTTPException, not an OSError,
    so a retry catching only OSError lets it straight through."""
    calls = []

    instruments = research_instruments([
        {"symbol": "EUR/USD", "category": "Forex"},
        {"symbol": "XAU/USD", "category": "Commodities", "name": "Gold"},
        {"symbol": "US30", "category": "Index", "display_name": "Dow Jones"},
        {"symbol": "US30", "category": "Indices"},
        {"symbol": "AAPL", "category": "Stocks"},
    ])
    assert [r["symbol"] for r in instruments] == ["EUR/USD", "US30", "XAU/USD"]
    assert instruments[1]["name"] == "Dow Jones"

    def flaky():
        calls.append(1)
        if len(calls) < ATTEMPTS:
            raise http.client.IncompleteRead(b"partial")
        return "recovered"

    assert retry(flaky, "selftest") == "recovered"
    assert len(calls) == ATTEMPTS, calls

    def unauthorised():
        raise LSEError(401, "invalid api key")

    try:
        retry(unauthorised, "selftest")
    except LSEError as e:
        assert e.status == 401
    else:
        raise AssertionError("a 401 must not be retried")
    # Paging: overlapping first rows must not duplicate, and a short page ends it.
    bars = [{"timestamp": f"2024-01-01T{h:02d}:00:00", "open": 1.0, "high": 1.0,
             "low": 1.0, "close": 1.0, "volume": 0.0} for h in range(24)]

    class StubClient:
        def candles(self, _symbol, _timeframe, start=None, end=None, limit=0, order=""):
            after = [b for b in bars if b["timestamp"] >= start]
            return after[:8]

    global PAGE
    PAGE, previous = 8, PAGE
    try:
        rows = fetch_chunks(StubClient(), "EUR/USD", "1h", "2024-01-01T00:00:00", "")
    finally:
        PAGE = previous
    stamps = [r["timestamp"] for r in rows]
    assert len(stamps) == len(set(stamps)), "paging duplicated the overlap row"
    assert len(rows) == 24, f"expected every bar, got {len(rows)}"

    # normalise(): the vault hands back ISO strings in a plain column.
    frame = normalise(pd.DataFrame([
        {"timestamp": "2024-01-01T00:00:00+00:00", "open": 1.1, "high": 1.2,
         "low": 1.0, "close": 1.15, "volume": 3.0},
        {"timestamp": "2024-01-01T01:00:00Z", "open": 1.15, "high": 1.25,
         "low": 1.05, "close": 1.2},
    ]))
    assert isinstance(frame.index, pd.DatetimeIndex), type(frame.index)
    assert frame.index.tz is None, "backtestingfx wants naive timestamps"
    assert list(frame.columns) == COLUMNS, list(frame.columns)
    assert frame["volume"].tolist() == [3.0, 0.0], "missing volume must default to 0"

    print("selftest ok")


def main() -> None:
    cmd = sys.argv[1]
    if cmd == "selftest":
        return selftest()

    client = LSE(api_key=os.environ["LSE_API_KEY"], timeout=REST_TIMEOUT)
    if cmd == "catalog":
        return catalog(client, sys.argv[2])
    if cmd == "download":
        symbol, timeframe, start, end, out = sys.argv[2:7]
        return download(client, symbol, timeframe, start, end, out)
    raise SystemExit(f"unknown command {cmd}")


if __name__ == "__main__":
    try:
        main()
    except LSEError as e:
        raise SystemExit(f"London Strategic Edge: {e.message} (status {e.status})")
