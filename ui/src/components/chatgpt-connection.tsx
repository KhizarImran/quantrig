import { useEffect, useState } from "react";
import { Check, ExternalLink, Loader2, Sparkles } from "lucide-react";
import { api, type ChatGPTStatus } from "@/lib/api";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";

export function ChatGPTConnection({ onChanged }: { onChanged: () => void }) {
  const [status, setStatus] = useState<ChatGPTStatus>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [warning, setWarning] = useState<string | null>(null);
  const [url, setUrl] = useState<string | null>(null);
  const remote = !["localhost", "127.0.0.1", "[::1]"].includes(window.location.hostname);

  useEffect(() => {
    let cancelled = false;
    api.chatgptStatus().then((next) => { if (!cancelled) setStatus(next); })
      .catch((err: unknown) => { if (!cancelled) setError(err instanceof Error ? err.message : String(err)); });
    return () => { cancelled = true; };
  }, []);

  useEffect(() => {
    if (!status?.pending) return;
    let cancelled = false;
    const timer = window.setInterval(() => {
      api.chatgptStatus().then((next) => {
        if (cancelled) return;
        setStatus(next);
        if (!next.pending) {
          setUrl(null);
          onChanged();
        }
      }).catch((err: unknown) => { if (!cancelled) setError(err instanceof Error ? err.message : String(err)); });
    }, 1500);
    return () => { cancelled = true; window.clearInterval(timer); };
  }, [status?.pending, onChanged]);

  async function signIn() {
    // Open synchronously so browser popup rules allow the authorization tab.
    const popup = window.open("about:blank", "_blank");
    if (popup) popup.opener = null;
    setBusy(true); setError(null); setWarning(null);
    try {
      const flow = await api.chatgptSignIn();
      setUrl(flow.url);
      setStatus(await api.chatgptStatus());
      if (popup) popup.location.href = flow.url;
    } catch (err) {
      popup?.close();
      setError(err instanceof Error ? err.message : String(err));
    } finally { setBusy(false); }
  }

  async function disconnect() {
    setBusy(true); setError(null); setWarning(null);
    try {
      if (status?.pending) { await api.chatgptCancel(); }
      else { const result = await api.chatgptDisconnect(); setWarning(result.warning); }
      setStatus(await api.chatgptStatus()); setUrl(null);
      onChanged();
    } catch (err) { setError(err instanceof Error ? err.message : String(err)); }
    finally { setBusy(false); }
  }

  return (
    <Card className="h-full">
      <CardHeader>
        <div className="flex items-start justify-between gap-3">
          <div className="flex size-10 items-center justify-center rounded-xl border bg-muted/50 text-muted-foreground"><Sparkles className="size-5" /></div>
          <Badge variant="outline" className={status?.connected ? "border-emerald-500/25 bg-emerald-500/10 text-emerald-500" : "text-muted-foreground"}>
            {!status ? "Loading…" : status.pending ? "Awaiting sign-in" : status.needs_sign_in ? "Sign in again" : status.connected ? <><Check /> Connected</> : "Not connected"}
          </Badge>
        </div>
        <CardTitle className="mt-2">ChatGPT</CardTitle>
        <CardDescription>Use your eligible ChatGPT Plus or Pro plan for the trading assistant.</CardDescription>
      </CardHeader>
      <CardContent className="mt-auto space-y-4">
        {status?.connected && (
          <div className="space-y-1 rounded-lg border bg-muted/20 p-3 text-xs">
            <p className="break-all font-medium">{status.email || "ChatGPT account"}</p>
            <p className="text-muted-foreground">{status.plan_enabled ? "Plan usage enabled. Choose ChatGPT in the chat provider menu." : "Signed in, but plan usage was not enabled. Continue with ChatGPT below to allow it."}</p>
          </div>
        )}
        {remote && !status?.connected && (
          <details className="rounded-lg border bg-muted/20 p-3 text-xs" open>
            <summary className="cursor-pointer font-medium">Remote server setup</summary>
            <div className="mt-3 space-y-2 text-muted-foreground">
              <p>Remote sign-in needs a temporary SSH port forward from your computer to this server’s callback on port 1455. Set it up in your SSH client or with your saved host alias, then select Continue with ChatGPT. Settings won’t display your SSH username or server address.</p>
              <p>Close the port forward after this page shows Connected. Token refresh works without it.</p>
            </div>
          </details>
        )}
        <p className="text-xs leading-relaxed text-muted-foreground">Eligible requests use your ChatGPT plan allowance. You can manage app usage and access in ChatGPT Settings. Your existing ChatGPT conversations are not imported.</p>
        <div className="flex flex-wrap items-center gap-2">
          <Button onClick={signIn} disabled={busy || !status || !status.callback_available || status.pending} variant={status?.connected ? "outline" : "default"}>
            {busy && <Loader2 className="size-4 animate-spin" />}
            Continue with ChatGPT
          </Button>
          {(status?.connected || status?.pending || status?.needs_sign_in) && <Button variant="ghost" disabled={busy} onClick={disconnect}>{status.pending ? "Cancel" : "Disconnect"}</Button>}
        </div>
        {url && <a href={url} target="_blank" rel="noopener noreferrer" className="inline-flex items-center gap-1.5 text-xs underline underline-offset-4">Open sign-in page <ExternalLink className="size-3" /></a>}
        {(error || status?.error) && <Alert variant="destructive"><AlertDescription>{error || status?.error}</AlertDescription></Alert>}
        {status && !status.callback_available && <Alert variant="destructive"><AlertDescription>The sign-in callback is unavailable. Check server port 1455 and restart Quantrig.</AlertDescription></Alert>}
        {warning && <Alert><AlertDescription>{warning}</AlertDescription></Alert>}
      </CardContent>
    </Card>
  );
}
