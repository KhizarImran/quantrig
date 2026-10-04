/** Every call goes through the Rust API. Nothing here touches the engine directly. */

export type Stats = Record<string, number>;
export type RunResult = { id: string; stats: Stats };
export type RunSummary = {
  id: string;
  project: string;
  strategy: string;
  dataset: string;
  created: number;
  source: "manual" | "agent" | "legacy";
  status: "completed" | "failed";
  return_pct: number | null;
};
export type SavedRun = Omit<RunSummary, "status" | "return_pct"> & {
  code: string;
  config: { cash: number; spread: number; commission: number } | null;
  stats: Stats | null;
  error: string | null;
  report_available: boolean;
};
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

export type KeyName = "lse_api_key" | "opencode_api_key";
export type SettingsState = Record<`${KeyName}_set`, boolean>;

/** One chat turn's messages, in OpenAI shape: assistant text, tool calls, tool results. */
export type ToolCall = { id: string; function: { name: string; arguments: string } };
export type Message = {
  role: "user" | "assistant" | "tool";
  content?: string | null;
  name?: string;
  tool_calls?: ToolCall[];
  tool_call_id?: string;
};

export type ChatEvent =
  | { type: "text"; delta: string }
  | { type: "reasoning"; delta: string }
  | { type: "message"; message: Message }
  | { type: "tool"; id: string; name: string; arguments: string }
  | { type: "tool_result"; message: Message }
  | { type: "done" }
  | { type: "error"; error: string };

/** A saved chat. `parts` is the rendered transcript, `history` what the model sees. */
export type ConversationSummary = { id: string; title: string; updated: number };
export type Conversation<P> = ConversationSummary & {
  model: string;
  parts: P[];
  history: Message[];
};

export const api = {
  settings: () => call<SettingsState>("/api/settings"),
  saveKey: (which: KeyName, value: string) =>
    call<SettingsState>("/api/settings", {
      method: "PUT",
      body: JSON.stringify({ [which]: value }),
    }),
  models: () =>
    call<{ data: { id: string }[] }>("/api/models").then((r) =>
      r.data.map((m) => m.id).sort(),
    ),
  /** Streams one turn. Yields events until `done` or `error`. */
  chat: async function* (model: string, session: string, messages: Message[]) {
    const res = await fetch("/api/chat", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ model, session, messages }),
    });
    if (!res.ok || !res.body) throw new Error((await res.json()).error ?? res.statusText);

    const reader = res.body.pipeThrough(new TextDecoderStream()).getReader();
    let buffer = "";
    while (true) {
      const { value, done } = await reader.read();
      if (done) return;
      buffer += value;
      // SSE frames are separated by a blank line; a frame can span reads.
      let split: number;
      while ((split = buffer.indexOf("\n\n")) !== -1) {
        const frame = buffer.slice(0, split);
        buffer = buffer.slice(split + 2);
        for (const line of frame.split("\n")) {
          if (line.startsWith("data:")) yield JSON.parse(line.slice(5)) as ChatEvent;
        }
      }
    }
  },
  conversations: () => call<ConversationSummary[]>("/api/conversations"),
  conversation: <P>(id: string) => call<Conversation<P>>(`/api/conversations/${id}`),
  saveConversation: <P>(id: string, v: Omit<Conversation<P>, "id" | "updated">) =>
    call<ConversationSummary>(`/api/conversations/${id}`, {
      method: "PUT",
      body: JSON.stringify(v),
    }),
  deleteConversation: (id: string) =>
    call<{ id: string }>(`/api/conversations/${id}`, { method: "DELETE" }),
  strategies: () => call<string[]>("/api/strategies"),
  strategy: (name: string) => call<{ name: string; code: string }>(`/api/strategies/${name}`),
  saveStrategy: (name: string, code: string) =>
    call<{ name: string }>("/api/strategies", {
      method: "PUT",
      body: JSON.stringify({ name, code }),
    }),
  pairs: (refresh = false) => call<Pair[]>(`/api/pairs?refresh=${refresh}`),
  datasets: () => call<Dataset[]>("/api/datasets"),
  runs: () => call<RunSummary[]>("/api/runs"),
  deleteRuns: (ids: string[]) => call<{ deleted: string[]; failed: { id: string; error: string }[] }>("/api/runs", {
    method: "DELETE", body: JSON.stringify({ ids }),
  }),
  savedRun: (id: string) => call<SavedRun>(`/api/runs/${id}`),
  moveRun: (id: string, project: string) => call<SavedRun>(`/api/runs/${id}`, {
    method: "PUT", body: JSON.stringify({ project }),
  }),
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
    project?: string;
    strategy?: string;
  }) => call<SavedRun>("/api/run", body(v)),
};

/** The vault's candle resolutions, coarse first — these are the useful ones for FX. */
export const TIMEFRAMES = ["1m", "5m", "15m", "30m", "1h", "4h", "1d", "1w"] as const;
