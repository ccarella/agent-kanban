# Agent Kanban

Terminal Kanban for dispatching local coding work to **Grok Build** via headless `grok -p`.

v0.1 is a keyboard-only Ratatui TUI. No web UI. No Cursor Cloud Agent runtime.

## Requirements

- **Rust 1.88+** (stable; `cargo` on `PATH`)
- **`grok` on `PATH`** for dispatch — [Grok Build CLI](https://x.ai/cli)

  ```bash
  curl -fsSL https://x.ai/cli/install.sh | bash
  ```

  Authenticate with `grok login`, or set `XAI_API_KEY` for headless use.

The board UI and persistence work **without** `grok`. Pressing `r` when `grok` is missing (or the process fails) moves the card to **Done** with a **fail** status and a short error. That is expected in CI or a VM that only has Rust.

## Run

```bash
cargo run
```

Release build:

```bash
cargo build --release
./target/release/agent-kanban
```

`cargo run` opens a board with three fixed columns: **Backlog / Running / Done**.

## Persistence

Board file (created on first save):

```
~/.agent-kanban/board.json
```

Override the path with `AGENT_KANBAN_BOARD` (absolute or relative):

```bash
AGENT_KANBAN_BOARD=./board.json cargo run
```

Quit (`q`) always saves. Mutations (create, edit, move, delete, dispatch start/finish) also save immediately. Relaunch restores the last saved board.

If the app quits or crashes while a card is actually running (`status: running`), the next load moves that card to **Done (fail)** with an interrupted-run message so it cannot stay stuck in Running.

## Dispatch

`r` on the selected card starts a real headless Grok invocation:

```bash
grok --no-auto-update --no-alt-screen --output-format plain -s <run-id> -p "<title>\n\n<body>"
```

- Prompt is the card **title + body**.
- The card moves to **Running** with a live elapsed-time status.
- **One concurrent run.** A second `r` is refused with a status-line message until the first finishes.
- On success → **Done** + **ok** + a short stdout summary.
- On failure (nonzero exit, missing binary, spawn error, quit mid-run) → **Done** + **fail** + error text.

Override the binary for tests or wrappers:

```bash
AGENT_KANBAN_GROK=/path/to/grok cargo run
```

`--no-alt-screen` keeps Grok from taking over this TUI. `--always-approve` is **not** passed.

## Keyboard

| Key | Action |
| --- | --- |
| `h` / `←`, `l` / `→` | Focus column |
| `j` / `↓`, `k` / `↑` | Select card in the focused column |
| `Enter` | Edit / view card (title + body/prompt) |
| `n` | New card (Backlog) |
| `d` | Delete selected (confirm `y` / `n`) |
| `1` / `2` / `3` | Move to Backlog / Running / Done |
| `r` | Run / dispatch selected (`grok -p`) |
| `q` | Quit (save) |
| `?` | Help overlay (this map) |

Editor: **Tab** switches title/body, **Ctrl+S** saves, **Esc** cancels. In the title field, **Enter** moves to the body.

## Card model

Each card stores: `id`, `title`, `body` (prompt), `column`, `status` (`idle` / `running` / `success` / `failed`), `last_summary` (short result or error), optional `run_id` (Grok session id).

## Out of scope (v0.1)

Multi-user, cloud sync, web UI, Cloud Agent runtime, custom columns, attachments, MCP/config UI, agent picker, spend tracking, parallel multi-run.

## Develop

```bash
cargo test
cargo build
```
