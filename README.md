# Agent Kanban

Keyboard-only terminal Kanban (Rust + Ratatui + Crossterm).

**M1 (v0.2) is the board shell only.** No agent dispatch, no `grok` subprocess, no background runner.

## Requirements

- **Rust 1.88+** (edition 2021; `cargo` on `PATH`)
  - MSRV is set in `Cargo.toml` as `rust-version = "1.88"`.

No other runtime dependencies for M1.

## Run

```bash
cargo run
```

Release:

```bash
cargo build --release
./target/release/agent-kanban
```

`cargo run` opens **five columns on one screen**:

**Capture · To Do · In Progress · Review · Done**

## Persistence (storage V1)

One JSON file (an object with a `cards` array). No database.

Default path:

```
~/.agent-kanban/board.json
```

Override:

```bash
AGENT_KANBAN_BOARD=./board.json cargo run
```

`q` always auto-saves. Create / edit / move / review also save immediately. Quit and relaunch restore the last file.

## Keyboard (M1)

| Key | Action |
| --- | --- |
| `j` / `↓`, `k` / `↑` | Move focus among cards |
| `n` | Inline title → new card in **Capture** |
| `Enter` | Full-screen editor for body / context |
| `h` / `←` | Move focused card one column left |
| `l` / `→` | Move focused card one column right |
| `r` | **Review only:** `a` accept → Done; `v` revise (edit comments) → To Do and bump `revision_count` |
| `q` | Quit (auto-save) |
| `?` | Help overlay |

Cards show **title**. When `revision_count > 0`, a **rev N** badge is shown.

## Card model

Each card is persisted as:

| Field | Notes |
| --- | --- |
| `id` | UUID |
| `title` | Required |
| `body` | Body / context (edited full-screen) |
| `status` | `capture` · `to_do` · `in_progress` · `review` · `done` |
| `revision_count` | Incremented on Review → revise |
| `agent_log` | Array of `{ at, kind, message }` (stored in M1; used for revise comments) |
| `created_at` | RFC3339 UTC |
| `updated_at` | RFC3339 UTC |

## Out of M1

No `grok` shell-out, no auto-pick from To Do, no dispatcher, no parallel runs, no web UI. Those belong to later milestones.

## Develop

```bash
cargo test
cargo build
```
