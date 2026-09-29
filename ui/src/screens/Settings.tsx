import { useEffect, useState } from "react";
import { Check, KeyRound, Loader2 } from "lucide-react";
import { api, type KeyName, type SettingsState } from "@/lib/api";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

const KEYS: { name: KeyName; label: string; hint: string }[] = [
  {
    name: "lse_api_key",
    label: "LSE_API_KEY",
    hint: "London Strategic Edge — free key at londonstrategicedge.com/data. Downloads candles.",
  },
  {
    name: "opencode_api_key",
    label: "OPENCODE_API_KEY",
    hint: "OpenCode Go — key from opencode.ai/go. Powers the chat agent.",
  },
];

function KeyField({
  name,
  label,
  hint,
  isSet,
  onSaved,
}: {
  name: KeyName;
  label: string;
  hint: string;
  isSet: boolean | undefined;
  onSaved: () => void;
}) {
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function save(e: React.FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await api.saveKey(name, value);
      setValue("");
      onSaved();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <form onSubmit={save} className="space-y-2">
      <div className="flex items-center justify-between">
        <Label htmlFor={name} className="font-mono text-xs">
          {label}
        </Label>
        {isSet !== undefined &&
          (isSet ? (
            <span className="flex items-center gap-1 text-xs text-emerald-600 dark:text-emerald-500">
              <Check className="size-3" /> set
            </span>
          ) : (
            <span className="text-xs text-muted-foreground">not set</span>
          ))}
      </div>
      <div className="flex gap-2">
        <Input
          id={name}
          type="password"
          autoComplete="off"
          spellCheck={false}
          placeholder={isSet ? "•••••••• — enter a new key to replace" : "paste your key"}
          value={value}
          onChange={(e) => setValue(e.target.value)}
          className="font-mono"
        />
        <Button type="submit" disabled={busy || !value.trim()}>
          {busy && <Loader2 className="size-4 animate-spin" />}
          Save
        </Button>
      </div>
      <p className="text-xs text-muted-foreground">{hint}</p>
      {error && (
        <Alert variant="destructive">
          <AlertDescription>{error}</AlertDescription>
        </Alert>
      )}
    </form>
  );
}

export function Settings({ onKeySaved }: { onKeySaved: () => void }) {
  const [state, setState] = useState<SettingsState | undefined>();

  const reload = () => {
    api.settings().then(setState).catch(() => undefined);
    onKeySaved();
  };
  useEffect(reload, []);

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
            a cleared environment — and are never readable back through the API.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-6">
          {KEYS.map((k) => (
            <KeyField
              key={k.name}
              {...k}
              isSet={state?.[`${k.name}_set`]}
              onSaved={reload}
            />
          ))}
        </CardContent>
      </Card>
    </div>
  );
}
