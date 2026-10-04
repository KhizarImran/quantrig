import { Fragment, useEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import { createHighlighter } from "shiki";

// Load the Python grammar once, shared by every mounted editor.
let highlighterPromise: ReturnType<typeof createHighlighter> | undefined;
function loadHighlighter() {
  return highlighterPromise ??= createHighlighter({
    langs: ["python"],
    themes: ["dark-plus", "light-plus"],
  });
}

export function PythonEditor({ value, onChange }: {
  value: string;
  onChange: (value: string) => void;
}) {
  const [highlighter, setHighlighter] = useState<Awaited<ReturnType<typeof createHighlighter>>>();
  const highlightLayer = useRef<HTMLPreElement>(null);

  useEffect(() => {
    let active = true;
    loadHighlighter().then((loaded) => {
      if (active) setHighlighter(loaded);
    }).catch(() => undefined); // Plain text remains editable if loading fails.
    return () => { active = false; };
  }, []);

  const tokens = useMemo(() => highlighter?.codeToTokens(value, {
    lang: "python",
    themes: { light: "light-plus", dark: "dark-plus" },
  }).tokens, [highlighter, value]);

  return (
    <div className="python-editor relative min-h-0 flex-1 overflow-hidden bg-muted/20">
      <pre
        ref={highlightLayer}
        aria-hidden="true"
        className="python-editor-surface pointer-events-none absolute inset-0 overflow-hidden"
      >
        <code>
          {tokens ? tokens.map((line, lineIndex) => (
            <Fragment key={lineIndex}>
              {line.map((token, tokenIndex) => (
                <span key={tokenIndex} style={{ color: token.color, ...token.htmlStyle } as CSSProperties}>
                  {token.content}
                </span>
              ))}
              {lineIndex < tokens.length - 1 ? "\n" : ""}
            </Fragment>
          )) : value}
          {"\n"}
        </code>
      </pre>
      <textarea
        value={value}
        onChange={(event) => onChange(event.target.value)}
        onScroll={(event) => {
          if (highlightLayer.current) {
            highlightLayer.current.scrollTop = event.currentTarget.scrollTop;
            highlightLayer.current.scrollLeft = event.currentTarget.scrollLeft;
          }
        }}
        onKeyDown={(event) => {
          if (event.key !== "Tab" || event.shiftKey) return;
          event.preventDefault();
          const input = event.currentTarget;
          input.setRangeText("    ", input.selectionStart, input.selectionEnd, "end");
          onChange(input.value);
        }}
        aria-label="Python strategy source"
        spellCheck={false}
        autoCapitalize="off"
        autoCorrect="off"
        wrap="off"
        className="python-editor-surface relative size-full resize-none bg-transparent text-transparent caret-foreground outline-none"
      />
    </div>
  );
}
