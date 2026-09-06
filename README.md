# Agent Kanban

Keyboard-only terminal Kanban (Rust + Ratatui + Crossterm).

**M1 (v0.2) is the board shell only.** No agent, no `grok`, no Review accept/revise, no dispatcher.

## Requirements

- **Rust 1.88+** (edition 2021; `cargo` on `PATH`)
  - MSRV is set in `Cargo.toml` as `rust-version = "1.88"`.

## Run

```bash
cargo run
```

`cargo run` opens **five columns on one screen**:

**Capture · To Do · In Progress · Review · Done**

## Persistence

Single file **`./board.json`** in the process current working directory. A JSON **array of cards**. Created on first save if missing.

`q` / clean exit auto-saves. Create, edit, and column moves also save immediately. Relaunch from the same cwd restores the board.

Do not use `~/.agent-kanban/` — that path is not the default.

## Keyboard (M1)

| Key | Action |
| --- | --- |
| `j` / `↓`, `k` / `↑` | Select card in the focused column |
| `h` / `←` | Move focused card one column left (updates `status`) |
| `l` / `→` | Move focused card one column right (updates `status`) |
| `n` | Inline title → new card in **Capture** |
| `Enter` | Full-screen body editor |
| `q` | Quit and auto-save |
| `?` | Help overlay (optional) |

Cards show **title**. When `revision_count > 0`, a **rev N** badge is shown (count is persisted; Review revise is M2).

## Card model

Each card in the JSON array:

| Field | Notes |
| --- | --- |
| `id` | UUID |
| `title` | Required |
| `body` | Body / context |
| `status` | `capture` \| `todo` \| `in_progress` \| `review` \| `done` |
| `revision_count` | Default `0` |
| `agent_log` | Empty array in M1 |
| `created_at` | RFC3339 UTC |
| `updated_at` | RFC3339 UTC |

## Held for M2

- `r` Review accept / revise
- Auto dispatcher and any `grok` call
- In Progress live spinner from an agent

## Develop

```bash
cargo test
cargo build
```
