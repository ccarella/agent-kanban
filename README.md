# Agent Kanban

Keyboard-only terminal Kanban (Rust + Ratatui + Crossterm). **M3** adds a live In Progress spinner and readable errors/summaries in the board UI. Dispatch is still the M2 in-process `grok` runner (or stub).

Columns: **Capture · To Do · In Progress · Review · Done**

## Requirements

- **Rust 1.88+** (edition 2021; `cargo` on `PATH`)
  - MSRV is set in `Cargo.toml` as `rust-version = "1.88"`.
- For **live** dispatch: `grok` on `PATH` (or `AGENT_KANBAN_GROK`) and a valid local API key / grok auth.
- Cloud / CI VMs often have no grok auth — use the **stub** flag (below). Dogfood prefers live `grok` when auth is present.

## Run

```bash
cargo run
```

Writes **`./board.json`** in the process current working directory (JSON **array of cards**). Created on first save. `q` auto-saves.

Override the board path with `AGENT_KANBAN_BOARD`. Do not use `~/.agent-kanban/` — that path is not the default.

## Keyboard (M1 + M2)

| Key | Action |
| --- | --- |
| `j` / `↓`, `k` / `↑` | Select card in the focused column |
| `h` / `←` | Move focused card one column left (updates `status`) |
| `l` / `→` | Move focused card one column right (updates `status`) |
| `n` | Inline title → new card in **Capture** |
| `Enter` | Full-screen title/body editor |
| `r` | **Review only:** accept → **Done**, or revise (edit body) → **To Do** (`revision_count++`) |
| `q` | Quit and auto-save |
| `?` | Help overlay |

Cards show **title**. When `revision_count > 0`, a **rev N** badge is shown.

### Editor (Enter)

Usable for enriching **Capture → To Do** before the dispatcher picks the card.

| Key | Action |
| --- | --- |
| `Tab` | Title ↔ body |
| `↑` `↓` / `PgUp` `PgDn` | Move by line in the body |
| `Home` / `End` | Start / end of the current line (body) |
| `Ctrl+S` | Save (revise-from-Review also returns the card to To Do) |
| `Ctrl+T` | Save and move **Capture → To Do** |
| `Esc` | Cancel |

### Review (`r`)

On a card in **Review**:

- `a` or `Enter` — accept → **Done** (`agent_log` kind `accept`)
- `v` — edit title/body; `Ctrl+S` → **To Do**, `revision_count++` (`agent_log` kind `revise`)
- `Esc` — stay in Review

## Dispatcher (M2)

In-process **interval wake** (default 2s). No cron. Picks **one** `status=todo` card at a time:

1. Move to **In Progress**, append `agent_log` (`kind=dispatch`), persist.
2. Shell headless:

   ```bash
   grok -p "…" --cwd … --always-approve --json
   ```

   The prompt includes the card **title**, **body**, revision, and id. `--cwd` is `AGENT_KANBAN_CWD` or the process cwd.

3. **Success** (exit 0 + parseable JSON) → **Review**, `agent_log` kind `success`.
4. **Failure** (nonzero exit, missing `grok`, timeout, bad JSON, cancel) → **To Do**, `revision_count++`, `agent_log` kind `error`.

A card is **never** left in In Progress after the process ends: quit interrupts the child; the next launch **reclaims** any orphan In Progress cards back to To Do (rev + error log).

Failed cards are not immediately re-picked in the same session (cooldown). Edit, `Ctrl+T`, revise, or move the card with `h`/`l` to retry.

## M3 — live In Progress + readable errors

While a card is dispatched, the TUI shows a **spinner** on the In Progress card (and column title) and on the status line, plus elapsed seconds. The pulse comes from the **event loop tick** (~80ms while a job is running, otherwise ~200ms) — it updates without a keypress and without quit/relaunch.

After a run, the **latest error or summary** is readable in the board:

- Status line keeps the last dispatch result (`Agent failed … — <error>` or `Agent finished → Review — <summary>`).
- Detail line under the status shows the selected card’s latest `error` / `success` message.
- Failed cards show a red **`! …`** snippet under the title.

You do not need to open `./board.json` to see why a card failed.

Parallel runs, ACP, DB/sync, Chrome, and a streaming transcript UI are still out of scope.

## Dogfood (one-shot, live grok)

On a VM with **Rust 1.88+** and `grok` on `PATH` (authenticated):

```bash
cd "$(mktemp -d)"
cargo run --manifest-path /path/to/agent-kanban/Cargo.toml
```

1. `n` — title → **Capture**.
2. `Enter` — enrich the body / context, then `Ctrl+T` → **To Do**.
3. Wait for the dispatcher: card moves to **In Progress** with a live spinner on the card and status line.
4. **Success** → **Review**. Read the summary on the status/detail line.
5. `r` then `a` → **Done**. Or `r` then `v`, edit, `Ctrl+S` → **To Do** (`rev N`); the dispatcher will pick it again.
6. **Failure** → **To Do** with **rev N**, a `!` error snippet on the card, and the error on the status line. Edit or `h`/`l` to retry.

Point at another binary or working tree:

```bash
AGENT_KANBAN_GROK=/usr/local/bin/grok AGENT_KANBAN_CWD=/path/to/repo cargo run
```

## Stub / CI (no grok auth)

Stub path stays for CI and VMs without grok auth. Same status transitions, `agent_log`, and `revision_count` as live grok.

```bash
AGENT_KANBAN_STUB_DISPATCH=1 cargo run
```

| Env | Effect |
| --- | --- |
| `AGENT_KANBAN_STUB_DISPATCH=1` | Fake agent (success unless fail is requested) |
| `AGENT_KANBAN_STUB_FAIL=1` | Stub always fails |
| Card title/body contains `[stub:fail]` | That card’s stub run fails |
| `AGENT_KANBAN_STUB_DELAY_MS` | Stub pause so In Progress + spinner stay visible (default `500`; tests use `0`) |

### Smoke: stub

```bash
cd "$(mktemp -d)"
AGENT_KANBAN_STUB_DISPATCH=1 cargo run --manifest-path /path/to/agent-kanban/Cargo.toml
```

1. `n` create a card → Capture.
2. `Enter`, type a body, `Ctrl+T` → To Do.
3. Watch **In Progress**: spinner on the card and status line (default 500ms stub delay), then **Review**.
4. `r` then `a` → Done. Or `r` then `v`, edit, `Ctrl+S` → To Do with **rev 1**.

Force a fail (error visible in the TUI, not only JSON):

```bash
AGENT_KANBAN_STUB_DISPATCH=1 AGENT_KANBAN_STUB_FAIL=1 cargo run
```

Card returns to To Do with **rev N**, an `error` `agent_log` entry, a `!` snippet, and the error on the status/detail line. It will not auto-retry until you edit or move it.

Instant stub (CI-style, spinner may be a blink):

```bash
AGENT_KANBAN_STUB_DISPATCH=1 AGENT_KANBAN_STUB_DELAY_MS=0 cargo run
```

`cargo test` uses a zero-delay stub.

## Environment

| Variable | Default | Meaning |
| --- | --- | --- |
| `AGENT_KANBAN_BOARD` | `./board.json` | Board file |
| `AGENT_KANBAN_STUB_DISPATCH` | off | `1` / `true` / `yes` / `on` → stub dispatcher |
| `AGENT_KANBAN_STUB_FAIL` | off | Stub returns failure |
| `AGENT_KANBAN_STUB_DELAY_MS` | `500` | Stub pause (ms) so live In Progress is visible |
| `AGENT_KANBAN_DISPATCH` | on | `0` / `false` disables auto-dispatch |
| `AGENT_KANBAN_GROK` | `grok` | Dispatcher binary |
| `AGENT_KANBAN_CWD` | process cwd | Passed as `--cwd` |
| `AGENT_KANBAN_DISPATCH_INTERVAL_MS` | `2000` | Interval wake |
| `AGENT_KANBAN_DISPATCH_TIMEOUT_SECS` | `300` | Kill hung grok |

Truthy flags: `1`, `true`, `yes`, `on` (any case).

## Card model

Each card in the JSON array:

| Field | Notes |
| --- | --- |
| `id` | UUID |
| `title` | Required |
| `body` | Body / comments / context |
| `status` | `capture` \| `todo` \| `in_progress` \| `review` \| `done` |
| `revision_count` | Default `0`; shown as **rev N** when &gt; 0 |
| `agent_log` | `{ at, kind, message }` — `dispatch`, `success`, `error`, `accept`, `revise`, `edit` |
| `created_at` | RFC3339 UTC |
| `updated_at` | RFC3339 UTC |

No new persist keys for M3. Live spinner is in-process UI state; errors/summaries are the existing `agent_log` entries, now shown in the TUI.

## Held for later

- Parallel agents
- ACP / streaming transcript UI
- DB/sync, Chrome, auth beyond a local grok API key

## Develop

```bash
cargo test
cargo build
```
