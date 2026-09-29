import { useEffect, useRef, useState } from "react";
import { api, type ChatEvent, type Message } from "@/lib/api";
import {
  Conversation,
  ConversationContent,
  ConversationScrollButton,
} from "@/components/ai-elements/conversation";
import { Message as Bubble, MessageContent } from "@/components/ai-elements/message";
import {
  Reasoning,
  ReasoningContent,
  ReasoningTrigger,
} from "@/components/ai-elements/reasoning";
import {
  Tool,
  ToolContent,
  ToolHeader,
  ToolInput,
  ToolOutput,
} from "@/components/ai-elements/tool";
import { Loader2, Send } from "lucide-react";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";

const DEFAULT_MODEL = "glm-5.3";

/** What the transcript is made of: the stream rebuilt into renderable parts. */
type Part =
  | { kind: "user"; text: string }
  | { kind: "text"; text: string }
  | { kind: "reasoning"; text: string }
  | { kind: "tool"; id: string; name: string; args: string; output?: string };

export function Chat({ onChanged }: { onChanged: () => void }) {
  const [models, setModels] = useState<string[]>([DEFAULT_MODEL]);
  const [model, setModel] = useState(DEFAULT_MODEL);
  const [parts, setParts] = useState<Part[]>([]);
  const [input, setInput] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // One id for the life of this conversation: Go caches prompts against it.
  const session = useRef(crypto.randomUUID());
  // The API-shaped history, kept separately from what we render.
  const history = useRef<Message[]>([]);

  useEffect(() => {
    api.models().then(setModels).catch(() => undefined);
  }, []);

  /** Append to the last part when it is the same kind, so deltas coalesce. */
  function appendDelta(kind: "text" | "reasoning", delta: string) {
    setParts((prev) => {
      const last = prev[prev.length - 1];
      if (last?.kind === kind) {
        return [...prev.slice(0, -1), { ...last, text: last.text + delta }];
      }
      return [...prev, { kind, text: delta }];
    });
  }

  function apply(e: ChatEvent) {
    switch (e.type) {
      case "text":
      case "reasoning":
        return appendDelta(e.type, e.delta);
      case "tool":
        return setParts((p) => [
          ...p,
          { kind: "tool", id: e.id, name: e.name, args: e.arguments },
        ]);
      case "tool_result":
        history.current.push(e.message);
        return setParts((p) =>
          p.map((part) =>
            part.kind === "tool" && part.id === e.message.tool_call_id
              ? { ...part, output: e.message.content ?? "" }
              : part,
          ),
        );
      case "message":
        return history.current.push(e.message);
      case "error":
        return setError(e.error);
      case "done":
        return;
    }
  }

  async function send(text: string) {
    if (!text.trim() || busy) return;
    setInput("");
    setError(null);
    setBusy(true);
    setParts((p) => [...p, { kind: "user", text }]);
    history.current.push({ role: "user", content: text });

    try {
      for await (const e of api.chat(model, session.current, history.current)) apply(e);
      onChanged(); // the agent may have written a strategy or run a backtest
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="mx-auto flex h-full w-full max-w-3xl flex-col gap-3 p-4">
      <div className="flex items-center justify-between gap-4">
        <p className="text-sm text-muted-foreground">
          The agent writes strategies and runs them. It sees its own results.
        </p>
        <Select value={model} onValueChange={(v) => v && setModel(v)}>
          <SelectTrigger size="sm" className="w-56 font-mono text-xs">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {models.map((m) => (
              <SelectItem key={m} value={m} className="font-mono text-xs">
                {m}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      <Conversation className="min-h-0 flex-1 rounded-lg border">
        <ConversationContent>
          {parts.length === 0 && !busy && (
            <div className="py-16 text-center">
              <p className="font-medium">Ask for a strategy</p>
              <p className="mt-1 text-sm text-muted-foreground">
                “Write a mean-reversion strategy and backtest it on EUR_USD@1h.”
              </p>
            </div>
          )}

          {parts.map((part, i) => {
            if (part.kind === "user") {
              return (
                <Bubble from="user" key={i}>
                  <MessageContent>{part.text}</MessageContent>
                </Bubble>
              );
            }
            if (part.kind === "reasoning") {
              return (
                <Reasoning key={i} isStreaming={busy && i === parts.length - 1}>
                  <ReasoningTrigger />
                  <ReasoningContent>{part.text}</ReasoningContent>
                </Reasoning>
              );
            }
            if (part.kind === "tool") {
              return (
                <Tool key={part.id || i}>
                  <ToolHeader
                    type={`tool-${part.name}`}
                    state={part.output === undefined ? "input-available" : "output-available"}
                  />
                  <ToolContent>
                    <ToolInput input={safeJson(part.args)} />
                    {part.output !== undefined && (
                      <ToolOutput errorText={undefined} output={part.output} />
                    )}
                  </ToolContent>
                </Tool>
              );
            }
            return (
              <Bubble from="assistant" key={i}>
                <MessageContent>{part.text}</MessageContent>
              </Bubble>
            );
          })}
        </ConversationContent>
        <ConversationScrollButton />
      </Conversation>

      {error && (
        <Alert variant="destructive">
          <AlertDescription>{error}</AlertDescription>
        </Alert>
      )}

      {/* ponytail: a textarea and a button. AI Elements' prompt-input carries
          attachments, command menus and model pickers we do not use. */}
      <form
        onSubmit={(e) => {
          e.preventDefault();
          send(input);
        }}
        className="flex gap-2"
      >
        <textarea
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              send(input);
            }
          }}
          rows={2}
          placeholder="Ask for a strategy, or a change to one…"
          className="min-h-0 flex-1 resize-none rounded-md border bg-transparent px-3 py-2 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring"
        />
        <Button type="submit" disabled={busy || !input.trim()}>
          {busy ? <Loader2 className="size-4 animate-spin" /> : <Send className="size-4" />}
        </Button>
      </form>
    </div>
  );
}

/** Tool arguments stream in as text and may still be partial JSON. */
function safeJson(raw: string): unknown {
  try {
    return JSON.parse(raw);
  } catch {
    return raw;
  }
}
