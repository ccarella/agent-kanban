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
    let board: Board = serde_json::from_str(&data)
        .with_context(|| format!("failed to parse board file {}", path.display()))?;
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
    use crate::model::{AgentLogEntry, Card, Status};

    #[test]
    fn roundtrip_preserves_v1_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("board.json");
        let mut board = Board::default();
        let mut card = Card::new("Hello");
        card.body = "context goes here".into();
        card.status = Status::Review;
        card.revision_count = 1;
        card.agent_log.push(AgentLogEntry {
            at: card.created_at.clone(),
            kind: "revision".into(),
            message: "add tests".into(),
        });
        board.add_card(card);
        save_board(&path, &board).unwrap();

        let loaded = load_board(&path).unwrap();
        assert_eq!(loaded.version, 1);
        assert_eq!(loaded.cards.len(), 1);
        let c = &loaded.cards[0];
        assert_eq!(c.title, "Hello");
        assert_eq!(c.body, "context goes here");
        assert_eq!(c.status, Status::Review);
        assert_eq!(c.revision_count, 1);
        assert_eq!(c.agent_log[0].message, "add tests");
        assert!(!c.created_at.is_empty());
        assert!(!c.updated_at.is_empty());
    }

    #[test]
    fn missing_file_is_empty_board() {
        let dir = tempfile::tempdir().unwrap();
        let board = load_board(&dir.path().join("missing.json")).unwrap();
        assert!(board.cards.is_empty());
    }
}
