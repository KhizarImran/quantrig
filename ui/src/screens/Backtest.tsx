import { useCallback, useEffect, useRef, useState } from "react";
import { Play, Loader2 } from "lucide-react";
import { api, type Dataset, type RunResult, type RunSummary, type SavedRun } from "@/lib/api";
import { BacktestHistory } from "@/components/backtest-history";
import { PythonEditor } from "@/components/python-editor";
import { HERO, SPECS, TILE_ORDER, formatValue, polarityOf } from "@/lib/format";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";

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

/** Direction glyph, so status colour never carries the meaning on its own. */
function Sign({ polarity }: { polarity: "good" | "bad" | null }) {
  if (!polarity) return null;
  return <span className="mr-1 text-[0.62em] align-[0.12em]">{polarity === "good" ? "▲" : "▼"}</span>;
}

const tone = (p: "good" | "bad" | null) =>
  p === "good" ? "text-emerald-600 dark:text-emerald-500" : p === "bad" ? "text-destructive" : "";

type DatasetSelection =
  | { kind: "latest"; name: string }
  | { kind: "saved"; run: Pick<SavedRun, "id" | "dataset" | "candle_revision" | "instrument"> };

function Summary({ result, currency }: { result: RunResult; currency: "USD" | null }) {
  const heroPolarity = polarityOf(HERO, result.stats[HERO]);
  return (
    <div className="p-6">
      <div className="mb-6">
        <div className="text-xs text-muted-foreground">{SPECS[HERO].label}</div>
        <div className={`text-5xl font-semibold tracking-tight ${tone(heroPolarity)}`}>
          <Sign polarity={heroPolarity} />
          {formatValue(HERO, result.stats[HERO], currency)}
        </div>
      </div>
      <div className="grid grid-cols-2 overflow-hidden rounded-lg border bg-border gap-px sm:grid-cols-3">
        {TILE_ORDER.filter((k) => k in result.stats).map((k) => {
          const polarity = polarityOf(k, result.stats[k]);
          return (
            <div key={k} className="bg-card px-4 py-3">
              <div className="text-xs text-muted-foreground">{SPECS[k]?.label ?? k}</div>
              <div className={`text-lg font-semibold ${tone(polarity)}`}>
                <Sign polarity={polarity} />
                {formatValue(k, result.stats[k], currency)}
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}

export function Backtest({
  datasets,
  onNeedData,
  version,
  historyVersion,
}: {
  datasets: Dataset[];
  onNeedData: () => void;
  /** Bumped when the agent writes a strategy, so the list refreshes. */
  version: number;
  historyVersion: number;
}) {
  const [code, setCode] = useState(EXAMPLE);
  const [strategies, setStrategies] = useState<string[]>([]);
  const [strategy, setStrategy] = useState("");
  const [dataset, setDataset] = useState<DatasetSelection>({ kind: "latest", name: "" });
  const [cash, setCash] = useState("10000");
  const [spread, setSpread] = useState("0.0001");
  const [commission, setCommission] = useState("0");
  const [result, setResult] = useState<SavedRun | null>(null);
  const [project, setProject] = useState("");
  const [runLabel, setRunLabel] = useState("SMA crossover");
  const [runs, setRuns] = useState<RunSummary[]>([]);
  const [historyError, setHistoryError] = useState<string | null>(null);
  const [historyLoading, setHistoryLoading] = useState(true);
  const historyRequest = useRef<AbortController | null>(null);
  const [opening, setOpening] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const firstSupported = datasets.find((d) => d.backtest_instrument);
  const savedInput = dataset.kind === "saved" ? dataset.run : null;
  const selected = dataset.kind === "saved" ? dataset.run.dataset : dataset.name || firstSupported?.name || "";
  const selectedDataset = datasets.find((d) => d.name === selected);
  const canRun = savedInput
    ? Boolean(savedInput.candle_revision && (savedInput.instrument ?? selectedDataset?.backtest_instrument))
    : Boolean(selectedDataset?.backtest_instrument);

  function selectLatest(name: string) {
    setDataset({ kind: "latest", name });
    setError(null);
  }

  useEffect(() => {
    api.strategies().then(setStrategies).catch(() => setStrategies([]));
  }, [version]);

  const refreshHistory = useCallback(() => {
    historyRequest.current?.abort();
    const request = new AbortController();
    historyRequest.current = request;
    return api.runs(request.signal).then((saved) => {
      if (request.signal.aborted) return;
      setRuns(saved);
      setHistoryError(null);
    }, (err: unknown) => {
      if (!request.signal.aborted) {
        setHistoryError(err instanceof Error ? err.message : String(err));
      }
    }).finally(() => {
      if (!request.signal.aborted) setHistoryLoading(false);
    });
  }, []);

  useEffect(() => {
    void refreshHistory();
    return () => historyRequest.current?.abort();
  }, [historyVersion, refreshHistory, version]);

  function refreshHistoryAfterAction() {
    setHistoryLoading(true);
    return refreshHistory();
  }

  async function openRun(id: string) {
    setOpening(true);
    try {
      const saved = await api.savedRun(id);
      setResult(saved);
      setCode(saved.code);
      setProject(saved.project);
      setRunLabel(saved.strategy);
      setStrategy("");
      setDataset({ kind: "saved", run: saved });
      if (saved.config) {
        setCash(String(saved.config.cash));
        setSpread(String(saved.config.spread));
        setCommission(String(saved.config.commission));
      }
      setError(saved.error);
    } catch (err) {
      setHistoryError(err instanceof Error ? err.message : String(err));
    } finally { setOpening(false); }
  }

  async function moveProject() {
    if (!result) return;
    setOpening(true);
    try {
      const saved = await api.moveRun(result.id, project);
      setResult(saved);
      setProject(saved.project);
      await refreshHistoryAfterAction();
    } catch (err) {
      setHistoryError(err instanceof Error ? err.message : String(err));
    } finally { setOpening(false); }
  }

  function newRun() {
    setResult(null);
    setError(null);
    setCode(EXAMPLE);
    setStrategy("");
    setDataset({ kind: "latest", name: "" });
    setRunLabel("SMA crossover");
    setCash("10000");
    setSpread("0.0001");
    setCommission("0");
  }

  async function deleteRuns(ids: string[]) {
    const outcome = await api.deleteRuns(ids);
    if (result && outcome.deleted.includes(result.id)) newRun();
    await refreshHistoryAfterAction();
    if (outcome.failed.length) {
      throw new Error(`Deleted ${outcome.deleted.length} runs. ${outcome.failed.length} could not be deleted: ${outcome.failed[0].error}`);
    }
  }

  async function load(name: string) {
    setStrategy(name);
    try {
      setCode((await api.strategy(name)).code);
      setRunLabel(name);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    if (!canRun || busy || opening) return;
    setBusy(true);
    setError(null);
    try {
      const run = await api.run({
        code,
        dataset: selected,
        rerun_of: savedInput?.id,
        cash: Number(cash),
        spread: Number(spread),
        commission: Number(commission),
        project,
        strategy: runLabel,
      });
      setResult(run);
      setDataset({ kind: "saved", run });
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      if (!savedInput) setResult(null);
    } finally {
      setBusy(false);
      refreshHistoryAfterAction();
    }
  }

  return (
    <div className="chat-scrollbars grid h-full min-h-0 grid-cols-[15rem_minmax(0,1fr)]">
      <BacktestHistory runs={runs} selected={result?.id} disabled={busy || opening} loading={historyLoading}
        error={historyError} onOpen={openRun} onNew={newRun} onRetry={refreshHistoryAfterAction} onDelete={deleteRuns} />
      <div className="grid min-h-0 gap-4 overflow-auto p-4 xl:grid-cols-[minmax(360px,42%)_minmax(0,1fr)]">
      <form onSubmit={submit} className="grid min-h-0 grid-rows-[1fr_auto] gap-4">
        <Card className="flex min-h-[28rem] flex-col overflow-hidden py-0">
          <div className="grid gap-3 border-b p-3 sm:grid-cols-2">
            <div className="space-y-1.5">
              <Label htmlFor="backtest-project">Project</Label>
              <Input id="backtest-project" list="backtest-projects" value={project} maxLength={80}
                onChange={(e) => setProject(e.target.value)} placeholder="e.g. Martingale" />
              <datalist id="backtest-projects">{Array.from(new Set(runs.map((r) => r.project))).map((name) => <option key={name} value={name} />)}</datalist>
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="backtest-label">Strategy name</Label>
              <Input id="backtest-label" value={runLabel} onChange={(e) => setRunLabel(e.target.value)} placeholder="e.g. Martingale v2" />
            </div>
            <p className="text-xs text-muted-foreground sm:col-span-2">Use the same project for different pairs and timeframes. Leaving it blank saves to Ungrouped.</p>
            {result && <Button type="button" size="sm" variant="outline" disabled={opening || busy || (project.trim() || "Ungrouped") === result.project} onClick={moveProject}>Move saved run to this project</Button>}
          </div>
          <div className="flex items-center justify-between gap-3 border-b px-3 py-2">
            <span className="text-xs tracking-wide text-muted-foreground uppercase">
              Strategy
            </span>
            {strategies.length > 0 && (
              <Select value={strategy} disabled={busy || opening} onValueChange={(v) => v && load(v)}>
                <SelectTrigger size="sm" className="w-56 font-mono text-xs">
                  <SelectValue placeholder="load saved…" />
                </SelectTrigger>
                <SelectContent>
                  {strategies.map((n) => (
                    <SelectItem key={n} value={n} className="font-mono text-xs">
                      {n}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            )}
          </div>
          <PythonEditor
            value={code}
            onChange={setCode}
          />
        </Card>

        <Card>
          <CardContent className="flex flex-wrap items-end gap-3">
            {!firstSupported && !canRun && <Alert><AlertDescription>Download a supported FX dataset, such as EUR/USD, to run a backtest.<Button type="button" size="sm" variant="outline" onClick={onNeedData}>Go to Data</Button></AlertDescription></Alert>}
            {savedInput && (
              <Alert><AlertDescription>
                {savedInput.candle_revision
                  ? "Running again uses the saved candle revision."
                  : "The original candle revision is unavailable for this saved run."}
                {selectedDataset?.backtest_instrument && <Button type="button" size="sm" variant="outline" disabled={busy || opening}
                  onClick={() => selectLatest(selected)}>Use latest candles</Button>}
              </AlertDescription></Alert>
            )}
            {!savedInput && firstSupported && selected && !canRun && (
              <Alert><AlertDescription>
                {selectedDataset
                  ? "This dataset needs verified contract sizing or currency conversion. Choose a supported dataset to run again."
                  : "The saved dataset is unavailable. Choose a dataset to run again."}
              </AlertDescription></Alert>
            )}
            <div className="min-w-48 flex-[2] space-y-2">
              <Label htmlFor="backtest-dataset">Dataset</Label>
              <Select value={selected} disabled={busy || opening} onValueChange={(v) => v && selectLatest(v)}>
                <SelectTrigger id="backtest-dataset" className="w-full font-mono">
                  <SelectValue placeholder="Choose a dataset" />
                </SelectTrigger>
                <SelectContent>
                  {datasets.map((d) => (
                    <SelectItem key={d.name} value={d.name} disabled={!d.backtest_instrument} className="font-mono">
                      {d.name}{!d.backtest_instrument && " (unsupported)"}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div className="min-w-24 flex-1 space-y-2">
              <Label htmlFor="cash">Cash (USD)</Label>
              <Input id="cash" value={cash} onChange={(e) => setCash(e.target.value)} className="font-mono" />
            </div>
            <div className="min-w-24 flex-1 space-y-2">
              <Label htmlFor="spread">Spread</Label>
              <Input id="spread" value={spread} onChange={(e) => setSpread(e.target.value)} className="font-mono" />
            </div>
            <div className="min-w-24 flex-1 space-y-2">
              <Label htmlFor="commission">Commission</Label>
              <Input
                id="commission"
                value={commission}
                onChange={(e) => setCommission(e.target.value)}
                className="font-mono"
              />
            </div>
            <Button type="submit" disabled={busy || opening || !canRun}>
              {busy ? <Loader2 className="size-4 animate-spin" /> : <Play className="size-4" />}
              Run backtest
            </Button>
          </CardContent>
        </Card>
      </form>

      <Card className="flex min-h-0 flex-col overflow-hidden py-0">
        {result && <div className="border-b px-4 py-3 text-xs text-muted-foreground">
          <p className="font-medium text-foreground">{result.project} / {result.strategy}</p>
          <p className="mt-1">{result.dataset || "Archived report"} · {new Date(result.created).toLocaleString()} · {result.source === "agent" ? "Chat run" : "Saved run"}</p>
          {result.candle_revision && <p className="mt-1">
            Candle span {result.candle_revision.start.slice(0, 10)} to {result.candle_revision.end.slice(0, 10)} ·{" "}
            {result.candle_revision.rows.toLocaleString()} bars
          </p>}
          <p className="mt-1">{result.instrument ? (
            <>
              {result.instrument.contract_size.toLocaleString()} {result.instrument.base_currency} per lot ·{" "}
              {result.instrument.account_currency} account · Quote conversion {result.instrument.quote_to_account}
            </>
          ) : "Contract sizing and currency conversion were not recorded for this run."}</p>
          <p className="mt-1">Running again creates a new entry.</p>
        </div>}
        {busy && <div className="h-0.5 w-full animate-pulse bg-primary" />}

        {error && (
          <div className="p-6">
            <Alert variant="destructive">
              <AlertTitle>Strategy failed</AlertTitle>
              <AlertDescription>
                <pre className="mt-2 max-h-[50vh] overflow-auto whitespace-pre-wrap font-mono text-xs">
                  {error}
                </pre>
              </AlertDescription>
            </Alert>
          </div>
        )}

        {!error && !result && (
          <div className="m-auto max-w-xs p-6 text-center">
            <p className="font-medium">No results yet</p>
            <p className="mt-1 text-sm text-muted-foreground">
              Edit the strategy and run it. Stats and the full report land here.
            </p>
          </div>
        )}

        {result && !error && (
          <>
            {result.stats ? <Summary result={{ id: result.id, stats: result.stats }} currency={result.instrument?.account_currency ?? null} /> : <p className="p-4 text-xs text-muted-foreground">This older run saved only its source and report. Original settings and summary stats are unavailable.</p>}
            {result.report_available && <iframe
              src={`/report/${result.id}`}
              title="Backtest report"
              sandbox="allow-scripts"
              className="min-h-80 flex-1 border-t bg-white"
            />}
          </>
        )}
      </Card>
      </div>
    </div>
  );
}
