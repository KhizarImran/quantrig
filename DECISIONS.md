# quantrig — scoping decisions

AI-driven harness for FX/futures algo trading. Agent writes strategies,
`backtestingfx` runs them, user goes live on their own broker account.
Self-hosted, open source. Name: PyPI + GitHub org both free; `quantrig.dev` free.

## Shape

Jesse's model (jesse.trade), not a CLI tool:

- **Engine repo** — pip package + pre-built dashboard, published as a Docker image.
- **Template repo** — what users clone: `strategies/`, `.env.example`, `docker/compose.yml`.
  `git clone && docker compose up` → `localhost:9000`.

`backtestingfx` ships manylinux wheels (x86_64 + aarch64), so the image needs no
Rust toolchain.

## Architecture

One container, not Jesse's three. SQLite for config + run history, parquet for
candles, in-process asyncio queue + WebSocket for progress.
`# ponytail: add postgres/redis when concurrent users demand it.`

**Split fetcher from sandbox — non-negotiable:**

```
[fetcher]  our code, holds API keys, network allowed → parquet
[sandbox]  agent-written strategy code, NO network, NO secrets → results
[api]      stores results, streams to UI
```

Strategy code is LLM-written and untrusted. It must never share a process with
customer credentials. Retrofitting this means rewriting the runner.

Everything is a client of the HTTP/WS API — web UI, MCP server, future TUI.
Nothing touches SQLite or the engine directly.

bind to `127.0.0.1`, password-gate, envelope-encrypt keys at rest.

## Stack

Rust everywhere except the strategy layer.

| Component | Crate |
| --- | --- |
| api (HTTP + WS) | axum + tokio |
| SQLite (config, run history) | sqlx or rusqlite |
| fetcher (keys, network → parquet) | reqwest + arrow/parquet |
| sandbox supervisor | spawn under `bwrap` — no isolation code of our own |
| MCP server | rmcp (official Rust SDK) |
| TUI (later) | ratatui |
| MT5 socket bridge (later) | tokio |

**Python lives only inside the sandbox** — the agent-written strategies, plus a
thin runner that calls `backtestingfx` and writes results. ~100 lines, written once.

**TypeScript is the three screens.** Nothing else.

The language boundary is free because the sandbox is a separate process anyway —
it sits exactly where the security boundary already is. Spawn Python, read
results. `# ponytail: no pyo3, no FFI. Add when the subprocess is measurably the
bottleneck — it won't be.`

Strategies stay Python and that is not negotiable. An agent loop that compiles
Rust per iteration is dead on latency, and `backtestingfx`'s surface is Python.
`backtestingfx` having a Rust core is not a reason to write strategies in Rust —
that trades the product premise for one less runtime.

## Bring-your-own-keys

Data and AI keys are always the customer's. We never redistribute market data
(vendor licences forbid it) and never resell tokens.

Caveat: Codex / Claude subscription logins are OAuth-bound to a local CLI.
They work self-hosted only. Hosted tier = API keys only.

## Agent hookup

MCP server in the container (`list_strategies`, `write_strategy`, `run_backtest`,
`get_results`, `import_data`). The user's own agent CLI connects from the host with
its own auth. We never handle their AI credentials.

## Build order

1. **Weeks 1–2** — backend + Docker, no UI. Driveable by curl.
2. **Week 3** — MCP server. Before the UI: if it's clean enough for an agent it's
   clean enough for a browser.
3. **Weeks 4–7** — web UI. Three screens: Config, Backtest, Results. Ugly is fine.
4. **Later** — live execution (MT5 bridge, reconciliation, kill switches). Months.
5. **Later** — Ratatui TUI, monitoring-only, for live on a headless VPS.
   Good-first-issue, not our work.

Cut: in-browser editor, LSP, Jupyter, genetic optimiser, multi-engine abstraction.

## Money

Backtesting free forever, **pay to go live** — highest willingness to pay is the
moment capital is at risk. Never meter research; it kills the agent loop.

Free self-hosted → ~£29–49 hosted research (bounded compute quota) → ~£79–149
live → desk tier. Verify against current comparables before committing.

Hosted = single-tenant managed instances of the same image, not multi-tenant SaaS.
Sidesteps data redistribution, untrusted-code isolation, and credential custody.

A licence flag in open-source code gets patched out. Sell a *service* (hosted
execution, monitoring) and keep the live module closed and server-validated.

Wedge: funded-account / prop-challenge traders. Cash-motivated, hard risk rules
an algo enforces better than a human, already paying for challenges. Our kill
switches map 1:1 onto their rules.

Licence the core AGPL-3.0 or BSL before the first outside PR — MIT lets someone
host it commercially, and relicensing later needs every contributor's signature.

## Open

- MT5 is Windows-only; Linux needs Wine or an MQL5 socket bridge. Unresolved.
- Backtest→live semantic drift (spread, slippage, partial fills, swap) is where
  these projects die. One order-intent abstraction shared by both paths.
- Walk-forward / out-of-sample enforced in the runner from v0, so an agent
  cannot report a curve-fit result as a win.
