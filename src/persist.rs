use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::model::{Board, Card};

/// Default: `./board.json` in the process cwd. Optional override: `AGENT_KANBAN_BOARD`.
pub fn default_board_path() -> PathBuf {
    if let Ok(path) = std::env::var("AGENT_KANBAN_BOARD") {
        if !path.trim().is_empty() {
            return PathBuf::from(path);
        }
    }
    PathBuf::from("board.json")
}

pub fn load_board(path: &Path) -> Result<Board> {
    if !path.exists() {
        return Ok(Board::default());
    }
    let data = fs::read_to_string(path)
        .with_context(|| format!("failed to read board file {}", path.display()))?;
    let cards: Vec<Card> = serde_json::from_str(&data).with_context(|| {
        format!(
            "failed to parse board file {} (expected a JSON array of cards)",
            path.display()
        )
    })?;
    Ok(Board { cards })
}

pub fn save_board(path: &Path, board: &Board) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
    }
    let json = serde_json::to_string_pretty(&board.cards).context("failed to serialize board")?;
    let tmp = match path.file_name() {
        Some(name) => {
            let mut tmp_name = name.to_os_string();
            tmp_name.push(".tmp");
            path.with_file_name(tmp_name)
        }
        None => path.with_extension("json.tmp"),
    };
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
    fn default_path_is_cwd_board_json() {
        let prev = std::env::var("AGENT_KANBAN_BOARD").ok();
        std::env::remove_var("AGENT_KANBAN_BOARD");
        assert_eq!(default_board_path(), PathBuf::from("board.json"));
        match prev {
            Some(v) => std::env::set_var("AGENT_KANBAN_BOARD", v),
            None => std::env::remove_var("AGENT_KANBAN_BOARD"),
        }
    }

    #[test]
    fn roundtrip_is_a_json_array_with_todo_status() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("board.json");
        let mut board = Board::default();
        let mut card = Card::new("Hello");
        card.body = "context goes here".into();
        card.status = Status::Todo;
        card.revision_count = 1;
        card.agent_log.push(AgentLogEntry {
            at: card.created_at.clone(),
            kind: "note".into(),
            message: "stored".into(),
        });
        board.add_card(card);
        save_board(&path, &board).unwrap();

        let raw = fs::read_to_string(&path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert!(
            value.is_array(),
            "persist file must be a JSON array, got {raw}"
        );
        assert_eq!(value[0]["status"], "todo");
        assert_eq!(value[0]["revision_count"], 1);
        assert!(value[0]["agent_log"].is_array());

        let loaded = load_board(&path).unwrap();
        assert_eq!(loaded.cards.len(), 1);
        let c = &loaded.cards[0];
        assert_eq!(c.title, "Hello");
        assert_eq!(c.body, "context goes here");
        assert_eq!(c.status, Status::Todo);
        assert_eq!(c.revision_count, 1);
        assert_eq!(c.agent_log[0].message, "stored");
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
