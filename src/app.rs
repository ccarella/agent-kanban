use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::Instant;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::dispatch::{self, DispatchOutcome};
use crate::model::{Board, Card, CardStatus, Column};
use crate::persist;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorField {
    Title,
    Body,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorState {
    pub card_id: Option<String>,
    pub title: String,
    pub body: String,
    pub field: EditorField,
    pub cursor: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    Board,
    Help,
    Editor(EditorState),
    ConfirmDelete { id: String, title: String },
}

pub struct ActiveRun {
    pub card_id: String,
    pub run_id: String,
    pub started: Instant,
    rx: Receiver<DispatchOutcome>,
}

pub struct App {
    pub board: Board,
    pub board_path: PathBuf,
    pub focused: Column,
    pub selected_id: Option<String>,
    pub mode: Mode,
    pub status_message: String,
    pub should_quit: bool,
    pub dirty: bool,
    active_run: Option<ActiveRun>,
}

impl App {
    pub fn load() -> Result<Self> {
        Self::load_from(persist::default_board_path())
    }

    pub fn load_from(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let board = persist::load_board(&path)?;
        let mut app = Self {
            selected_id: board.cards.first().map(|c| c.id.clone()),
            board,
            board_path: path,
            focused: Column::Backlog,
            mode: Mode::Board,
            status_message: String::new(),
            should_quit: false,
            dirty: false,
            active_run: None,
        };
        // Recovery already applied in load_board; surface it once.
        if app.board.cards.iter().any(|c| {
            c.status == CardStatus::Failed
                && c.last_summary
                    .as_deref()
                    .is_some_and(|s| s.contains("interrupted"))
        }) {
            app.status_message = "Recovered interrupted run(s) → Done (fail).".to_string();
        }
        app.ensure_selection();
        Ok(app)
    }

    pub fn save(&mut self) -> Result<()> {
        persist::save_board(&self.board_path, &self.board)?;
        self.dirty = false;
        Ok(())
    }

    pub fn persist(&mut self) {
        if let Err(err) = self.save() {
            self.status_message = format!("save failed: {err}");
        }
    }

    pub fn is_running(&self) -> bool {
        self.active_run.is_some()
    }

    pub fn running_card_id(&self) -> Option<&str> {
        self.active_run.as_ref().map(|r| r.card_id.as_str())
    }

    pub fn run_elapsed_secs(&self) -> Option<u64> {
        self.active_run
            .as_ref()
            .map(|r| r.started.elapsed().as_secs())
    }

    pub fn selected_card(&self) -> Option<&Card> {
        self.selected_id
            .as_deref()
            .and_then(|id| self.board.get(id))
    }

    fn cards_in(&self, column: Column) -> Vec<&Card> {
        self.board.cards_in(column)
    }

    fn ensure_selection(&mut self) {
        if let Some(id) = &self.selected_id {
            if self.board.get(id).is_some() {
                return;
            }
        }
        let focused_cards = self.cards_in(self.focused);
        self.selected_id = focused_cards
            .first()
            .map(|c| c.id.clone())
            .or_else(|| self.board.cards.first().map(|c| c.id.clone()));
    }

    fn select_in_focused(&mut self, delta: isize) {
        let ids: Vec<String> = self
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

    fn focus_column(&mut self, column: Column) {
        self.focused = column;
        let ids: Vec<String> = self
            .cards_in(column)
            .into_iter()
            .map(|c| c.id.clone())
            .collect();
        if let Some(id) = &self.selected_id {
            if ids.iter().any(|x| x == id) {
                return;
            }
        }
        self.selected_id = ids.first().cloned();
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
            Mode::ConfirmDelete { .. } => self.handle_delete_key(key),
            Mode::Editor(_) => self.handle_editor_key(key),
            Mode::Board => self.handle_board_key(key),
        }
    }

    fn handle_help_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') => {
                if key.code == KeyCode::Char('q') {
                    self.quit();
                } else {
                    self.mode = Mode::Board;
                }
            }
            _ => self.mode = Mode::Board,
        }
    }

    fn handle_delete_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                if let Mode::ConfirmDelete { id, .. } = &self.mode {
                    let id = id.clone();
                    self.delete_card(&id);
                }
                self.mode = Mode::Board;
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                self.mode = Mode::Board;
                self.status_message = "Delete cancelled.".to_string();
            }
            _ => {}
        }
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
            KeyCode::Tab => self.editor_switch_field(),
            KeyCode::BackTab => self.editor_switch_field(),
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

    fn handle_board_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') => self.quit(),
            KeyCode::Char('?') => self.mode = Mode::Help,
            KeyCode::Char('h') | KeyCode::Left => self.focus_column(self.focused.saturating_prev()),
            KeyCode::Char('l') | KeyCode::Right => {
                self.focus_column(self.focused.saturating_next())
            }
            KeyCode::Char('j') | KeyCode::Down => self.select_in_focused(1),
            KeyCode::Char('k') | KeyCode::Up => self.select_in_focused(-1),
            KeyCode::Char('n') => self.open_new_card(),
            KeyCode::Enter => self.open_edit_selected(),
            KeyCode::Char('d') => self.prompt_delete(),
            KeyCode::Char('1') => self.move_selected(Column::Backlog),
            KeyCode::Char('2') => self.move_selected(Column::Running),
            KeyCode::Char('3') => self.move_selected(Column::Done),
            KeyCode::Char('r') => self.dispatch_selected(),
            _ => {}
        }
    }

    fn open_new_card(&mut self) {
        self.mode = Mode::Editor(EditorState {
            card_id: None,
            title: String::new(),
            body: String::new(),
            field: EditorField::Title,
            cursor: 0,
        });
        self.status_message =
            "New card — Tab switches fields, Ctrl+S saves, Esc cancels.".to_string();
    }

    fn open_edit_selected(&mut self) {
        let Some(card) = self.selected_card().cloned() else {
            self.status_message = "No card selected. Press n to create one.".to_string();
            return;
        };
        self.mode = Mode::Editor(EditorState {
            card_id: Some(card.id),
            title: card.title,
            body: card.body,
            field: EditorField::Title,
            cursor: 0,
        });
        if let Some(ed) = self.editor_state_mut() {
            ed.cursor = ed.title.chars().count();
        }
        self.status_message =
            "Edit card — Tab switches fields, Ctrl+S saves, Esc cancels.".to_string();
    }

    fn prompt_delete(&mut self) {
        let Some(card) = self.selected_card() else {
            self.status_message = "No card selected.".to_string();
            return;
        };
        if self.running_card_id() == Some(card.id.as_str()) {
            self.status_message =
                "Cannot delete a card while its grok run is in progress.".to_string();
            return;
        }
        self.mode = Mode::ConfirmDelete {
            id: card.id.clone(),
            title: card.title.clone(),
        };
    }

    fn delete_card(&mut self, id: &str) {
        if self.running_card_id() == Some(id) {
            self.status_message =
                "Cannot delete a card while its grok run is in progress.".to_string();
            return;
        }
        if self.board.remove_card(id).is_some() {
            self.status_message = "Card deleted.".to_string();
            self.ensure_selection();
            self.persist();
        }
    }

    fn move_selected(&mut self, column: Column) {
        let Some(id) = self.selected_id.clone() else {
            self.status_message = "No card selected.".to_string();
            return;
        };
        if self.board.move_card(&id, column) {
            self.focused = column;
            self.status_message = format!("Moved to {}.", column.title());
            self.persist();
        }
    }

    pub fn dispatch_selected(&mut self) {
        if self.active_run.is_some() {
            self.status_message =
                "A run is already in progress. Only one grok -p dispatch at a time.".to_string();
            return;
        }
        let Some(card) = self.selected_card().cloned() else {
            self.status_message = "No card selected. Press n to create one.".to_string();
            return;
        };
        let prompt = card.prompt();
        if prompt.trim().is_empty() {
            self.status_message = "Card has no title or body to send as a grok prompt.".to_string();
            return;
        }

        let run_id = dispatch::new_run_id();
        if let Some(c) = self.board.get_mut(&card.id) {
            c.column = Column::Running;
            c.status = CardStatus::Running;
            c.run_id = Some(run_id.clone());
            c.last_summary = Some("Dispatching grok -p…".to_string());
        }
        self.focused = Column::Running;
        self.persist();

        let (tx, rx) = mpsc::channel();
        let card_id = card.id.clone();
        let run_id_thread = run_id.clone();
        thread::spawn(move || {
            let outcome = dispatch::run_headless(&card_id, &prompt, &run_id_thread);
            let _ = tx.send(outcome);
        });
        self.active_run = Some(ActiveRun {
            card_id: card.id,
            run_id,
            started: Instant::now(),
            rx,
        });
        self.status_message = "Started grok -p (headless). Card moved to Running.".to_string();
    }

    pub fn poll_dispatch(&mut self) {
        let Some(run) = self.active_run.as_ref() else {
            return;
        };
        match run.rx.try_recv() {
            Ok(outcome) => {
                self.active_run = None;
                self.apply_outcome(outcome);
            }
            Err(TryRecvError::Empty) => {
                let secs = self.run_elapsed_secs().unwrap_or(0);
                if let Some(id) = self.running_card_id().map(str::to_string) {
                    if let Some(card) = self.board.get_mut(&id) {
                        card.last_summary = Some(format!("running… {secs}s"));
                    }
                }
            }
            Err(TryRecvError::Disconnected) => {
                let card_id = run.card_id.clone();
                self.active_run = None;
                self.apply_outcome(DispatchOutcome {
                    card_id,
                    run_id: String::new(),
                    success: false,
                    summary: "grok worker thread ended unexpectedly.".to_string(),
                });
            }
        }
    }

    pub fn apply_outcome(&mut self, outcome: DispatchOutcome) {
        let Some(card) = self.board.get_mut(&outcome.card_id) else {
            self.status_message = "Dispatch finished, but the card was deleted.".to_string();
            return;
        };
        card.column = Column::Done;
        card.run_id = if outcome.run_id.is_empty() {
            card.run_id.clone()
        } else {
            Some(outcome.run_id.clone())
        };
        if outcome.success {
            card.status = CardStatus::Success;
            card.last_summary = Some(outcome.summary);
            self.status_message = "grok finished successfully → Done (ok).".to_string();
        } else {
            card.status = CardStatus::Failed;
            card.last_summary = Some(outcome.summary);
            self.status_message = "grok failed → Done (fail).".to_string();
        }
        self.focused = Column::Done;
        self.selected_id = Some(outcome.card_id);
        self.persist();
    }

    fn quit(&mut self) {
        if let Some(run) = self.active_run.take() {
            if let Some(card) = self.board.get_mut(&run.card_id) {
                card.column = Column::Done;
                card.status = CardStatus::Failed;
                card.last_summary =
                    Some("Run interrupted (quit while grok -p was still running).".to_string());
            }
            self.status_message = "Quit during a run — card marked Done (fail).".to_string();
        }
        self.persist();
        self.should_quit = true;
    }

    fn commit_editor(&mut self) {
        let Mode::Editor(state) = &self.mode else {
            return;
        };
        let title = state.title.trim().to_string();
        let body = state.body.clone();
        if title.is_empty() && body.trim().is_empty() {
            self.status_message = "Title or body is required.".to_string();
            return;
        }
        let title = if title.is_empty() {
            "Untitled".to_string()
        } else {
            title
        };

        if let Some(id) = state.card_id.clone() {
            if let Some(card) = self.board.get_mut(&id) {
                card.title = title;
                card.body = body;
            }
            self.selected_id = Some(id);
            self.status_message = "Card updated.".to_string();
        } else {
            let card = Card::new(title, body);
            self.selected_id = Some(card.id.clone());
            self.focused = Column::Backlog;
            self.board.add_card(card);
            self.status_message = "Card created in Backlog.".to_string();
        }
        self.mode = Mode::Board;
        self.persist();
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
            let next = (state.cursor as isize + delta).clamp(0, len) as usize;
            state.cursor = next;
        }
    }

    fn editor_insert(&mut self, ch: char) {
        if let Some(state) = self.editor_state_mut() {
            let field = match state.field {
                EditorField::Title => &mut state.title,
                EditorField::Body => &mut state.body,
            };
            if state.field == EditorField::Title && ch == '\n' {
                return;
            }
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
    use crate::model::CardStatus;
    use std::time::Duration;

    fn app_in_tmp() -> (App, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("board.json");
        let app = App::load_from(&path).unwrap();
        (app, dir)
    }

    fn wait_until_idle(app: &mut App) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.is_running() && Instant::now() < deadline {
            app.poll_dispatch();
            thread::sleep(Duration::from_millis(20));
        }
        app.poll_dispatch();
    }

    #[test]
    fn create_card_via_editor_persists() {
        let (mut app, dir) = app_in_tmp();
        app.handle_key(press(KeyCode::Char('n')));
        for ch in "Ship it".chars() {
            app.handle_key(press(KeyCode::Char(ch)));
        }
        app.handle_key(press(KeyCode::Enter));
        for ch in "Write the README".chars() {
            app.handle_key(press(KeyCode::Char(ch)));
        }
        let mut save = press(KeyCode::Char('s'));
        save.modifiers = KeyModifiers::CONTROL;
        app.handle_key(save);

        assert_eq!(app.board.cards.len(), 1);
        assert_eq!(app.board.cards[0].title, "Ship it");
        assert_eq!(app.board.cards[0].body, "Write the README");
        assert_eq!(app.board.cards[0].column, Column::Backlog);

        let reloaded = persist::load_board(&dir.path().join("board.json")).unwrap();
        assert_eq!(reloaded.cards.len(), 1);
        assert_eq!(reloaded.cards[0].title, "Ship it");
    }

    #[test]
    fn move_keys_change_column() {
        let (mut app, _dir) = app_in_tmp();
        app.board.add_card(Card::new("A", "b"));
        app.selected_id = Some(app.board.cards[0].id.clone());
        app.handle_key(press(KeyCode::Char('3')));
        assert_eq!(app.board.cards[0].column, Column::Done);
        app.handle_key(press(KeyCode::Char('1')));
        assert_eq!(app.board.cards[0].column, Column::Backlog);
    }

    #[test]
    fn dispatch_without_grok_lands_in_done_fail() {
        let (mut app, _dir) = app_in_tmp();
        std::env::set_var("AGENT_KANBAN_GROK", "agent-kanban-no-such-grok-binary");
        app.board.add_card(Card::new("Ask grok", "Say hi."));
        app.selected_id = Some(app.board.cards[0].id.clone());
        app.dispatch_selected();
        assert_eq!(app.board.cards[0].column, Column::Running);
        assert_eq!(app.board.cards[0].status, CardStatus::Running);
        wait_until_idle(&mut app);
        std::env::remove_var("AGENT_KANBAN_GROK");
        assert!(!app.is_running());
        assert_eq!(app.board.cards[0].column, Column::Done);
        assert_eq!(app.board.cards[0].status, CardStatus::Failed);
        assert!(app.board.cards[0]
            .last_summary
            .as_deref()
            .unwrap()
            .contains("not found"));
    }

    #[test]
    fn second_dispatch_is_refused() {
        let (mut app, _dir) = app_in_tmp();
        app.board.add_card(Card::new("One", "p"));
        app.board.add_card(Card::new("Two", "p"));
        let first = app.board.cards[0].id.clone();
        let second = app.board.cards[1].id.clone();
        app.selected_id = Some(first);
        // Inject a pending run so the second dispatch is refused without spawning grok.
        let (_tx, rx) = mpsc::channel();
        app.active_run = Some(ActiveRun {
            card_id: second.clone(),
            run_id: "hold".into(),
            started: Instant::now(),
            rx,
        });
        app.selected_id = Some(second);
        app.dispatch_selected();
        assert!(app.status_message.contains("already in progress"));
        assert_eq!(
            app.board
                .cards
                .iter()
                .filter(|c| c.status == CardStatus::Running)
                .count(),
            0
        );
    }

    #[test]
    fn quit_saves_and_sets_flag() {
        let (mut app, dir) = app_in_tmp();
        app.board.add_card(Card::new("Keep", "me"));
        app.handle_key(press(KeyCode::Char('q')));
        assert!(app.should_quit);
        let loaded = persist::load_board(&dir.path().join("board.json")).unwrap();
        assert_eq!(loaded.cards[0].title, "Keep");
    }
}
