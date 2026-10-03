import { useEffect, useRef, useState } from "react";
import { newSessionId } from "@/lib/session";
import {
  api,
  type ChatEvent,
  type ConversationSummary,
  type Message,
} from "@/lib/api";
import {
  Conversation,
  ConversationContent,
  ConversationScrollButton,
} from "@/components/ai-elements/conversation";
import { Message as Bubble, MessageContent, MessageResponse } from "@/components/ai-elements/message";
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
import { Check, Loader2, Plus, Save, Send, Trash2, X } from "lucide-react";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
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

/** Strategy code open in the right-hand panel, editable before it is saved. */
type Draft = { name: string; code: string; saved: boolean };

/** The code a write_strategy call carried, once its arguments are complete JSON. */
function draftOf(part: Part): Draft | null {
  if (part.kind !== "tool" || part.name !== "write_strategy") return null;
  const args = safeJson(part.args);
  if (typeof args !== "object" || args === null) return null;
  const { name, code } = args as { name?: unknown; code?: unknown };
  if (typeof code !== "string") return null;
  return { name: typeof name === "string" ? name : "", code, saved: false };
}

function titleOf(parts: Part[]) {
  const first = parts.find((p) => p.kind === "user");
  return first ? first.text.slice(0, 60) : "New chat";
}

export function Chat({ onChanged }: { onChanged: () => void }) {
  const [models, setModels] = useState<string[]>([DEFAULT_MODEL]);
  const [model, setModel] = useState(DEFAULT_MODEL);
  const [parts, setPartsState] = useState<Part[]>([]);
  const [input, setInput] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [convos, setConvos] = useState<ConversationSummary[]>([]);
  const [draft, setDraft] = useState<Draft | null>(null);
  const [saving, setSaving] = useState(false);
  // One id for the life of this conversation: Go caches prompts against it, and
  // the API saves the conversation under it.
  const [session, setSession] = useState<string>(newSessionId);
  // The API-shaped history, kept separately from what we render.
  const history = useRef<Message[]>([]);
  // Mirrors `parts` synchronously, so the save after a turn sees every delta.
  const partsRef = useRef<Part[]>([]);

  function setParts(next: Part[] | ((prev: Part[]) => Part[])) {
    partsRef.current = typeof next === "function" ? next(partsRef.current) : next;
    setPartsState(partsRef.current);
  }

  function refreshList() {
    api.conversations().then(setConvos).catch(() => undefined);
  }

  useEffect(() => {
    api.models().then(setModels).catch(() => undefined);
    refreshList();
  }, []);

  /** Saved server-side, so a conversation outlives the tab and the browser. */
  async function persist(id: string) {
    try {
      await api.saveConversation(id, {
        title: titleOf(partsRef.current),
        model,
        parts: partsRef.current,
        history: history.current,
      });
      refreshList();
    } catch (err) {
      setError(`couldn't save the conversation: ${err instanceof Error ? err.message : err}`);
    }
  }

  function reset(id: string, next: Part[], messages: Message[]) {
    setSession(id);
    setParts(next);
    history.current = messages;
    setError(null);
    // Reopening a chat reopens the last strategy it wrote.
    setDraft(next.map(draftOf).filter((d) => d !== null).at(-1) ?? null);
  }

  async function open(id: string) {
    if (busy || id === session) return;
    try {
      const c = await api.conversation<Part>(id);
      reset(c.id, c.parts ?? [], c.history ?? []);
      if (c.model) setModel(c.model);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }

  async function remove(id: string) {
    await api.deleteConversation(id).catch(() => undefined);
    if (id === session) reset(newSessionId(), [], []);
    refreshList();
  }

  async function saveDraft() {
    if (!draft) return;
    setSaving(true);
    try {
      await api.saveStrategy(draft.name.trim(), draft.code);
      setDraft({ ...draft, saved: true });
      onChanged(); // Backtest's "load saved…" list picks it up
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSaving(false);
    }
  }

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
      case "tool_result": {
        history.current.push(e.message);
        // The agent wrote a strategy: split the screen and show it.
        const call = partsRef.current.find(
          (p) => p.kind === "tool" && p.id === e.message.tool_call_id,
        );
        const written = call && draftOf(call);
        if (written) setDraft(written);
        return setParts((p) =>
          p.map((part) =>
            part.kind === "tool" && part.id === e.message.tool_call_id
              ? { ...part, output: e.message.content ?? "" }
              : part,
          ),
        );
      }
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
    // Captured: the turn belongs to this conversation. Saved now so it shows
    // in the list straight away, and again once the turn ends.
    const id = session;
    persist(id);

    try {
      for await (const e of api.chat(model, id, history.current)) apply(e);
      onChanged(); // the agent may have written a strategy or run a backtest
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
      persist(id);
    }
  }

  return (
    <div
      className={
        draft
          ? "grid h-full min-h-0 grid-cols-[14rem_minmax(0,1fr)_minmax(0,1fr)]"
          : "grid h-full min-h-0 grid-cols-[14rem_minmax(0,1fr)]"
      }
    >
      <aside className="flex min-h-0 flex-col gap-2 border-r p-3">
        <Button
          variant="outline"
          size="sm"
          disabled={busy}
          onClick={() => reset(newSessionId(), [], [])}
        >
          <Plus className="size-4" /> New chat
        </Button>
        <nav className="-mx-1 min-h-0 flex-1 overflow-auto">
          {convos.map((c) => (
            <div
              key={c.id}
              className={`group flex items-center rounded-md px-1 hover:bg-muted ${
                c.id === session ? "bg-muted" : ""
              }`}
            >
              <button
                type="button"
                disabled={busy}
                onClick={() => open(c.id)}
                title={c.title}
                className="min-w-0 flex-1 truncate px-1 py-1.5 text-left text-sm disabled:opacity-50"
              >
                {c.title || "Untitled"}
              </button>
              <Button
                variant="ghost"
                size="icon-xs"
                disabled={busy && c.id === session}
                onClick={() => remove(c.id)}
                className="opacity-0 group-hover:opacity-100 focus-visible:opacity-100"
                aria-label="Delete conversation"
              >
                <Trash2 className="size-3.5" />
              </Button>
            </div>
          ))}
        </nav>
      </aside>

      <div className="mx-auto flex h-full min-h-0 w-full max-w-3xl flex-col gap-3 p-4">
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
                const written = draftOf(part);
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
                      {written && (
                        <Button
                          variant="outline"
                          size="sm"
                          className="mx-4 mb-3"
                          onClick={() => setDraft(written)}
                        >
                          Open in code panel
                        </Button>
                      )}
                    </ToolContent>
                  </Tool>
                );
              }
              return (
                <Bubble from="assistant" key={i}>
                  <MessageContent>
                    <MessageResponse isAnimating={busy && i === parts.length - 1}>
                      {part.text}
                    </MessageResponse>
                  </MessageContent>
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

      {/* The agent already wrote this to disk; Save keeps your edits, or a
          copy under a new name, for the Backtest screen's picker. */}
      {draft && (
        <section className="flex min-h-0 flex-col border-l">
          <div className="flex items-center gap-2 border-b px-3 py-2">
            <Input
              value={draft.name}
              onChange={(e) => setDraft({ ...draft, name: e.target.value, saved: false })}
              placeholder="strategy name"
              aria-label="Strategy name"
              className="h-7 max-w-56 font-mono text-xs"
            />
            <Button
              size="sm"
              disabled={saving || !draft.name.trim() || !draft.code.trim()}
              onClick={saveDraft}
            >
              {draft.saved ? <Check className="size-4" /> : <Save className="size-4" />}
              {draft.saved ? "Saved" : "Save"}
            </Button>
            <span className="min-w-0 flex-1 truncate text-xs text-muted-foreground">
              {draft.saved && "Load it on Backtest → load saved…"}
            </span>
            <Button
              variant="ghost"
              size="icon-sm"
              onClick={() => setDraft(null)}
              aria-label="Close code panel"
            >
              <X className="size-4" />
            </Button>
          </div>
          <textarea
            value={draft.code}
            onChange={(e) => setDraft({ ...draft, code: e.target.value, saved: false })}
            spellCheck={false}
            className="min-h-0 flex-1 resize-none bg-transparent p-4 font-mono text-[13px] leading-relaxed outline-none"
          />
        </section>
      )}
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
