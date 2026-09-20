import { useEffect, useState } from "react";
import { Check, KeyRound, Loader2 } from "lucide-react";
import { api } from "@/lib/api";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

export function Settings({ onKeySaved }: { onKeySaved: () => void }) {
  const [isSet, setIsSet] = useState<boolean | null>(null);
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    api.settings().then((s) => setIsSet(s.lse_api_key_set)).catch(() => setIsSet(false));
  }, []);

  async function save(e: React.FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await api.saveKey(value);
      setIsSet(true);
      setSaved(true);
      setValue("");
      onKeySaved();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="mx-auto w-full max-w-2xl p-6">
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <KeyRound className="size-4" />
            Environment variables
          </CardTitle>
          <CardDescription>
            Stored on this machine only. Keys never reach a strategy — the sandbox runs with
            a cleared environment.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <form onSubmit={save} className="space-y-4">
            <div className="space-y-2">
              <div className="flex items-center justify-between">
                <Label htmlFor="lse-key" className="font-mono text-xs">
                  LSE_API_KEY
                </Label>
                {isSet !== null &&
                  (isSet ? (
                    <span className="flex items-center gap-1 text-xs text-emerald-600 dark:text-emerald-500">
                      <Check className="size-3" /> set
                    </span>
                  ) : (
                    <span className="text-xs text-muted-foreground">not set</span>
                  ))}
              </div>
              <Input
                id="lse-key"
                type="password"
                autoComplete="off"
                spellCheck={false}
                placeholder={isSet ? "•••••••• — enter a new key to replace" : "paste your key"}
                value={value}
                onChange={(e) => setValue(e.target.value)}
                className="font-mono"
              />
              <p className="text-xs text-muted-foreground">
                London Strategic Edge — free key at londonstrategicedge.com/data. Used to
                download candles; it is never readable back through the API.
              </p>
            </div>

            {error && (
              <Alert variant="destructive">
                <AlertDescription>{error}</AlertDescription>
              </Alert>
            )}
            {saved && !error && (
              <Alert>
                <AlertDescription>Key saved.</AlertDescription>
              </Alert>
            )}

            <Button type="submit" disabled={busy || !value.trim()}>
              {busy && <Loader2 className="size-4 animate-spin" />}
              Save key
            </Button>
          </form>
        </CardContent>
      </Card>
    </div>
  );
}
