use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::model::Board;

/// Override with `AGENT_KANBAN_BOARD`. Default: `~/.agent-kanban/board.json`.
pub fn default_board_path() -> PathBuf {
    if let Ok(path) = std::env::var("AGENT_KANBAN_BOARD") {
        if !path.trim().is_empty() {
            return PathBuf::from(path);
        }
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".agent-kanban")
        .join("board.json")
}

pub fn load_board(path: &Path) -> Result<Board> {
    if !path.exists() {
        return Ok(Board::default());
    }
    let data = fs::read_to_string(path)
        .with_context(|| format!("failed to read board file {}", path.display()))?;
    let mut board: Board = serde_json::from_str(&data)
        .with_context(|| format!("failed to parse board file {}", path.display()))?;
    board.recover_interrupted_runs();
    Ok(board)
}

pub fn save_board(path: &Path, board: &Board) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(board).context("failed to serialize board")?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json).with_context(|| format!("failed to write {}", tmp.display()))?;
    fs::rename(&tmp, path)
        .with_context(|| format!("failed to persist board to {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Card, CardStatus, Column};

    #[test]
    fn roundtrip_preserves_cards() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("board.json");
        let mut board = Board::default();
        board.add_card(Card::new("Hello", "World"));
        save_board(&path, &board).unwrap();
        let loaded = load_board(&path).unwrap();
        assert_eq!(loaded.cards.len(), 1);
        assert_eq!(loaded.cards[0].title, "Hello");
        assert_eq!(loaded.cards[0].body, "World");
    }

    #[test]
    fn missing_file_is_empty_board() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.json");
        let board = load_board(&path).unwrap();
        assert!(board.cards.is_empty());
    }

    #[test]
    fn load_recovers_running_status() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("board.json");
        let mut board = Board::default();
        let mut card = Card::new("Live", "prompt");
        card.column = Column::Running;
        card.status = CardStatus::Running;
        board.add_card(card);
        save_board(&path, &board).unwrap();
        let loaded = load_board(&path).unwrap();
        assert_eq!(loaded.cards[0].column, Column::Done);
        assert_eq!(loaded.cards[0].status, CardStatus::Failed);
    }
}
