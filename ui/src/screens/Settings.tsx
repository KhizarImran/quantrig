import { useCallback, useEffect, useState } from "react";
import { Bot, Check, Database, ExternalLink, Loader2, ShieldCheck } from "lucide-react";
import { ChatGPTConnection } from "@/components/chatgpt-connection";
import { api, type KeyName, type SettingsState } from "@/lib/api";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

type Connector = {
  id: string;
  category: "ai" | "data";
  title: string;
  description: string;
} & (
  | { kind: "api-key"; keyName: KeyName; website: string }
  | { kind: "chatgpt" }
);

const CONNECTORS: Connector[] = [
  {
    id: "opencode",
    category: "ai",
    title: "OpenCode Go",
    description: "Write strategies, run backtests, and explore results with the chat assistant.",
    kind: "api-key",
    keyName: "opencode_api_key",
    website: "https://opencode.ai/go",
  },
  {
    id: "chatgpt",
    category: "ai",
    title: "ChatGPT",
    description: "Connect your ChatGPT subscription to the trading assistant.",
    kind: "chatgpt",
  },
  {
    id: "lse",
    category: "data",
    title: "London Strategic Edge",
    description: "Download historical market candles for your strategies and backtests.",
    kind: "api-key",
    keyName: "lse_api_key",
    website: "https://londonstrategicedge.com/data",
  },
];

const SECTIONS = [
  { id: "ai", title: "AI", description: "Choose the services that power your assistant.", icon: Bot },
  { id: "data", title: "Data", description: "Connect market data sources for research and backtesting.", icon: Database },
] as const;

function KeyField({
  name,
  isSet,
  onSaved,
}: {
  name: KeyName;
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
          API key
        </Label>
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
      {error && (
        <Alert variant="destructive">
          <AlertDescription>{error}</AlertDescription>
        </Alert>
      )}
    </form>
  );
}

function ConnectorCard({ connector, state, onSaved }: {
  connector: Connector;
  state: SettingsState | undefined;
  onSaved: () => void;
}) {
  if (connector.kind === "chatgpt") return <ChatGPTConnection onChanged={onSaved} />;
  const isSet = state?.[`${connector.keyName}_set`];
  const Icon = connector.category === "ai" ? Bot : Database;

  return (
    <Card className="h-full">
      <CardHeader>
        <div className="flex items-start justify-between gap-3">
          <div className="flex size-10 items-center justify-center rounded-xl border bg-muted/50 text-muted-foreground">
            <Icon className="size-5" />
          </div>
          <Badge variant="outline" className={isSet ? "border-emerald-500/25 bg-emerald-500/10 text-emerald-500" : "text-muted-foreground"}>
            {isSet === undefined ? "Loading…" : isSet ? <><Check /> Configured</> : "Not configured"}
          </Badge>
        </div>
        <CardTitle className="mt-2">{connector.title}</CardTitle>
        <CardDescription>{connector.description}</CardDescription>
      </CardHeader>
      <CardContent className="mt-auto space-y-4">
        <KeyField name={connector.keyName} isSet={isSet} onSaved={onSaved} />
        <a href={connector.website} target="_blank" rel="noopener noreferrer" className="inline-flex items-center gap-1.5 text-xs text-muted-foreground underline-offset-4 hover:text-foreground hover:underline">
          Get an API key <ExternalLink className="size-3" />
        </a>
      </CardContent>
    </Card>
  );
}

export function Settings({ onKeySaved }: { onKeySaved: () => void }) {
  const [state, setState] = useState<SettingsState | undefined>();
  const [error, setError] = useState<string | null>(null);

  const reload = useCallback(() => api.settings().then((settings) => {
    setState(settings);
    setError(null);
  }).catch((err: unknown) => {
    setError(err instanceof Error ? err.message : String(err));
  }), []);

  useEffect(() => { void reload(); }, [reload]);

  function onSaved() {
    void reload();
    onKeySaved();
  }

  return (
    <div className="mx-auto w-full max-w-5xl space-y-8 p-4 sm:p-6">
      <div className="space-y-1">
        <h1 className="text-xl font-semibold tracking-tight">Connections</h1>
        <p className="text-sm text-muted-foreground">Manage your AI services and market data sources.</p>
      </div>
      {error && (
        <Alert variant="destructive">
          <AlertDescription className="flex flex-wrap items-center justify-between gap-2">
            <span>Could not load connections: {error}</span>
            <Button variant="outline" size="sm" onClick={() => void reload()}>Retry</Button>
          </AlertDescription>
        </Alert>
      )}
      {SECTIONS.map(({ id, title, description, icon: Icon }) => (
        <section key={id} aria-labelledby={`connections-${id}`} className="space-y-4">
          <div className="flex items-center gap-3">
            <Icon className="size-4 text-muted-foreground" />
            <div>
              <h2 id={`connections-${id}`} className="text-sm font-semibold">{title}</h2>
              <p className="text-xs text-muted-foreground">{description}</p>
            </div>
          </div>
          <div className="grid gap-4 md:grid-cols-2">
            {CONNECTORS.filter((connector) => connector.category === id).map((connector) => (
              <ConnectorCard key={connector.id} connector={connector} state={state} onSaved={onSaved} />
            ))}
          </div>
        </section>
      ))}
      <div className="flex items-start gap-2 border-t pt-4 text-xs leading-relaxed text-muted-foreground">
        <ShieldCheck className="mt-0.5 size-4 shrink-0" />
        <p>Credentials are stored on this server, excluded from strategy environments, and never returned by the settings API. Configured means a key is saved; it does not verify access to the service.</p>
      </div>
    </div>
  );
}
