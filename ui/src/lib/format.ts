/** Display rules for one backtest's stats. Ordering here is the reading order. */

export type Stats = Record<string, number>;

type Kind = "money" | "pct" | "ratio" | "count";
/** Which way is good — so the tile can show a direction glyph, not colour alone. */
type Polarity = "up-good" | "neutral";

type Spec = { label: string; kind: Kind; polarity: Polarity };

export const HERO = "total_return_pct";

export const SPECS: Record<string, Spec> = {
  total_return_pct: { label: "Total return", kind: "pct", polarity: "up-good" },
  final_cash: { label: "Final balance", kind: "money", polarity: "neutral" },
  num_trades: { label: "Trades", kind: "count", polarity: "neutral" },
  win_rate_pct: { label: "Win rate", kind: "pct", polarity: "neutral" },
  profit_factor: { label: "Profit factor", kind: "ratio", polarity: "up-good" },
  max_drawdown_pct: { label: "Max drawdown", kind: "pct", polarity: "neutral" },
  sharpe_ratio: { label: "Sharpe", kind: "ratio", polarity: "up-good" },
  avg_pnl: { label: "Avg PnL / trade", kind: "money", polarity: "up-good" },
  best_trade: { label: "Best trade", kind: "money", polarity: "neutral" },
  worst_trade: { label: "Worst trade", kind: "money", polarity: "neutral" },
  num_wins: { label: "Winning trades", kind: "count", polarity: "neutral" },
  initial_cash: { label: "Starting balance", kind: "money", polarity: "neutral" },
};

/** Reading order for the tile grid: headline metrics first. */
export const TILE_ORDER = [
  "final_cash",
  "num_trades",
  "win_rate_pct",
  "profit_factor",
  "max_drawdown_pct",
  "sharpe_ratio",
  "avg_pnl",
  "best_trade",
  "worst_trade",
];

const money = new Intl.NumberFormat(undefined, {
  style: "currency",
  currency: "USD",
  maximumFractionDigits: 2,
});

export function formatValue(key: string, v: number): string {
  if (!Number.isFinite(v)) return "—";
  switch (SPECS[key]?.kind) {
    case "money":
      return money.format(v);
    case "pct":
      return `${v.toFixed(2)}%`;
    case "count":
      return v.toLocaleString();
    default:
      return v.toFixed(2);
  }
}

/** "good" | "bad" | null — null means the number has no better or worse direction. */
export function polarityOf(key: string, v: number): "good" | "bad" | null {
  if (SPECS[key]?.polarity !== "up-good" || !Number.isFinite(v)) return null;
  const pivot = SPECS[key].kind === "ratio" && key === "profit_factor" ? 1 : 0;
  if (v === pivot) return null;
  return v > pivot ? "good" : "bad";
}
