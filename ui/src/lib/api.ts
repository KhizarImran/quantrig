/** Every call goes through the Rust API. Nothing here touches the engine directly. */

export type Stats = Record<string, number>;
export type RunResult = { id: string; stats: Stats };
export type Pair = { symbol: string; name: string };
export type Dataset = {
  name: string;
  bytes: number;
  rows: number | null;
  start: string | null;
  end: string | null;
};

async function call<T>(url: string, init?: RequestInit): Promise<T> {
  const res = await fetch(url, {
    ...init,
    headers: init?.body ? { "content-type": "application/json" } : undefined,
  });
  const data = await res.json();
  if (!res.ok) throw new Error(data.error ?? res.statusText);
  return data as T;
}

const body = (v: unknown) => ({ method: "POST", body: JSON.stringify(v) });

export const api = {
  settings: () => call<{ lse_api_key_set: boolean }>("/api/settings"),
  saveKey: (lse_api_key: string) =>
    call<{ lse_api_key_set: boolean }>("/api/settings", {
      method: "PUT",
      body: JSON.stringify({ lse_api_key }),
    }),
  pairs: () => call<Pair[]>("/api/pairs"),
  datasets: () => call<Dataset[]>("/api/datasets"),
  download: (v: { symbol: string; timeframe: string; start: string; end: string }) =>
    call<{ name: string; summary: { rows: number; start: string; end: string } }>(
      "/api/datasets",
      body(v),
    ),
  run: (v: {
    code: string;
    dataset: string;
    cash: number;
    spread: number;
    commission: number;
  }) => call<RunResult>("/api/run", body(v)),
};

/** The vault's candle resolutions, coarse first — these are the useful ones for FX. */
export const TIMEFRAMES = ["1m", "5m", "15m", "30m", "1h", "4h", "1d", "1w"] as const;
