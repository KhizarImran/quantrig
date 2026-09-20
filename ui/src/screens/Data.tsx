import { useState } from "react";
import { Download, Loader2 } from "lucide-react";
import { api, TIMEFRAMES, type Dataset, type Pair } from "@/lib/api";
import { FX_PAIRS } from "@/lib/pairs";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";

const size = (bytes: number) =>
  bytes >= 1e6 ? `${(bytes / 1e6).toFixed(1)} MB` : `${Math.max(1, Math.round(bytes / 1e3))} KB`;
const day = (iso: string | null) => (iso ? iso.slice(0, 10) : "—");

/** A year of hourly candles is a sane first pull. */
function defaultStart() {
  const d = new Date();
  d.setFullYear(d.getFullYear() - 1);
  return d.toISOString().slice(0, 10);
}

export function Data({ datasets, reload }: { datasets: Dataset[]; reload: () => void }) {
  // Built-in list by default: nothing is fetched until you ask for it.
  const [pairs, setPairs] = useState<Pair[]>(FX_PAIRS);
  const [pairsError, setPairsError] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [symbol, setSymbol] = useState("EUR/USD");
  const [timeframe, setTimeframe] = useState("1h");
  const [start, setStart] = useState(defaultStart);
  const [end, setEnd] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function refreshPairs() {
    setRefreshing(true);
    setPairsError(null);
    try {
      setPairs(await api.pairs());
    } catch (e) {
      setPairsError(e instanceof Error ? e.message : String(e));
    } finally {
      setRefreshing(false);
    }
  }

  async function download(e: React.FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await api.download({ symbol, timeframe, start, end });
      reload();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="mx-auto grid w-full max-w-5xl gap-6 p-6">
      <Card>
        <CardHeader>
          <CardTitle>Download candles</CardTitle>
          <CardDescription>
            Pulled from London Strategic Edge and stored as parquet. FX history reaches back
            to 2009.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <form onSubmit={download} className="grid gap-4 sm:grid-cols-[2fr_1fr_1fr_1fr_auto] sm:items-end">
            <div className="space-y-2">
              <div className="flex items-center justify-between">
                <Label>Pair</Label>
                <button
                  type="button"
                  onClick={refreshPairs}
                  disabled={refreshing}
                  className="text-xs text-muted-foreground underline-offset-2 hover:underline"
                >
                  {refreshing ? "refreshing…" : "refresh from LSE"}
                </button>
              </div>
              <Select value={symbol} onValueChange={(v) => v && setSymbol(v)}>
                <SelectTrigger className="w-full font-mono">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {pairs.map((p) => (
                    <SelectItem key={p.symbol} value={p.symbol} className="font-mono">
                      {p.symbol}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>

            <div className="space-y-2">
              <Label>Timeframe</Label>
              <Select value={timeframe} onValueChange={(v) => v && setTimeframe(v)}>
                <SelectTrigger className="w-full font-mono">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {TIMEFRAMES.map((t) => (
                    <SelectItem key={t} value={t} className="font-mono">
                      {t}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>

            <div className="space-y-2">
              <Label htmlFor="start">From</Label>
              <Input id="start" type="date" value={start} onChange={(e) => setStart(e.target.value)} />
            </div>

            <div className="space-y-2">
              <Label htmlFor="end">To</Label>
              <Input id="end" type="date" value={end} onChange={(e) => setEnd(e.target.value)} />
            </div>

            <Button type="submit" disabled={busy}>
              {busy ? <Loader2 className="size-4 animate-spin" /> : <Download className="size-4" />}
              Download
            </Button>
          </form>

          {pairsError && (
            <Alert className="mt-4">
              <AlertDescription>
                Could not refresh the pair list ({pairsError}). Using the built-in list —
                downloads are unaffected.
              </AlertDescription>
            </Alert>
          )}
          {error && (
            <Alert variant="destructive" className="mt-4">
              <AlertDescription>{error}</AlertDescription>
            </Alert>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Downloaded</CardTitle>
          <CardDescription>These are what a backtest can run against.</CardDescription>
        </CardHeader>
        <CardContent>
          {datasets.length === 0 ? (
            <p className="py-6 text-center text-sm text-muted-foreground">
              Nothing downloaded yet.
            </p>
          ) : (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Dataset</TableHead>
                  <TableHead className="text-right">Bars</TableHead>
                  <TableHead>From</TableHead>
                  <TableHead>To</TableHead>
                  <TableHead className="text-right">Size</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {datasets.map((d) => (
                  <TableRow key={d.name}>
                    <TableCell className="font-mono">{d.name}</TableCell>
                    <TableCell className="text-right tabular-nums">
                      {d.rows?.toLocaleString() ?? "—"}
                    </TableCell>
                    <TableCell className="tabular-nums">{day(d.start)}</TableCell>
                    <TableCell className="tabular-nums">{day(d.end)}</TableCell>
                    <TableCell className="text-right tabular-nums">{size(d.bytes)}</TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
