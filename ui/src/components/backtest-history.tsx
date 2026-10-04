import { Folder, Loader2, Plus, Trash2 } from "lucide-react";
import { useState } from "react";
import type { RunSummary } from "@/lib/api";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";

export function BacktestHistory({ runs, selected, disabled, loading, error, onOpen, onNew, onRetry, onDelete }: {
  runs: RunSummary[];
  selected?: string;
  disabled: boolean;
  loading: boolean;
  error: string | null;
  onOpen: (id: string) => void;
  onNew: () => void;
  onRetry: () => void;
  onDelete: (ids: string[]) => Promise<void>;
}) {
  const [pending, setPending] = useState<{ title: string; ids: string[]; group: boolean } | null>(null);
  const [deleting, setDeleting] = useState(false);
  const [deleteError, setDeleteError] = useState<string | null>(null);
  function confirm(title: string, ids: string[], group: boolean) {
    setPending({ title, ids, group });
    setDeleteError(null);
  }
  async function remove() {
    if (!pending || deleting) return;
    setDeleting(true);
    setDeleteError(null);
    try {
      // Intersect with current history on retry after a partially successful delete.
      const ids = pending.ids.filter((id) => runs.some((run) => run.id === id));
      if (ids.length) await onDelete(ids);
      setPending(null);
    } catch (err) {
      setDeleteError(err instanceof Error ? err.message : String(err));
    } finally { setDeleting(false); }
  }
  const groups = new Map<string, RunSummary[]>();
  for (const run of runs) {
    const group = groups.get(run.project) ?? [];
    group.push(run);
    groups.set(run.project, group);
  }
  return (
    <aside className="flex min-h-0 flex-col gap-3 border-r p-3">
      <Button variant="outline" size="sm" disabled={disabled} onClick={onNew}>
        <Plus className="size-4" /> New backtest
      </Button>
      <div className="flex items-center justify-between text-xs text-muted-foreground">
        <span>Strategy projects</span>
        {loading ? <Loader2 className="size-3 animate-spin" aria-label="Loading history" /> : <span>{runs.length} runs</span>}
      </div>
      {error && <div className="text-xs text-destructive">{error}<Button variant="ghost" size="sm" onClick={onRetry}>Retry</Button></div>}
      {!loading && !error && runs.length === 0 && (
        <p className="text-xs leading-relaxed text-muted-foreground">Your runs will appear here, grouped by project across pairs and timeframes.</p>
      )}
      <nav className="-mx-1 min-h-0 flex-1 overflow-auto" aria-label="Backtest history">
        {Array.from(groups, ([project, entries]) => (
          <details key={project} open className="mb-3">
            <summary className="group relative cursor-pointer rounded-md py-2 pl-2 pr-8 text-sm hover:bg-muted">
              <span className="inline-flex max-w-[85%] items-center gap-2 align-middle">
                <Folder className="size-3.5 shrink-0 text-muted-foreground" />
                <span className="truncate" title={project}>{project}</span>
                <span className="text-xs text-muted-foreground">{entries.length}</span>
              </span>
              <Button type="button" variant="ghost" size="icon-xs" disabled={disabled || deleting}
                className="absolute right-1 top-2 text-muted-foreground hover:text-destructive"
                aria-label={`Delete project ${project}`} title={`Delete ${project} and its ${entries.length} runs`}
                onClick={(event) => { event.preventDefault(); event.stopPropagation(); confirm(project, entries.map((run) => run.id), true); }}>
                <Trash2 className="size-3.5" />
              </Button>
            </summary>
            <div className="ml-3 space-y-1 border-l pl-2">
              {entries.map((run) => (
                <div key={run.id} className={`group flex items-start rounded-md hover:bg-muted ${selected === run.id ? "bg-muted" : ""}`}>
                <button
                  type="button"
                  disabled={disabled || deleting}
                  onClick={() => onOpen(run.id)}
                  aria-current={selected === run.id ? "true" : undefined}
                  className="min-w-0 flex-1 rounded-md px-2 py-2 text-left disabled:opacity-50"
                >
                  <div className="truncate text-xs font-medium" title={run.strategy}>{run.strategy}</div>
                  <div className="mt-1 truncate font-mono text-[11px] text-muted-foreground">{run.dataset || "Archived report"}</div>
                  <div className="mt-1 flex items-center justify-between gap-1 text-[10px] text-muted-foreground">
                    <time dateTime={new Date(run.created).toISOString()}>{new Date(run.created).toLocaleString(undefined, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" })}</time>
                    {run.status === "failed" ? <span className="text-destructive">Failed</span> : run.return_pct !== null && <span className={run.return_pct >= 0 ? "text-emerald-600 dark:text-emerald-500" : "text-destructive"}>{run.return_pct.toFixed(2)}%</span>}
                  </div>
                </button>
                <Button type="button" variant="ghost" size="icon-xs" disabled={disabled || deleting}
                  className="mt-1.5 mr-1 shrink-0 text-muted-foreground hover:text-destructive"
                  aria-label={`Delete ${run.strategy} backtest`} title="Delete backtest"
                  onClick={() => confirm(run.strategy, [run.id], false)}><Trash2 className="size-3.5" /></Button>
                </div>
              ))}
            </div>
          </details>
        ))}
      </nav>
      <Dialog open={pending !== null} onOpenChange={(open) => { if (!open && !deleting) setPending(null); }}>
        <DialogContent showCloseButton={!deleting}>
          <DialogHeader>
            <DialogTitle>{pending?.group ? "Delete project?" : "Delete backtest?"}</DialogTitle>
            <DialogDescription>
              {pending?.group
                ? `Delete “${pending.title}” and all ${pending.ids.length} saved backtests shown in this group?`
                : `Delete the saved backtest “${pending?.title}”?`}
              {" "}This permanently removes the saved results, reports and run snapshots. Your saved strategies and candle datasets are kept.
            </DialogDescription>
          </DialogHeader>
          {deleteError && <p role="alert" className="text-sm text-destructive">{deleteError}</p>}
          <DialogFooter>
            <Button variant="outline" disabled={deleting} onClick={() => setPending(null)}>Cancel</Button>
            <Button variant="destructive" disabled={deleting || disabled} onClick={remove}>
              {deleting ? <Loader2 className="size-4 animate-spin" /> : <Trash2 className="size-4" />}
              {pending?.group ? "Delete project and runs" : "Delete backtest"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </aside>
  );
}
