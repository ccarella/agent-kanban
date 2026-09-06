use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const BOARD_VERSION: u32 = 1;

/// Column / workflow status. Serialized as `status` on each card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Capture,
    ToDo,
    InProgress,
    Review,
    Done,
}

impl Status {
    pub const ALL: [Status; 5] = [
        Status::Capture,
        Status::ToDo,
        Status::InProgress,
        Status::Review,
        Status::Done,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Status::Capture => "Capture",
            Status::ToDo => "To Do",
            Status::InProgress => "In Progress",
            Status::Review => "Review",
            Status::Done => "Done",
        }
    }

    pub fn index(self) -> usize {
        match self {
            Status::Capture => 0,
            Status::ToDo => 1,
            Status::InProgress => 2,
            Status::Review => 3,
            Status::Done => 4,
        }
    }

    pub fn saturating_left(self) -> Self {
        match self {
            Status::Capture => Status::Capture,
            Status::ToDo => Status::Capture,
            Status::InProgress => Status::ToDo,
            Status::Review => Status::InProgress,
            Status::Done => Status::Review,
        }
    }

    pub fn saturating_right(self) -> Self {
        match self {
            Status::Capture => Status::ToDo,
            Status::ToDo => Status::InProgress,
            Status::InProgress => Status::Review,
            Status::Review => Status::Done,
            Status::Done => Status::Done,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentLogEntry {
    pub at: String,
    pub kind: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Card {
    pub id: String,
    pub title: String,
    pub body: String,
    pub status: Status,
    #[serde(default)]
    pub revision_count: u32,
    #[serde(default)]
    pub agent_log: Vec<AgentLogEntry>,
    pub created_at: String,
    pub updated_at: String,
}

impl Card {
    pub fn new(title: impl Into<String>) -> Self {
        let now = now_iso8601();
        Self {
            id: Uuid::new_v4().to_string(),
            title: title.into(),
            body: String::new(),
            status: Status::Capture,
            revision_count: 0,
            agent_log: Vec::new(),
            created_at: now.clone(),
            updated_at: now,
        }
    }

    pub fn touch(&mut self) {
        self.updated_at = now_iso8601();
    }

    pub fn rev_badge(&self) -> Option<String> {
        if self.revision_count > 0 {
            Some(format!("rev {}", self.revision_count))
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Board {
    pub version: u32,
    pub cards: Vec<Card>,
}

impl Default for Board {
    fn default() -> Self {
        Self {
            version: BOARD_VERSION,
            cards: Vec::new(),
        }
    }
}

impl Board {
    pub fn cards_in(&self, status: Status) -> Vec<&Card> {
        self.cards.iter().filter(|c| c.status == status).collect()
    }

    pub fn get(&self, id: &str) -> Option<&Card> {
        self.cards.iter().find(|c| c.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Card> {
        self.cards.iter_mut().find(|c| c.id == id)
    }

    pub fn add_card(&mut self, card: Card) {
        self.cards.push(card);
    }

    pub fn remove_card(&mut self, id: &str) -> Option<Card> {
        let idx = self.cards.iter().position(|c| c.id == id)?;
        Some(self.cards.remove(idx))
    }

    pub fn move_card(&mut self, id: &str, status: Status) -> bool {
        if let Some(card) = self.get_mut(id) {
            if card.status != status {
                card.status = status;
                card.touch();
            }
            true
        } else {
            false
        }
    }

    /// Cards in column-major order (Capture top-to-bottom, then To Do, …).
    pub fn cards_in_board_order(&self) -> Vec<&Card> {
        let mut out = Vec::with_capacity(self.cards.len());
        for status in Status::ALL {
            out.extend(self.cards_in(status));
        }
        out
    }
}

pub fn now_iso8601() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

pub fn truncate_chars(s: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i >= max_chars {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_card_lands_in_capture() {
        let card = Card::new("Inbox item");
        assert_eq!(card.status, Status::Capture);
        assert_eq!(card.revision_count, 0);
        assert!(card.agent_log.is_empty());
        assert!(card.rev_badge().is_none());
        assert!(!card.created_at.is_empty());
    }

    #[test]
    fn rev_badge_only_when_revised() {
        let mut card = Card::new("Work");
        assert!(card.rev_badge().is_none());
        card.revision_count = 2;
        assert_eq!(card.rev_badge().as_deref(), Some("rev 2"));
    }

    #[test]
    fn move_updates_status_and_timestamp() {
        let mut board = Board::default();
        let card = Card::new("A");
        let id = card.id.clone();
        let created = card.updated_at.clone();
        board.add_card(card);
        assert!(board.move_card(&id, Status::ToDo));
        assert_eq!(board.get(&id).unwrap().status, Status::ToDo);
        assert!(board.get(&id).unwrap().updated_at >= created);
    }

    #[test]
    fn saturating_edges() {
        assert_eq!(Status::Capture.saturating_left(), Status::Capture);
        assert_eq!(Status::Done.saturating_right(), Status::Done);
        assert_eq!(Status::Capture.saturating_right(), Status::ToDo);
        assert_eq!(Status::Review.saturating_left(), Status::InProgress);
    }
}
