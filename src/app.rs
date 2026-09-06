use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::dispatch::{self, DispatchConfig, DispatchOutcome, OutcomeKind};
use crate::model::{Board, Card, Status};
use crate::persist;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorField {
    Title,
    Body,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorCommit {
    /// Persist title/body; leave status unchanged.
    Save,
    /// Review revise: persist, bump `revision_count`, return to To Do.
    Revise,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorState {
    pub card_id: String,
    pub title: String,
    pub body: String,
    pub field: EditorField,
    pub cursor: usize,
    pub preferred_col: usize,
    pub scroll: u16,
    pub commit: EditorCommit,
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
    ReviewPrompt { card_id: String },
}

pub struct Inflight {
    pub card_id: String,
    pub rx: Receiver<DispatchOutcome>,
    pub kill: Sender<()>,
}

pub struct App {
    pub board: Board,
    pub board_path: PathBuf,
    pub focused: Status,
    pub selected_id: Option<String>,
    pub mode: Mode,
    pub status_message: String,
    pub should_quit: bool,
    pub dispatch_config: DispatchConfig,
    pub last_dispatch_wake: Instant,
    pub inflight: Option<Inflight>,
    /// Cards that failed dispatch this session; skipped until a human edit/move/revise.
    pub dispatch_cooldown: HashSet<String>,
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
            dispatch_config: DispatchConfig::from_env(),
            last_dispatch_wake: Instant::now(),
            inflight: None,
            dispatch_cooldown: HashSet::new(),
        };
        app.ensure_selection();
        app.reclaim_orphaned_in_progress();
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

    pub fn dispatching_title(&self) -> Option<String> {
        let id = self.inflight.as_ref()?.card_id.as_str();
        self.board.get(id).map(|c| c.title.clone())
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
            Mode::ReviewPrompt { .. } => self.handle_review_prompt_key(key),
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
            KeyCode::Char('r') => self.open_review_prompt(),
            KeyCode::Enter => self.open_editor(EditorCommit::Save),
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

    fn open_review_prompt(&mut self) {
        let Some(card) = self.selected_card().cloned() else {
            self.status_message = "No card selected.".to_string();
            return;
        };
        if card.status != Status::Review {
            self.status_message = "r accepts or revises a card in Review.".to_string();
            return;
        }
        self.mode = Mode::ReviewPrompt {
            card_id: card.id.clone(),
        };
        self.status_message = "Review — a accept → Done, v revise (edit body) → To Do.".to_string();
    }

    fn handle_review_prompt_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Board;
                self.status_message = "Review cancelled.".to_string();
            }
            KeyCode::Char('q') => self.quit(),
            KeyCode::Char('a') | KeyCode::Enter => self.accept_review(),
            KeyCode::Char('v') | KeyCode::Char('e') => self.revise_review(),
            _ => {}
        }
    }

    fn accept_review(&mut self) {
        let Mode::ReviewPrompt { card_id } = &self.mode else {
            return;
        };
        let id = card_id.clone();
        let Some(card) = self.board.get_mut(&id) else {
            self.mode = Mode::Board;
            return;
        };
        if card.status != Status::Review {
            self.mode = Mode::Board;
            self.status_message = "Card is no longer in Review.".to_string();
            return;
        }
        card.status = Status::Done;
        card.log("accept", "accepted from Review");
        self.focused = Status::Done;
        self.selected_id = Some(id);
        self.mode = Mode::Board;
        self.status_message = "Accepted → Done.".to_string();
        self.persist();
    }

    fn revise_review(&mut self) {
        let Mode::ReviewPrompt { card_id } = &self.mode else {
            return;
        };
        let id = card_id.clone();
        let Some(card) = self.board.get(&id).cloned() else {
            self.mode = Mode::Board;
            return;
        };
        self.selected_id = Some(id);
        self.open_editor_for(card, EditorCommit::Revise);
    }

    fn open_editor(&mut self, commit: EditorCommit) {
        let Some(card) = self.selected_card().cloned() else {
            self.status_message = "No card selected. Press n to create one.".to_string();
            return;
        };
        self.open_editor_for(card, commit);
    }

    fn open_editor_for(&mut self, card: Card, commit: EditorCommit) {
        let cursor = card.body.chars().count();
        let preferred_col = line_col(&card.body, cursor).1;
        self.mode = Mode::Editor(EditorState {
            card_id: card.id,
            title: card.title.clone(),
            body: card.body,
            field: EditorField::Body,
            cursor,
            preferred_col,
            scroll: 0,
            commit,
        });
        self.status_message = match commit {
            EditorCommit::Save => {
                "Editor — Tab title/body, ↑↓ lines, Ctrl+S save, Ctrl+T save→To Do, Esc cancel."
                    .to_string()
            }
            EditorCommit::Revise => {
                "Revise body/comments — Ctrl+S returns the card to To Do (rev + 1).".to_string()
            }
        };
    }

    fn handle_editor_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('s') => {
                    self.commit_editor(false);
                    return;
                }
                KeyCode::Char('t') => {
                    self.commit_editor(true);
                    return;
                }
                KeyCode::Char('a') | KeyCode::Home => {
                    self.editor_set_cursor(0);
                    return;
                }
                KeyCode::Char('e') | KeyCode::End => {
                    if let Some(len) = self.editor_field_len() {
                        self.editor_set_cursor(len);
                    }
                    return;
                }
                _ => {}
            }
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
            KeyCode::Up => self.editor_vertical(-1),
            KeyCode::Down => self.editor_vertical(1),
            KeyCode::PageUp => self.editor_vertical(-10),
            KeyCode::PageDown => self.editor_vertical(10),
            KeyCode::Home => self.editor_home(),
            KeyCode::End => self.editor_end(),
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.editor_insert(ch);
            }
            _ => {}
        }
    }

    /// `queue_todo`: after a normal save, move Capture → To Do (enrich workflow).
    fn commit_editor(&mut self, queue_todo: bool) {
        let Mode::Editor(state) = &self.mode else {
            return;
        };
        let id = state.card_id.clone();
        let title = state.title.trim().to_string();
        let body = state.body.clone();
        let commit = state.commit;
        if title.is_empty() {
            self.status_message = "Title is required.".to_string();
            return;
        }
        if let Some(card) = self.board.get_mut(&id) {
            card.title = title;
            card.body = body;
            card.touch();
            self.dispatch_cooldown.remove(&id);
            match commit {
                EditorCommit::Revise => {
                    card.status = Status::Todo;
                    card.revision_count = card.revision_count.saturating_add(1);
                    card.log("revise", "revised from Review; returned to To Do");
                    self.focused = Status::Todo;
                    self.status_message = format!("Revised → To Do (rev {}).", card.revision_count);
                }
                EditorCommit::Save => {
                    if queue_todo && card.status == Status::Capture {
                        card.status = Status::Todo;
                        card.log("edit", "enriched and queued to To Do");
                        self.focused = Status::Todo;
                        self.status_message = "Saved and moved to To Do.".to_string();
                    } else {
                        self.status_message = "Card updated.".to_string();
                    }
                }
            }
        }
        self.selected_id = Some(id);
        self.mode = Mode::Board;
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
        self.dispatch_cooldown.remove(&id);
        self.focused = next;
        self.status_message = format!("Moved to {}.", next.title());
        self.persist();
    }

    fn quit(&mut self) {
        self.cancel_inflight();
        self.reclaim_orphaned_in_progress();
        self.persist();
        self.should_quit = true;
    }

    /// Interval wake: finish a running job, then maybe pick one To Do card.
    pub fn on_tick(&mut self) {
        self.poll_dispatch();
        self.maybe_start_dispatch();
    }

    pub fn poll_dispatch(&mut self) {
        let Some(inf) = self.inflight.as_ref() else {
            return;
        };
        match inf.rx.try_recv() {
            Ok(outcome) => {
                let id = inf.card_id.clone();
                self.inflight = None;
                self.apply_outcome(&id, outcome);
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                let id = inf.card_id.clone();
                self.inflight = None;
                self.apply_outcome(
                    &id,
                    DispatchOutcome::failure("dispatcher thread ended unexpectedly"),
                );
            }
        }
    }

    pub fn maybe_start_dispatch(&mut self) {
        if !self.dispatch_config.enabled || self.inflight.is_some() {
            return;
        }
        if !matches!(self.mode, Mode::Board) {
            return;
        }
        if self.last_dispatch_wake.elapsed() < self.dispatch_config.interval {
            return;
        }
        self.last_dispatch_wake = Instant::now();
        let Some(id) = self.next_dispatch_card_id() else {
            return;
        };
        let Some(card) = self.board.get_mut(&id) else {
            return;
        };
        card.status = Status::InProgress;
        card.log("dispatch", "picked from To Do");
        let snapshot = card.clone();
        if self.selected_id.as_deref() == Some(id.as_str()) {
            self.focused = Status::InProgress;
        }
        let label = snapshot.title.clone();
        self.persist();

        let (rx, kill) = dispatch::spawn_dispatch(self.dispatch_config.clone(), snapshot);
        self.inflight = Some(Inflight {
            card_id: id,
            rx,
            kill,
        });
        let kind = if self.dispatch_config.stub {
            "stub"
        } else {
            "grok"
        };
        self.status_message = format!("Dispatching ({kind}): {label}");
    }

    fn next_dispatch_card_id(&self) -> Option<String> {
        self.board
            .cards_in(Status::Todo)
            .into_iter()
            .find(|c| !self.dispatch_cooldown.contains(&c.id))
            .map(|c| c.id.clone())
    }

    fn apply_outcome(&mut self, id: &str, outcome: DispatchOutcome) {
        let Some(card) = self.board.get_mut(id) else {
            return;
        };
        match outcome.kind {
            OutcomeKind::Success => {
                card.status = Status::Review;
                card.log("success", outcome.message);
                self.status_message = format!("Agent finished → Review ({})", card.title);
            }
            OutcomeKind::Failure => {
                card.status = Status::Todo;
                card.revision_count = card.revision_count.saturating_add(1);
                card.log("error", outcome.message);
                self.dispatch_cooldown.insert(id.to_string());
                self.status_message = format!(
                    "Agent failed → To Do (rev {}) — {}",
                    card.revision_count, card.title
                );
            }
        }
        if self.selected_id.as_deref() == Some(id) {
            if let Some(card) = self.board.get(id) {
                self.focused = card.status;
            }
        }
        self.persist();
    }

    fn cancel_inflight(&mut self) {
        let Some(inf) = self.inflight.take() else {
            return;
        };
        let _ = inf.kill.send(());
        let outcome = inf
            .rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap_or_else(|_| DispatchOutcome::failure("interrupted on quit"));
        // Quitting must never leave the card in In Progress, even if grok raced to success.
        let outcome = match outcome.kind {
            OutcomeKind::Success => outcome,
            OutcomeKind::Failure => {
                DispatchOutcome::failure(format!("interrupted on quit: {}", outcome.message))
            }
        };
        self.apply_outcome(&inf.card_id, outcome);
    }

    /// Crash / unclean exit: In Progress means the agent was working. Return to To Do.
    pub fn reclaim_orphaned_in_progress(&mut self) {
        let ids: Vec<String> = self
            .board
            .cards
            .iter()
            .filter(|c| c.status == Status::InProgress)
            .map(|c| c.id.clone())
            .collect();
        if ids.is_empty() {
            return;
        }
        for id in &ids {
            if let Some(card) = self.board.get_mut(id) {
                card.status = Status::Todo;
                card.revision_count = card.revision_count.saturating_add(1);
                card.log(
                    "error",
                    "reclaimed: process ended while card was in progress",
                );
                self.dispatch_cooldown.insert(id.clone());
            }
        }
        if self
            .selected_id
            .as_deref()
            .is_some_and(|id| ids.iter().any(|x| x == id))
        {
            self.focused = Status::Todo;
        }
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
            state.preferred_col = match state.field {
                EditorField::Title => state.cursor,
                EditorField::Body => line_col(&state.body, state.cursor).1,
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
            state.preferred_col = match field {
                EditorField::Title => state.cursor,
                EditorField::Body => line_col(&state.body, state.cursor).1,
            };
        }
    }

    fn editor_set_cursor(&mut self, pos: usize) {
        if let Some(state) = self.editor_state_mut() {
            let text = match state.field {
                EditorField::Title => &state.title,
                EditorField::Body => &state.body,
            };
            let len = text.chars().count();
            state.cursor = pos.min(len);
            state.preferred_col = line_col(text, state.cursor).1;
        }
    }

    fn editor_move_cursor(&mut self, delta: isize) {
        if let Some(state) = self.editor_state_mut() {
            let text = match state.field {
                EditorField::Title => &state.title,
                EditorField::Body => &state.body,
            };
            let len = text.chars().count() as isize;
            state.cursor = (state.cursor as isize + delta).clamp(0, len) as usize;
            state.preferred_col = line_col(text, state.cursor).1;
        }
    }

    fn editor_vertical(&mut self, dir: isize) {
        if self.editor_field() == Some(EditorField::Title) {
            if dir > 0 {
                self.editor_set_field(EditorField::Body);
            }
            return;
        }
        if let Some(state) = self.editor_state_mut() {
            if state.field != EditorField::Body {
                return;
            }
            let (line, _) = line_col(&state.body, state.cursor);
            let line_count = state.body.split('\n').count();
            let target =
                (line as isize + dir).clamp(0, line_count.saturating_sub(1) as isize) as usize;
            state.cursor = cursor_at_line_col(&state.body, target, state.preferred_col);
            let (new_line, _) = line_col(&state.body, state.cursor);
            if new_line < state.scroll as usize {
                state.scroll = new_line as u16;
            }
        }
    }

    fn editor_home(&mut self) {
        if let Some(state) = self.editor_state_mut() {
            match state.field {
                EditorField::Title => {
                    state.cursor = 0;
                    state.preferred_col = 0;
                }
                EditorField::Body => {
                    let (line, _) = line_col(&state.body, state.cursor);
                    state.cursor = cursor_at_line_col(&state.body, line, 0);
                    state.preferred_col = 0;
                }
            }
        }
    }

    fn editor_end(&mut self) {
        if let Some(state) = self.editor_state_mut() {
            match state.field {
                EditorField::Title => {
                    state.cursor = state.title.chars().count();
                    state.preferred_col = state.cursor;
                }
                EditorField::Body => {
                    let (line, _) = line_col(&state.body, state.cursor);
                    let col = usize::MAX;
                    state.cursor = cursor_at_line_col(&state.body, line, col);
                    state.preferred_col = line_col(&state.body, state.cursor).1;
                }
            }
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
            state.preferred_col = line_col(field, state.cursor).1;
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
            state.preferred_col = line_col(field, state.cursor).1;
        }
    }

    fn editor_delete(&mut self) {
        if let Some(state) = self.editor_state_mut() {
            let field = match state.field {
                EditorField::Title => &mut state.title,
                EditorField::Body => &mut state.body,
            };
            remove_char(field, state.cursor);
            state.preferred_col = line_col(field, state.cursor).1;
        }
    }
}

/// 0-based (line, column) for a character-index cursor.
pub fn line_col(text: &str, cursor: usize) -> (usize, usize) {
    let mut line = 0;
    let mut col = 0;
    for (i, ch) in text.chars().enumerate() {
        if i == cursor {
            return (line, col);
        }
        if ch == '\n' {
            line += 1;
            col = 0;
        } else {
            col += 1;
        }
    }
    (line, col)
}

pub fn cursor_at_line_col(text: &str, target_line: usize, target_col: usize) -> usize {
    let mut line = 0;
    for (i, ch) in text.chars().enumerate() {
        if line == target_line {
            let mut col = 0;
            for (j, c) in text[char_byte(text, i)..].chars().enumerate() {
                if c == '\n' || col == target_col {
                    return i + j;
                }
                col += 1;
            }
            return text.chars().count();
        }
        if ch == '\n' {
            line += 1;
        }
    }
    text.chars().count()
}

fn char_byte(text: &str, char_idx: usize) -> usize {
    text.char_indices()
        .nth(char_idx)
        .map(|(i, _)| i)
        .unwrap_or(text.len())
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
    use std::thread;
    use std::time::Duration;

    fn app_in_tmp() -> (App, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::load_from(dir.path().join("board.json")).unwrap();
        app.dispatch_config = DispatchConfig::stub_for_tests();
        (app, dir)
    }

    fn todo_card(title: &str) -> Card {
        let mut card = Card::new(title);
        card.status = Status::Todo;
        card.body = "do the work".into();
        card
    }

    /// Drive the interval wake until no job is in flight (stub completes on a later tick).
    fn pump_dispatch(app: &mut App) {
        for _ in 0..80 {
            app.on_tick();
            if app.inflight.is_none()
                && !app
                    .board
                    .cards
                    .iter()
                    .any(|c| c.status == Status::InProgress)
            {
                return;
            }
            thread::sleep(Duration::from_millis(5));
        }
        panic!(
            "dispatch still in progress: inflight={:?} statuses={:?}",
            app.inflight.as_ref().map(|i| i.card_id.clone()),
            app.board
                .cards
                .iter()
                .map(|c| (c.title.clone(), c.status))
                .collect::<Vec<_>>()
        );
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
    fn editor_up_down_and_ctrl_t_queue_capture_to_todo() {
        let (mut app, _dir) = app_in_tmp();
        let mut card = Card::new("Enrich me");
        card.body = "line1\nline2".into();
        app.board.add_card(card);
        app.selected_id = Some(app.board.cards[0].id.clone());
        app.handle_key(press(KeyCode::Enter));
        // cursor starts at end of body (line 1)
        app.handle_key(press(KeyCode::Up));
        match &app.mode {
            Mode::Editor(state) => {
                assert_eq!(line_col(&state.body, state.cursor), (0, 5));
            }
            other => panic!("expected editor, got {other:?}"),
        }
        app.handle_key(press(KeyCode::Down));
        match &app.mode {
            Mode::Editor(state) => {
                assert_eq!(line_col(&state.body, state.cursor).0, 1);
            }
            other => panic!("expected editor, got {other:?}"),
        }
        let mut queue = press(KeyCode::Char('t'));
        queue.modifiers = KeyModifiers::CONTROL;
        app.handle_key(queue);
        assert_eq!(app.board.cards[0].status, Status::Todo);
        assert_eq!(app.board.cards[0].last_log().unwrap().kind, "edit");
    }

    #[test]
    fn r_on_non_review_is_noop() {
        let (mut app, _dir) = app_in_tmp();
        app.board.add_card(Card::new("Inbox"));
        app.selected_id = Some(app.board.cards[0].id.clone());
        app.handle_key(press(KeyCode::Char('r')));
        assert_eq!(app.board.cards[0].status, Status::Capture);
        assert!(matches!(app.mode, Mode::Board));
        assert_eq!(app.board.cards[0].revision_count, 0);
    }

    #[test]
    fn review_r_accept_moves_to_done() {
        let (mut app, dir) = app_in_tmp();
        let mut card = Card::new("In review");
        card.status = Status::Review;
        app.board.add_card(card);
        app.selected_id = Some(app.board.cards[0].id.clone());
        app.focused = Status::Review;
        app.handle_key(press(KeyCode::Char('r')));
        assert!(matches!(app.mode, Mode::ReviewPrompt { .. }));
        app.handle_key(press(KeyCode::Char('a')));
        assert_eq!(app.board.cards[0].status, Status::Done);
        assert_eq!(app.board.cards[0].last_log().unwrap().kind, "accept");
        let loaded = persist::load_board(&dir.path().join("board.json")).unwrap();
        assert_eq!(loaded.cards[0].status, Status::Done);
    }

    #[test]
    fn review_r_revise_edits_body_and_returns_to_todo() {
        let (mut app, _dir) = app_in_tmp();
        let mut card = Card::new("Needs work");
        card.status = Status::Review;
        card.body = "first pass".into();
        app.board.add_card(card);
        app.selected_id = Some(app.board.cards[0].id.clone());
        app.focused = Status::Review;
        app.handle_key(press(KeyCode::Char('r')));
        app.handle_key(press(KeyCode::Char('v')));
        assert!(matches!(
            app.mode,
            Mode::Editor(EditorState {
                commit: EditorCommit::Revise,
                ..
            })
        ));
        for ch in " — please add tests".chars() {
            app.handle_key(press(KeyCode::Char(ch)));
        }
        let mut save = press(KeyCode::Char('s'));
        save.modifiers = KeyModifiers::CONTROL;
        app.handle_key(save);
        assert_eq!(app.board.cards[0].status, Status::Todo);
        assert_eq!(app.board.cards[0].revision_count, 1);
        assert!(app.board.cards[0].body.contains("please add tests"));
        assert_eq!(app.board.cards[0].last_log().unwrap().kind, "revise");
        assert!(app.board.cards[0].rev_badge().unwrap().contains("rev 1"));
    }

    #[test]
    fn stub_dispatch_success_moves_todo_to_review_with_log() {
        let (mut app, dir) = app_in_tmp();
        app.board.add_card(todo_card("Implement M2"));
        app.selected_id = Some(app.board.cards[0].id.clone());

        app.on_tick();
        assert_eq!(app.board.cards[0].status, Status::InProgress);
        assert!(app.inflight.is_some());
        assert_eq!(app.board.cards[0].last_log().unwrap().kind, "dispatch");

        pump_dispatch(&mut app);
        assert_eq!(app.board.cards[0].status, Status::Review);
        assert_eq!(app.board.cards[0].revision_count, 0);
        assert!(app.board.cards[0]
            .agent_log
            .iter()
            .any(|e| e.kind == "success"));
        assert!(
            app.board
                .cards
                .iter()
                .all(|c| c.status != Status::InProgress),
            "must not leave a card in In Progress"
        );

        let loaded = persist::load_board(&dir.path().join("board.json")).unwrap();
        assert_eq!(loaded.cards[0].status, Status::Review);
    }

    #[test]
    fn stub_dispatch_fail_returns_todo_bumps_rev_and_logs_error() {
        let (mut app, _dir) = app_in_tmp();
        app.dispatch_config.stub_fail = true;
        app.board.add_card(todo_card("Broken"));
        app.on_tick();
        assert_eq!(app.board.cards[0].status, Status::InProgress);
        pump_dispatch(&mut app);
        assert_eq!(app.board.cards[0].status, Status::Todo);
        assert_eq!(app.board.cards[0].revision_count, 1);
        assert!(app.board.cards[0]
            .agent_log
            .iter()
            .any(|e| e.kind == "error"));
        assert!(app.inflight.is_none());
    }

    #[test]
    fn stub_dispatch_picks_one_todo_at_a_time() {
        let (mut app, _dir) = app_in_tmp();
        app.dispatch_config.interval = Duration::from_secs(3600);
        app.last_dispatch_wake = Instant::now() - Duration::from_secs(3600);
        app.board.add_card(todo_card("First"));
        app.board.add_card(todo_card("Second"));

        app.on_tick();
        let in_progress: Vec<_> = app
            .board
            .cards
            .iter()
            .filter(|c| c.status == Status::InProgress)
            .collect();
        let todo: Vec<_> = app
            .board
            .cards
            .iter()
            .filter(|c| c.status == Status::Todo)
            .collect();
        assert_eq!(in_progress.len(), 1);
        assert_eq!(todo.len(), 1);
        assert_eq!(in_progress[0].title, "First");
        assert_eq!(todo[0].title, "Second");
    }

    #[test]
    fn failed_card_is_not_immediately_redispatched() {
        let (mut app, _dir) = app_in_tmp();
        app.dispatch_config.stub_fail = true;
        app.board.add_card(todo_card("No loop"));
        pump_dispatch(&mut app);
        assert_eq!(app.board.cards[0].revision_count, 1);
        app.on_tick();
        app.on_tick();
        assert!(app.inflight.is_none());
        assert_eq!(app.board.cards[0].status, Status::Todo);
        assert_eq!(app.board.cards[0].revision_count, 1);
    }

    #[test]
    fn quit_does_not_leave_in_progress() {
        let (mut app, dir) = app_in_tmp();
        app.board.add_card(todo_card("Abort me"));
        app.on_tick();
        assert_eq!(app.board.cards[0].status, Status::InProgress);
        app.handle_key(press(KeyCode::Char('q')));
        assert!(app.should_quit);
        assert_ne!(app.board.cards[0].status, Status::InProgress);
        let loaded = persist::load_board(&dir.path().join("board.json")).unwrap();
        assert_ne!(loaded.cards[0].status, Status::InProgress);
    }

    #[test]
    fn load_reclaims_orphaned_in_progress() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("board.json");
        let mut board = Board::default();
        let mut card = todo_card("Stuck");
        card.status = Status::InProgress;
        board.add_card(card);
        persist::save_board(&path, &board).unwrap();

        let app = App::load_from(&path).unwrap();
        assert_eq!(app.board.cards[0].status, Status::Todo);
        assert_eq!(app.board.cards[0].revision_count, 1);
        assert_eq!(app.board.cards[0].last_log().unwrap().kind, "error");
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

    #[test]
    fn line_col_helpers() {
        assert_eq!(line_col("ab\ncd", 0), (0, 0));
        assert_eq!(line_col("ab\ncd", 3), (1, 0));
        assert_eq!(cursor_at_line_col("ab\ncd", 1, 1), 4);
        assert_eq!(cursor_at_line_col("ab\ncd", 0, 99), 2);
    }
}
