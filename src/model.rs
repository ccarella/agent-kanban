use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const BOARD_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Column {
    Backlog,
    Running,
    Done,
}

impl Column {
    pub const ALL: [Column; 3] = [Column::Backlog, Column::Running, Column::Done];

    pub fn title(self) -> &'static str {
        match self {
            Column::Backlog => "Backlog",
            Column::Running => "Running",
            Column::Done => "Done",
        }
    }

    pub fn from_number(n: u8) -> Option<Self> {
        match n {
            1 => Some(Column::Backlog),
            2 => Some(Column::Running),
            3 => Some(Column::Done),
            _ => None,
        }
    }

    pub fn index(self) -> usize {
        match self {
            Column::Backlog => 0,
            Column::Running => 1,
            Column::Done => 2,
        }
    }

    pub fn from_index(i: usize) -> Self {
        Self::ALL[i % 3]
    }

    pub fn saturating_prev(self) -> Self {
        match self {
            Column::Backlog => Column::Backlog,
            Column::Running => Column::Backlog,
            Column::Done => Column::Running,
        }
    }

    pub fn saturating_next(self) -> Self {
        match self {
            Column::Backlog => Column::Running,
            Column::Running => Column::Done,
            Column::Done => Column::Done,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardStatus {
    Idle,
    Running,
    Success,
    Failed,
}

impl CardStatus {
    pub fn label(self) -> &'static str {
        match self {
            CardStatus::Idle => "idle",
            CardStatus::Running => "running",
            CardStatus::Success => "ok",
            CardStatus::Failed => "fail",
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            CardStatus::Idle => "·",
            CardStatus::Running => "●",
            CardStatus::Success => "✓",
            CardStatus::Failed => "✗",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Card {
    pub id: String,
    pub title: String,
    pub body: String,
    pub column: Column,
    pub status: CardStatus,
    #[serde(default)]
    pub last_summary: Option<String>,
    #[serde(default)]
    pub run_id: Option<String>,
}

impl Card {
    pub fn new(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            title: title.into(),
            body: body.into(),
            column: Column::Backlog,
            status: CardStatus::Idle,
            last_summary: None,
            run_id: None,
        }
    }

    pub fn prompt(&self) -> String {
        let title = self.title.trim();
        let body = self.body.trim();
        match (title.is_empty(), body.is_empty()) {
            (true, true) => String::new(),
            (false, true) => title.to_string(),
            (true, false) => body.to_string(),
            (false, false) => format!("{title}\n\n{body}"),
        }
    }

    pub fn summary_preview(&self, max_chars: usize) -> String {
        let Some(summary) = self.last_summary.as_deref() else {
            return String::new();
        };
        let line = summary
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or("");
        truncate_chars(line, max_chars)
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
    pub fn cards_in(&self, column: Column) -> Vec<&Card> {
        self.cards.iter().filter(|c| c.column == column).collect()
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

    pub fn move_card(&mut self, id: &str, column: Column) -> bool {
        if let Some(card) = self.get_mut(id) {
            card.column = column;
            true
        } else {
            false
        }
    }

    /// Cards left in a live Running state (e.g. after a crash or quit) land in
    /// Done with a fail status so they never stay stuck in Running.
    pub fn recover_interrupted_runs(&mut self) -> usize {
        let mut recovered = 0;
        for card in &mut self.cards {
            if card.status == CardStatus::Running {
                card.column = Column::Done;
                card.status = CardStatus::Failed;
                card.last_summary = Some(
                    card.last_summary
                        .clone()
                        .filter(|s| !s.trim().is_empty())
                        .unwrap_or_else(|| "Run interrupted (app quit or crashed).".to_string()),
                );
                recovered += 1;
            }
        }
        recovered
    }
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

pub fn summarize_output(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return "(no output)".to_string();
    }
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    truncate_chars(trimmed, max_chars)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_joins_title_and_body() {
        let card = Card::new("Fix login", "Check the session cookie.");
        assert_eq!(card.prompt(), "Fix login\n\nCheck the session cookie.");
    }

    #[test]
    fn recover_moves_running_cards_to_done_fail() {
        let mut board = Board::default();
        let mut card = Card::new("Work", "do it");
        card.column = Column::Running;
        card.status = CardStatus::Running;
        board.add_card(card);
        assert_eq!(board.recover_interrupted_runs(), 1);
        let card = &board.cards[0];
        assert_eq!(card.column, Column::Done);
        assert_eq!(card.status, CardStatus::Failed);
        assert!(card
            .last_summary
            .as_deref()
            .unwrap()
            .contains("interrupted"));
    }

    #[test]
    fn recover_leaves_idle_running_column_alone() {
        let mut board = Board::default();
        let mut card = Card::new("Parked", "");
        card.column = Column::Running;
        card.status = CardStatus::Idle;
        board.add_card(card);
        assert_eq!(board.recover_interrupted_runs(), 0);
        assert_eq!(board.cards[0].column, Column::Running);
        assert_eq!(board.cards[0].status, CardStatus::Idle);
    }
}
