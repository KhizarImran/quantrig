import { useState } from "react";
import { Play, Loader2 } from "lucide-react";
import { api, type Dataset, type RunResult } from "@/lib/api";
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

function Summary({ result }: { result: RunResult }) {
  const heroPolarity = polarityOf(HERO, result.stats[HERO]);
  return (
    <div className="p-6">
      <div className="mb-6">
        <div className="text-xs text-muted-foreground">{SPECS[HERO].label}</div>
        <div className={`text-5xl font-semibold tracking-tight ${tone(heroPolarity)}`}>
          <Sign polarity={heroPolarity} />
          {formatValue(HERO, result.stats[HERO])}
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
                {formatValue(k, result.stats[k])}
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}

export function Backtest({ datasets, onNeedData }: { datasets: Dataset[]; onNeedData: () => void }) {
  const [code, setCode] = useState(EXAMPLE);
  const [dataset, setDataset] = useState("");
  const [cash, setCash] = useState("10000");
  const [spread, setSpread] = useState("0.0001");
  const [commission, setCommission] = useState("0");
  const [result, setResult] = useState<RunResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const selected = dataset || datasets[0]?.name || "";

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      setResult(
        await api.run({
          code,
          dataset: selected,
          cash: Number(cash),
          spread: Number(spread),
          commission: Number(commission),
        }),
      );
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      setResult(null);
    } finally {
      setBusy(false);
    }
  }

  function indent(e: React.KeyboardEvent<HTMLTextAreaElement>) {
    if (e.key !== "Tab") return;
    e.preventDefault();
    const el = e.currentTarget;
    el.setRangeText("    ", el.selectionStart, el.selectionEnd, "end");
    setCode(el.value);
  }

  if (datasets.length === 0) {
    return (
      <div className="mx-auto w-full max-w-md p-6">
        <Alert>
          <AlertTitle>No candles yet</AlertTitle>
          <AlertDescription className="flex flex-col items-start gap-3">
            <span>A backtest needs data. Download a pair and timeframe first.</span>
            <Button size="sm" onClick={onNeedData}>
              Go to Data
            </Button>
          </AlertDescription>
        </Alert>
      </div>
    );
  }

  return (
    <div className="grid h-full min-h-0 gap-4 p-4 lg:grid-cols-[minmax(420px,42%)_1fr]">
      <form onSubmit={submit} className="grid min-h-0 grid-rows-[1fr_auto] gap-4">
        <Card className="min-h-0 overflow-hidden py-0">
          <textarea
            value={code}
            onChange={(e) => setCode(e.target.value)}
            onKeyDown={indent}
            spellCheck={false}
            className="size-full resize-none bg-transparent p-4 font-mono text-[13px] leading-relaxed outline-none"
          />
        </Card>

        <Card>
          <CardContent className="flex flex-wrap items-end gap-3">
            <div className="min-w-48 flex-[2] space-y-2">
              <Label>Dataset</Label>
              <Select value={selected} onValueChange={(v) => v && setDataset(v)}>
                <SelectTrigger className="w-full font-mono">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {datasets.map((d) => (
                    <SelectItem key={d.name} value={d.name} className="font-mono">
                      {d.name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div className="min-w-24 flex-1 space-y-2">
              <Label htmlFor="cash">Cash</Label>
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
            <Button type="submit" disabled={busy}>
              {busy ? <Loader2 className="size-4 animate-spin" /> : <Play className="size-4" />}
              Run backtest
            </Button>
          </CardContent>
        </Card>
      </form>

      <Card className="flex min-h-0 flex-col overflow-hidden py-0">
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
            <Summary result={result} />
            <iframe
              src={`/report/${result.id}`}
              title="Backtest report"
              className="min-h-0 flex-1 border-t bg-white"
            />
          </>
        )}
      </Card>
    </div>
  );
}
