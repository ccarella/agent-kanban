use std::path::{Path, PathBuf};

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::model::{Board, Card, Status};
use crate::persist;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorField {
    Title,
    Body,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorState {
    pub card_id: String,
    pub title: String,
    pub body: String,
    pub field: EditorField,
    pub cursor: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineTitle {
    pub buffer: String,
    pub cursor: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    Board,
    Help,
    InlineTitle(InlineTitle),
    Editor(EditorState),
}

pub struct App {
    pub board: Board,
    pub board_path: PathBuf,
    pub focused: Status,
    pub selected_id: Option<String>,
    pub mode: Mode,
    pub status_message: String,
    pub should_quit: bool,
}

impl App {
    pub fn load() -> Result<Self> {
        Self::load_from(persist::default_board_path())
    }

    pub fn load_from(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let board = persist::load_board(&path)?;
        let selected_id = match board.cards_in(Status::Capture).first() {
            Some(c) => Some(c.id.clone()),
            None => board.cards_in_board_order().first().map(|c| c.id.clone()),
        };
        let focused = selected_id
            .as_deref()
            .and_then(|id| board.get(id))
            .map(|c| c.status)
            .unwrap_or(Status::Capture);
        let mut app = Self {
            selected_id,
            board,
            board_path: path,
            focused,
            mode: Mode::Board,
            status_message: String::new(),
            should_quit: false,
        };
        app.ensure_selection();
        Ok(app)
    }

    pub fn save(&mut self) -> Result<()> {
        persist::save_board(&self.board_path, &self.board)
    }

    pub fn persist(&mut self) {
        if let Err(err) = self.save() {
            self.status_message = format!("save failed: {err}");
        }
    }

    pub fn selected_card(&self) -> Option<&Card> {
        self.selected_id
            .as_deref()
            .and_then(|id| self.board.get(id))
    }

    fn ensure_selection(&mut self) {
        if let Some(id) = &self.selected_id {
            if let Some(card) = self.board.get(id) {
                self.focused = card.status;
                return;
            }
        }
        let in_col: Vec<String> = self
            .board
            .cards_in(self.focused)
            .into_iter()
            .map(|c| c.id.clone())
            .collect();
        self.selected_id = in_col.first().cloned().or_else(|| {
            self.board
                .cards_in_board_order()
                .first()
                .map(|c| c.id.clone())
        });
        if let Some(id) = &self.selected_id {
            if let Some(card) = self.board.get(id) {
                self.focused = card.status;
            }
        }
    }

    fn select_in_column(&mut self, delta: isize) {
        let ids: Vec<String> = self
            .board
            .cards_in(self.focused)
            .into_iter()
            .map(|c| c.id.clone())
            .collect();
        if ids.is_empty() {
            self.selected_id = None;
            return;
        }
        let current = self
            .selected_id
            .as_ref()
            .and_then(|id| ids.iter().position(|x| x == id))
            .unwrap_or(0);
        let next = if delta < 0 {
            current.saturating_sub(delta.unsigned_abs())
        } else {
            (current + delta as usize).min(ids.len() - 1)
        };
        self.selected_id = Some(ids[next].clone());
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        if key.kind != KeyEventKind::Press {
            return;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.quit();
            return;
        }

        match &self.mode {
            Mode::Help => self.handle_help_key(key),
            Mode::InlineTitle(_) => self.handle_inline_title_key(key),
            Mode::Editor(_) => self.handle_editor_key(key),
            Mode::Board => self.handle_board_key(key),
        }
    }

    fn handle_help_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') => self.quit(),
            KeyCode::Esc | KeyCode::Char('?') => self.mode = Mode::Board,
            _ => self.mode = Mode::Board,
        }
    }

    fn handle_board_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') => self.quit(),
            KeyCode::Char('?') => self.mode = Mode::Help,
            KeyCode::Char('j') | KeyCode::Down => self.select_in_column(1),
            KeyCode::Char('k') | KeyCode::Up => self.select_in_column(-1),
            KeyCode::Char('h') | KeyCode::Left => self.shift_selected(-1),
            KeyCode::Char('l') | KeyCode::Right => self.shift_selected(1),
            KeyCode::Char('n') => self.start_inline_title(),
            KeyCode::Enter => self.open_editor(),
            _ => {}
        }
    }

    fn start_inline_title(&mut self) {
        self.mode = Mode::InlineTitle(InlineTitle {
            buffer: String::new(),
            cursor: 0,
        });
        self.status_message = "New card title — Enter creates in Capture, Esc cancels.".to_string();
    }

    fn handle_inline_title_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Board;
                self.status_message = "New card cancelled.".to_string();
            }
            KeyCode::Enter => self.commit_inline_title(),
            KeyCode::Backspace => {
                if let Mode::InlineTitle(state) = &mut self.mode {
                    if state.cursor > 0 {
                        remove_char(&mut state.buffer, state.cursor - 1);
                        state.cursor -= 1;
                    }
                }
            }
            KeyCode::Delete => {
                if let Mode::InlineTitle(state) = &mut self.mode {
                    remove_char(&mut state.buffer, state.cursor);
                }
            }
            KeyCode::Left => {
                if let Mode::InlineTitle(state) = &mut self.mode {
                    state.cursor = state.cursor.saturating_sub(1);
                }
            }
            KeyCode::Right => {
                if let Mode::InlineTitle(state) = &mut self.mode {
                    state.cursor = (state.cursor + 1).min(state.buffer.chars().count());
                }
            }
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) && ch != '\n' => {
                if let Mode::InlineTitle(state) = &mut self.mode {
                    insert_char(&mut state.buffer, state.cursor, ch);
                    state.cursor += 1;
                }
            }
            _ => {}
        }
    }

    fn commit_inline_title(&mut self) {
        let Mode::InlineTitle(state) = &self.mode else {
            return;
        };
        let title = state.buffer.trim().to_string();
        if title.is_empty() {
            self.status_message = "Title is required.".to_string();
            return;
        }
        let card = Card::new(title);
        self.selected_id = Some(card.id.clone());
        self.focused = Status::Capture;
        self.board.add_card(card);
        self.mode = Mode::Board;
        self.status_message = "Card created in Capture.".to_string();
        self.persist();
    }

    fn open_editor(&mut self) {
        let Some(card) = self.selected_card().cloned() else {
            self.status_message = "No card selected. Press n to create one.".to_string();
            return;
        };
        self.mode = Mode::Editor(EditorState {
            card_id: card.id,
            title: card.title.clone(),
            body: card.body,
            field: EditorField::Body,
            cursor: 0,
        });
        if let Mode::Editor(state) = &mut self.mode {
            state.cursor = state.body.chars().count();
        }
        self.status_message =
            "Full-screen editor — Tab title/body, Ctrl+S saves, Esc cancels.".to_string();
    }

    fn handle_editor_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('s') {
            self.commit_editor();
            return;
        }
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Board;
                self.status_message = "Edit cancelled.".to_string();
            }
            KeyCode::Tab | KeyCode::BackTab => self.editor_switch_field(),
            KeyCode::Enter => {
                if self.editor_field() == Some(EditorField::Title) {
                    self.editor_set_field(EditorField::Body);
                } else {
                    self.editor_insert('\n');
                }
            }
            KeyCode::Backspace => self.editor_backspace(),
            KeyCode::Delete => self.editor_delete(),
            KeyCode::Left => self.editor_move_cursor(-1),
            KeyCode::Right => self.editor_move_cursor(1),
            KeyCode::Home => self.editor_set_cursor(0),
            KeyCode::End => {
                if let Some(len) = self.editor_field_len() {
                    self.editor_set_cursor(len);
                }
            }
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.editor_insert(ch);
            }
            _ => {}
        }
    }

    fn commit_editor(&mut self) {
        let Mode::Editor(state) = &self.mode else {
            return;
        };
        let id = state.card_id.clone();
        let title = state.title.trim().to_string();
        let body = state.body.clone();
        if title.is_empty() {
            self.status_message = "Title is required.".to_string();
            return;
        }
        if let Some(card) = self.board.get_mut(&id) {
            card.title = title;
            card.body = body;
            card.touch();
        }
        self.selected_id = Some(id);
        self.mode = Mode::Board;
        self.status_message = "Card updated.".to_string();
        self.persist();
    }

    fn shift_selected(&mut self, dir: isize) {
        let Some(id) = self.selected_id.clone() else {
            self.status_message = "No card selected.".to_string();
            return;
        };
        let Some(card) = self.board.get(&id) else {
            return;
        };
        let next = if dir < 0 {
            card.status.saturating_left()
        } else {
            card.status.saturating_right()
        };
        if next == card.status {
            self.status_message = format!("Already at {}.", card.status.title());
            return;
        }
        self.board.move_card(&id, next);
        self.focused = next;
        self.status_message = format!("Moved to {}.", next.title());
        self.persist();
    }

    fn quit(&mut self) {
        self.persist();
        self.should_quit = true;
    }

    fn editor_state_mut(&mut self) -> Option<&mut EditorState> {
        match &mut self.mode {
            Mode::Editor(state) => Some(state),
            _ => None,
        }
    }

    fn editor_field(&self) -> Option<EditorField> {
        match &self.mode {
            Mode::Editor(state) => Some(state.field),
            _ => None,
        }
    }

    fn editor_field_len(&self) -> Option<usize> {
        match &self.mode {
            Mode::Editor(state) => Some(match state.field {
                EditorField::Title => state.title.chars().count(),
                EditorField::Body => state.body.chars().count(),
            }),
            _ => None,
        }
    }

    fn editor_switch_field(&mut self) {
        if let Some(state) = self.editor_state_mut() {
            state.field = match state.field {
                EditorField::Title => EditorField::Body,
                EditorField::Body => EditorField::Title,
            };
            state.cursor = match state.field {
                EditorField::Title => state.title.chars().count(),
                EditorField::Body => state.body.chars().count(),
            };
        }
    }

    fn editor_set_field(&mut self, field: EditorField) {
        if let Some(state) = self.editor_state_mut() {
            state.field = field;
            state.cursor = match field {
                EditorField::Title => state.title.chars().count(),
                EditorField::Body => state.body.chars().count(),
            };
        }
    }

    fn editor_set_cursor(&mut self, pos: usize) {
        if let Some(state) = self.editor_state_mut() {
            let len = match state.field {
                EditorField::Title => state.title.chars().count(),
                EditorField::Body => state.body.chars().count(),
            };
            state.cursor = pos.min(len);
        }
    }

    fn editor_move_cursor(&mut self, delta: isize) {
        if let Some(state) = self.editor_state_mut() {
            let len = match state.field {
                EditorField::Title => state.title.chars().count(),
                EditorField::Body => state.body.chars().count(),
            } as isize;
            state.cursor = (state.cursor as isize + delta).clamp(0, len) as usize;
        }
    }

    fn editor_insert(&mut self, ch: char) {
        if let Some(state) = self.editor_state_mut() {
            if state.field == EditorField::Title && ch == '\n' {
                return;
            }
            let field = match state.field {
                EditorField::Title => &mut state.title,
                EditorField::Body => &mut state.body,
            };
            insert_char(field, state.cursor, ch);
            state.cursor += 1;
        }
    }

    fn editor_backspace(&mut self) {
        if let Some(state) = self.editor_state_mut() {
            if state.cursor == 0 {
                return;
            }
            let field = match state.field {
                EditorField::Title => &mut state.title,
                EditorField::Body => &mut state.body,
            };
            remove_char(field, state.cursor - 1);
            state.cursor -= 1;
        }
    }

    fn editor_delete(&mut self) {
        if let Some(state) = self.editor_state_mut() {
            let field = match state.field {
                EditorField::Title => &mut state.title,
                EditorField::Body => &mut state.body,
            };
            remove_char(field, state.cursor);
        }
    }
}

fn insert_char(s: &mut String, char_idx: usize, ch: char) {
    let byte = s
        .char_indices()
        .nth(char_idx)
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    s.insert(byte, ch);
}

fn remove_char(s: &mut String, char_idx: usize) {
    if let Some((byte, ch)) = s.char_indices().nth(char_idx) {
        s.replace_range(byte..byte + ch.len_utf8(), "");
    }
}

pub fn press(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_in_tmp() -> (App, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let app = App::load_from(dir.path().join("board.json")).unwrap();
        (app, dir)
    }

    #[test]
    fn n_creates_card_in_capture_and_persists() {
        let (mut app, dir) = app_in_tmp();
        app.handle_key(press(KeyCode::Char('n')));
        for ch in "Ship it".chars() {
            app.handle_key(press(KeyCode::Char(ch)));
        }
        app.handle_key(press(KeyCode::Enter));

        assert_eq!(app.board.cards.len(), 1);
        assert_eq!(app.board.cards[0].title, "Ship it");
        assert_eq!(app.board.cards[0].status, Status::Capture);
        assert!(app.board.cards[0].body.is_empty());

        let reloaded = persist::load_board(&dir.path().join("board.json")).unwrap();
        assert_eq!(reloaded.cards[0].title, "Ship it");
        assert_eq!(reloaded.cards[0].status, Status::Capture);
    }

    #[test]
    fn hl_and_arrows_move_card_across_columns() {
        let (mut app, _dir) = app_in_tmp();
        app.board.add_card(Card::new("A"));
        app.selected_id = Some(app.board.cards[0].id.clone());

        app.handle_key(press(KeyCode::Char('l')));
        assert_eq!(app.board.cards[0].status, Status::Todo);
        app.handle_key(press(KeyCode::Right));
        assert_eq!(app.board.cards[0].status, Status::InProgress);
        app.handle_key(press(KeyCode::Char('h')));
        assert_eq!(app.board.cards[0].status, Status::Todo);
        app.handle_key(press(KeyCode::Left));
        assert_eq!(app.board.cards[0].status, Status::Capture);
        app.handle_key(press(KeyCode::Char('h')));
        assert_eq!(app.board.cards[0].status, Status::Capture);
    }

    #[test]
    fn enter_opens_fullscreen_editor_and_saves_body() {
        let (mut app, dir) = app_in_tmp();
        app.board.add_card(Card::new("Title"));
        app.selected_id = Some(app.board.cards[0].id.clone());
        app.handle_key(press(KeyCode::Enter));
        assert!(matches!(app.mode, Mode::Editor(_)));
        for ch in "body and context".chars() {
            app.handle_key(press(KeyCode::Char(ch)));
        }
        let mut save = press(KeyCode::Char('s'));
        save.modifiers = KeyModifiers::CONTROL;
        app.handle_key(save);
        assert_eq!(app.board.cards[0].body, "body and context");
        let loaded = persist::load_board(&dir.path().join("board.json")).unwrap();
        assert_eq!(loaded.cards[0].body, "body and context");
    }

    #[test]
    fn r_does_not_run_review_or_agent() {
        let (mut app, _dir) = app_in_tmp();
        let mut card = Card::new("In review");
        card.status = Status::Review;
        app.board.add_card(card);
        app.selected_id = Some(app.board.cards[0].id.clone());
        app.focused = Status::Review;
        app.handle_key(press(KeyCode::Char('r')));
        assert_eq!(app.board.cards[0].status, Status::Review);
        assert_eq!(app.board.cards[0].revision_count, 0);
        assert!(matches!(app.mode, Mode::Board));
    }

    #[test]
    fn quit_saves() {
        let (mut app, dir) = app_in_tmp();
        app.board.add_card(Card::new("Keep"));
        app.handle_key(press(KeyCode::Char('q')));
        assert!(app.should_quit);
        let loaded = persist::load_board(&dir.path().join("board.json")).unwrap();
        assert_eq!(loaded.cards[0].title, "Keep");
    }

    #[test]
    fn jk_selects_only_within_column() {
        let (mut app, _dir) = app_in_tmp();
        app.board.add_card(Card::new("One"));
        app.board.add_card(Card::new("Two"));
        let mut other = Card::new("Other col");
        other.status = Status::Done;
        app.board.add_card(other);
        app.focused = Status::Capture;
        app.selected_id = Some(app.board.cards[0].id.clone());
        app.handle_key(press(KeyCode::Char('j')));
        assert_eq!(
            app.selected_id.as_deref(),
            Some(app.board.cards[1].id.as_str())
        );
        app.handle_key(press(KeyCode::Char('j')));
        assert_eq!(
            app.selected_id.as_deref(),
            Some(app.board.cards[1].id.as_str()),
            "j must stay in Capture, not jump to Done"
        );
        app.handle_key(press(KeyCode::Char('k')));
        assert_eq!(
            app.selected_id.as_deref(),
            Some(app.board.cards[0].id.as_str())
        );
    }
}
