# Agent Kanban

Keyboard-only terminal Kanban (Rust + Ratatui + Crossterm). **M2** adds an in-process background dispatcher that runs `grok` headless on one **To Do** card at a time, plus Review accept/revise.

Columns: **Capture · To Do · In Progress · Review · Done**

## Requirements

- **Rust 1.88+** (edition 2021; `cargo` on `PATH`)
  - MSRV is set in `Cargo.toml` as `rust-version = "1.88"`.
- For **live** dispatch: `grok` on `PATH` (or `AGENT_KANBAN_GROK`) and a valid local API key / grok auth.
- Cloud / CI VMs often have no grok auth — use the **stub** flag (below).

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

Parallel agents and a live In Progress spinner are **out of M2** (M3).

## Stub dispatch (no grok auth)

```bash
AGENT_KANBAN_STUB_DISPATCH=1 cargo run
```

The stub still moves **To Do → In Progress → Review** (or back to To Do on fail), writes `agent_log`, and bumps `revision_count` on failure — same transitions as live grok.

| Env | Effect |
| --- | --- |
| `AGENT_KANBAN_STUB_DISPATCH=1` | Fake agent (success unless fail is requested) |
| `AGENT_KANBAN_STUB_FAIL=1` | Stub always fails |
| Card title/body contains `[stub:fail]` | That card’s stub run fails |

### Smoke: stub (no grok)

```bash
# empty board in a temp dir
cd "$(mktemp -d)"
AGENT_KANBAN_STUB_DISPATCH=1 cargo run --manifest-path /path/to/agent-kanban/Cargo.toml
```

1. `n` create a card → Capture.
2. `Enter`, type a body, `Ctrl+T` → To Do.
3. Wait ~2s: card moves In Progress then **Review** (status line: stub).
4. `r` then `a` → Done. Or `r` then `v`, edit, `Ctrl+S` → To Do with **rev 1**; dispatcher will pick it again.

Force a fail loop:

```bash
AGENT_KANBAN_STUB_DISPATCH=1 AGENT_KANBAN_STUB_FAIL=1 cargo run
```

Card returns to To Do with **rev N** and an `error` `agent_log` entry. It will not auto-retry until you edit or move it.

### Smoke: real grok

```bash
# grok on PATH, authenticated
cargo run
```

Same Capture → To Do → (dispatcher) In Progress → Review flow. Inspect `./board.json` `agent_log` after a run.

Point at another binary or working tree:

```bash
AGENT_KANBAN_GROK=/usr/local/bin/grok AGENT_KANBAN_CWD=/path/to/repo cargo run
```

## Environment

| Variable | Default | Meaning |
| --- | --- | --- |
| `AGENT_KANBAN_BOARD` | `./board.json` | Board file |
| `AGENT_KANBAN_STUB_DISPATCH` | off | `1` / `true` / `yes` / `on` → stub dispatcher |
| `AGENT_KANBAN_STUB_FAIL` | off | Stub returns failure |
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

## Held for later

- Parallel agents
- Live In Progress spinner / streaming status (M3)
- DB/sync, ACP, auth beyond a local grok API key

## Develop

```bash
cargo test
cargo build
```
