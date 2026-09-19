import { useState } from "react";
import { HERO, SPECS, TILE_ORDER, formatValue, polarityOf, type Stats } from "./format";

const EXAMPLE = `from backtestingfx import Strategy

FAST, SLOW = 10, 30


class SmaCross(Strategy):
    def init(self):
        closes = [b.close for b in self._bars]
        self.long_signal = {}
        for i in range(SLOW - 1, len(closes)):
            window = closes[i - SLOW + 1 : i + 1]
            fast = sum(window[-FAST:]) / FAST
            slow = sum(window) / SLOW
            self.long_signal[self._bars[i].timestamp] = fast > slow

    def next(self):
        up = self.long_signal.get(self._bar.timestamp)
        if up is None:
            return
        if up and not self.positions:
            self.buy(lot_size=0.1)
        elif not up and self.positions:
            self.close_all()
`;

type RunResponse = { id: string; stats: Stats };

/** Direction glyph, so status colour never carries the meaning on its own. */
function Sign({ polarity }: { polarity: "good" | "bad" | null }) {
  if (!polarity) return null;
  return <span className="sign">{polarity === "good" ? "▲" : "▼"}</span>;
}

function Hero({ value }: { value: number }) {
  const polarity = polarityOf(HERO, value);
  return (
    <div className="hero">
      <span className="label">{SPECS[HERO].label}</span>
      <strong className={polarity ?? ""}>
        <Sign polarity={polarity} />
        {formatValue(HERO, value)}
      </strong>
    </div>
  );
}

function Tiles({ stats }: { stats: Stats }) {
  return (
    <div className="tiles">
      {TILE_ORDER.filter((k) => k in stats).map((k) => {
        const polarity = polarityOf(k, stats[k]);
        return (
          <div className="tile" key={k}>
            <span className="label">{SPECS[k]?.label ?? k}</span>
            <span className={`value ${polarity ?? ""}`}>
              <Sign polarity={polarity} />
              {formatValue(k, stats[k])}
            </span>
          </div>
        );
      })}
    </div>
  );
}

export default function App() {
  // ponytail: uncontrolled form, state only for what comes back.
  const [result, setResult] = useState<RunResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function submit(e: React.FormEvent<HTMLFormElement>) {
    e.preventDefault();
    const f = new FormData(e.currentTarget);
    setBusy(true);
    setError(null);
    try {
      const res = await fetch("/run", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          code: f.get("code"),
          candles: f.get("candles"),
          cash: Number(f.get("cash")),
          spread: Number(f.get("spread")),
          commission: Number(f.get("commission")),
        }),
      });
      const data = await res.json();
      if (res.ok) {
        setResult(data);
      } else {
        setError(data.error);
        setResult(null);
      }
    } catch (err) {
      setError(String(err));
      setResult(null);
    } finally {
      setBusy(false);
    }
  }

  /** Tab indents in the editor instead of escaping to the next control. */
  function indent(e: React.KeyboardEvent<HTMLTextAreaElement>) {
    if (e.key !== "Tab") return;
    e.preventDefault();
    const el = e.currentTarget;
    const at = el.selectionStart;
    el.setRangeText("    ", at, el.selectionEnd, "end");
  }

  return (
    <div className="app">
      <header>
        <span className="brand">quantrig</span>
        <span className="tagline">backtest · sandboxed</span>
      </header>

      <form onSubmit={submit}>
        <div className="panel editor">
          <div className="panel-head">
            <h2>Strategy</h2>
            <span className="hint">Python · runs with no network, no secrets</span>
          </div>
          <textarea name="code" defaultValue={EXAMPLE} spellCheck={false} onKeyDown={indent} />
        </div>

        <div className="panel params">
          <label>
            <span>Candles</span>
            <input name="candles" defaultValue="/tmp/eurusd.parquet" />
          </label>
          <label>
            <span>Cash</span>
            <input name="cash" type="number" step="any" defaultValue={10000} />
          </label>
          <label>
            <span>Spread</span>
            <input name="spread" type="number" step="any" defaultValue={0.0001} />
          </label>
          <label>
            <span>Commission</span>
            <input name="commission" type="number" step="any" defaultValue={0} />
          </label>
          <button disabled={busy}>{busy ? "Running…" : "Run backtest"}</button>
        </div>
      </form>

      <section className="panel results">
        {busy && <div className="progress" />}

        {error && (
          <div className="empty">
            <h2 className="bad">Strategy failed</h2>
            <pre className="err">{error}</pre>
          </div>
        )}

        {!error && !result && !busy && (
          <div className="empty">
            <h2>No results yet</h2>
            <p>Edit the strategy and run it. Stats and the full report land here.</p>
          </div>
        )}

        {result && !error && (
          <>
            <div className="summary">
              <Hero value={result.stats[HERO]} />
              <Tiles stats={result.stats} />
            </div>
            <iframe src={`/report/${result.id}`} title="Backtest report" />
          </>
        )}
      </section>
    </div>
  );
}
