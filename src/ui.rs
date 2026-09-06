use ratatui::layout::{Alignment, Constraint, Direction, Flex, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

use std::time::Duration;

use crate::app::{line_col, App, EditorCommit, EditorField, EditorState, InlineTitle, Mode};
use crate::model::{truncate_chars, Card, Status};

const TITLE_STYLE: Style = Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD);

pub const SPINNER_FRAMES: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

pub fn spinner_glyph(tick: u64) -> char {
    SPINNER_FRAMES[tick as usize % SPINNER_FRAMES.len()]
}

/// Status-line text while dispatch is in flight. `tick` must advance from the event loop.
pub fn format_running_line(kind: &str, title: &str, tick: u64, elapsed: Duration) -> String {
    format!(
        "{} Dispatching ({kind}): {} · {:.1}s",
        spinner_glyph(tick),
        truncate_chars(title, 40),
        elapsed.as_secs_f32()
    )
}

pub fn format_error_snippet(message: &str, max: usize) -> String {
    format!(
        "! {}",
        truncate_chars(message, max.saturating_sub(2).max(8))
    )
}

pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    match &app.mode {
        Mode::Editor(state) => {
            draw_fullscreen_editor(frame, area, app, state);
            return;
        }
        _ => draw_board_chrome(frame, area, app),
    }

    match &app.mode {
        Mode::Help => draw_help(frame, area),
        Mode::InlineTitle(state) => draw_inline_title(frame, area, state),
        Mode::ReviewPrompt { card_id } => draw_review_prompt(frame, area, app, card_id),
        Mode::Board | Mode::Editor(_) => {}
    }
}

fn draw_board_chrome(frame: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(6),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(area);

    draw_header(frame, chunks[0], app);
    draw_columns(frame, chunks[1], app);
    draw_status(frame, chunks[2], app);
    draw_footer(frame, chunks[3]);
}

fn draw_header(frame: &mut Frame, area: Rect, app: &App) {
    let path = app.board_path.display().to_string();
    let header = Line::from(vec![
        Span::styled(" Agent Kanban ", TITLE_STYLE),
        Span::styled(
            if app.dispatch_config.stub {
                "M3 · v0.2 · stub"
            } else {
                "M3 · v0.2"
            },
            Style::new().fg(Color::DarkGray),
        ),
        Span::raw("  "),
        Span::styled(path, Style::new().fg(Color::DarkGray)),
    ]);
    frame.render_widget(Paragraph::new(header), area);
}

fn draw_columns(frame: &mut Frame, area: Rect, app: &App) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(20),
            Constraint::Percentage(20),
            Constraint::Percentage(20),
            Constraint::Percentage(20),
            Constraint::Percentage(20),
        ])
        .split(area);

    for (i, status) in Status::ALL.iter().enumerate() {
        draw_column(frame, cols[i], app, *status);
    }
}

fn column_color(status: Status) -> Color {
    match status {
        Status::Capture => Color::Cyan,
        Status::Todo => Color::Blue,
        Status::InProgress => Color::Yellow,
        Status::Review => Color::Magenta,
        Status::Done => Color::Green,
    }
}

fn column_border_style(status: Status, focused: bool) -> Style {
    let color = column_color(status);
    if focused {
        Style::new().fg(color).add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(color)
    }
}

fn draw_column(frame: &mut Frame, area: Rect, app: &App, status: Status) {
    let focused = app.focused == status;
    let cards = app.board.cards_in(status);
    let running = status == Status::InProgress && app.inflight.is_some();
    let mark = if running {
        spinner_glyph(app.tick).to_string()
    } else if focused {
        "▸".to_string()
    } else {
        " ".to_string()
    };
    let title = format!(" {mark} {} ({}) ", status.title(), cards.len());
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(column_border_style(status, focused));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if cards.is_empty() {
        let hint = if status == Status::Capture {
            "n to capture"
        } else {
            "empty"
        };
        frame.render_widget(
            Paragraph::new(hint)
                .style(Style::new().fg(Color::DarkGray))
                .alignment(Alignment::Center),
            inner,
        );
        return;
    }

    let mut lines: Vec<Line> = Vec::new();
    for card in cards {
        let selected = app.selected_id.as_deref() == Some(card.id.as_str());
        lines.extend(card_lines(app, card, selected, inner.width));
        lines.push(Line::from(""));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn card_lines(app: &App, card: &Card, selected: bool, width: u16) -> Vec<Line<'static>> {
    let running = app.is_dispatching(&card.id);
    let marker = if running {
        spinner_glyph(app.tick).to_string()
    } else if selected {
        "▶".to_string()
    } else {
        " ".to_string()
    };
    let badge = card.rev_badge();
    let badge_width = badge.as_ref().map(|b| b.chars().count() + 1).unwrap_or(0);
    let max = (width as usize).saturating_sub(4 + badge_width).max(6);
    let title = truncate_chars(&card.title, max);
    let title_style = if running {
        Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD)
    } else if selected {
        Style::new()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD | Modifier::REVERSED)
    } else {
        Style::new().fg(Color::White)
    };
    let mut spans = vec![
        Span::styled(format!("{marker} "), title_style),
        Span::styled(title, title_style),
    ];
    if let Some(badge) = badge {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            badge,
            Style::new().fg(Color::Magenta).add_modifier(Modifier::BOLD),
        ));
    }
    let mut lines = vec![Line::from(spans)];
    if !running {
        if let Some(err) = card.board_error_snippet() {
            let snippet = format_error_snippet(&err, (width as usize).saturating_sub(2).max(10));
            lines.push(Line::from(Span::styled(
                format!("  {snippet}"),
                Style::new().fg(Color::Red),
            )));
        }
    }
    lines
}

fn draw_status(frame: &mut Frame, area: Rect, app: &App) {
    let col = app.focused.title();
    let running = app.running_status_line();
    let msg = if let Some(live) = running.clone() {
        live
    } else if app.status_message.is_empty() {
        "press ? for keys".to_string()
    } else {
        app.status_message.clone()
    };
    let live_style = if running.is_some() {
        Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD)
    } else if app.status_message.starts_with("Agent failed") {
        Style::new().fg(Color::Red)
    } else {
        Style::new()
    };
    let action = Line::from(vec![
        Span::styled(format!(" {col} "), Style::new().fg(Color::Cyan)),
        Span::styled("│ ", Style::new().fg(Color::DarkGray)),
        Span::styled(msg, live_style),
    ]);
    let detail_text = app
        .selected_outcome_line()
        .unwrap_or_else(|| "  press ? for keys".to_string());
    let detail_style = if app
        .selected_card()
        .and_then(|c| c.latest_outcome())
        .is_some_and(|o| o.kind == "error")
        && app.inflight.is_none()
    {
        Style::new().fg(Color::Red)
    } else {
        Style::new().fg(Color::DarkGray)
    };
    let detail = Line::from(Span::styled(format!(" {detail_text}"), detail_style));
    frame.render_widget(
        Paragraph::new(vec![action, detail]).block(Block::default().borders(Borders::TOP)),
        area,
    );
}

fn draw_footer(frame: &mut Frame, area: Rect) {
    let hints = " j/k select  h/l move  n capture  Enter edit  r review  ? help  q quit ";
    frame.render_widget(
        Paragraph::new(hints)
            .style(Style::new().fg(Color::DarkGray))
            .alignment(Alignment::Left),
        area,
    );
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let [h] = Layout::horizontal([Constraint::Length(width)])
        .flex(Flex::Center)
        .areas(area);
    let [v] = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(h);
    v
}

fn draw_help(frame: &mut Frame, area: Rect) {
    let popup = centered(area, 76, 26);
    frame.render_widget(Clear, popup);
    let text = vec![
        Line::from(Span::styled(
            "Keyboard (M3 live status + Review)",
            Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("  j / ↓ , k / ↑          select card in the focused column"),
        Line::from("  h / ←                  move focused card one column left"),
        Line::from("  l / →                  move focused card one column right"),
        Line::from("  n                      inline title → new card in Capture"),
        Line::from("  Enter                  full-screen body editor"),
        Line::from("  r                      Review: accept → Done, or revise → To Do"),
        Line::from("  q                      quit (auto-save ./board.json)"),
        Line::from("  ?                      this help"),
        Line::from(""),
        Line::from("  Dispatcher picks one To Do card (in-process interval; no cron)."),
        Line::from("  In Progress shows a live spinner (event-loop tick, no relaunch)."),
        Line::from("  Latest error/summary is on the status line and failed cards."),
        Line::from("  Persist: ./board.json   Esc or ? closes this overlay."),
    ];
    frame.render_widget(
        Paragraph::new(text).block(
            Block::default()
                .title(" Help ")
                .borders(Borders::ALL)
                .border_style(Style::new().fg(Color::Cyan)),
        ),
        popup,
    );
}

fn draw_inline_title(frame: &mut Frame, area: Rect, state: &InlineTitle) {
    let popup = centered(area, area.width.min(64), 5);
    frame.render_widget(Clear, popup);
    let line = with_cursor(&state.buffer, state.cursor, true);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                " New card → Capture ",
                Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            )),
            Line::from(line),
            Line::from(Span::styled(
                "Enter create  ·  Esc cancel",
                Style::new().fg(Color::DarkGray),
            )),
        ])
        .block(
            Block::default()
                .title(" Title ")
                .borders(Borders::ALL)
                .border_style(Style::new().fg(Color::Cyan)),
        ),
        popup,
    );
}

fn draw_review_prompt(frame: &mut Frame, area: Rect, app: &App, card_id: &str) {
    let popup = centered(area, area.width.min(64), 11);
    frame.render_widget(Clear, popup);
    let card = app.board.get(card_id);
    let title = card.map(|c| c.title.as_str()).unwrap_or("(missing)");
    let rev = card
        .and_then(|c| c.rev_badge())
        .unwrap_or_else(|| "rev 0".into());
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                " Review ",
                Style::new().fg(Color::Magenta).add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(format!("  {title}  ({rev})")),
            Line::from(""),
            Line::from("  a / Enter    accept → Done"),
            Line::from("  v            revise body/comments → To Do (rev + 1)"),
            Line::from("  Esc          cancel"),
        ])
        .block(
            Block::default()
                .title(" Accept or revise ")
                .borders(Borders::ALL)
                .border_style(Style::new().fg(Color::Magenta)),
        ),
        popup,
    );
}

fn draw_fullscreen_editor(frame: &mut Frame, area: Rect, app: &App, state: &EditorState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Min(6),
            Constraint::Length(2),
        ])
        .split(area);

    let card = app.board.get(&state.card_id);
    let (line, col) = match state.field {
        EditorField::Title => (0, state.cursor),
        EditorField::Body => line_col(&state.body, state.cursor),
    };
    let meta = match card {
        Some(c) => format!(
            " {}  ·  {}  ·  Ln {}, Col {}  ·  {} ",
            c.status.title(),
            c.rev_badge().unwrap_or_else(|| "rev 0".into()),
            line + 1,
            col + 1,
            c.id
        ),
        None => format!(" Ln {}, Col {} ", line + 1, col + 1),
    };
    let heading = match state.commit {
        EditorCommit::Revise => " Revise (save → To Do) ",
        EditorCommit::Save => " Full-screen editor ",
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(heading, TITLE_STYLE),
            Span::styled(meta, Style::new().fg(Color::DarkGray)),
        ])),
        chunks[0],
    );

    let title_focused = state.field == EditorField::Title;
    frame.render_widget(
        Paragraph::new(with_cursor(&state.title, state.cursor, title_focused)).block(
            Block::default()
                .title(" Title ")
                .borders(Borders::ALL)
                .border_style(field_style(title_focused)),
        ),
        chunks[1],
    );

    let body_focused = state.field == EditorField::Body;
    let body_block = Block::default()
        .title(" Body / comments / context ")
        .borders(Borders::ALL)
        .border_style(field_style(body_focused));
    let inner_h = body_block.inner(chunks[2]).height;
    let mut scroll = state.scroll;
    if body_focused {
        let line = line as u16;
        if line < scroll {
            scroll = line;
        } else if inner_h > 0 && line >= scroll.saturating_add(inner_h) {
            scroll = line.saturating_sub(inner_h.saturating_sub(1));
        }
    }
    frame.render_widget(
        Paragraph::new(with_cursor(&state.body, state.cursor, body_focused))
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0))
            .block(body_block),
        chunks[2],
    );

    frame.render_widget(
        Paragraph::new(
            " Tab fields  ·  ↑↓ lines  ·  Home/End line  ·  Ctrl+S save  ·  Ctrl+T save→To Do  ·  Esc ",
        )
        .style(Style::new().fg(Color::DarkGray)),
        chunks[3],
    );
}

fn field_style(focused: bool) -> Style {
    if focused {
        Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(Color::DarkGray)
    }
}

fn with_cursor(text: &str, cursor: usize, show: bool) -> String {
    if !show {
        return text.to_string();
    }
    let mut out = String::new();
    let mut placed = false;
    for (i, ch) in text.chars().enumerate() {
        if i == cursor {
            out.push('▏');
            placed = true;
        }
        out.push(ch);
    }
    if !placed {
        out.push('▏');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::model::Card;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
        let buf = terminal.backend().buffer();
        let mut out = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn renders_five_columns_on_one_screen() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::load_from(dir.path().join("board.json")).unwrap();
        let mut card = Card::new("First card");
        card.revision_count = 2;
        app.board.add_card(card);
        app.selected_id = Some(app.board.cards[0].id.clone());

        let backend = TestBackend::new(140, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &app)).unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("Capture"), "{text}");
        assert!(text.contains("To Do"), "{text}");
        assert!(text.contains("In Progress"), "{text}");
        assert!(text.contains("Review"), "{text}");
        assert!(text.contains("Done"), "{text}");
        assert!(text.contains("First card"), "{text}");
        assert!(text.contains("rev 2"), "{text}");
        assert!(text.contains("Agent Kanban"));
        assert!(!text.contains("Backlog"));
        assert!(!text.contains("Running"));
        assert!(!text.contains("grok"));
    }

    #[test]
    fn help_overlay_lists_m3_keymap() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::load_from(dir.path().join("board.json")).unwrap();
        app.mode = Mode::Help;

        let backend = TestBackend::new(120, 32);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &app)).unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("new card in Capture"));
        assert!(text.contains("select card in the focused column"));
        assert!(text.contains("./board.json"));
        assert!(text.contains("quit"));
        assert!(text.contains("accept"));
        assert!(text.contains("revise"));
        assert!(text.contains("To Do"));
        assert!(text.contains("spinner") || text.contains("live"));
        assert!(text.contains("error") || text.contains("summary"));
    }

    fn hung_inflight(card_id: String) -> crate::app::Inflight {
        let (_tx, rx) = std::sync::mpsc::channel();
        let (kill, _kill_rx) = std::sync::mpsc::channel();
        crate::app::Inflight {
            card_id,
            rx,
            kill,
            started: std::time::Instant::now(),
        }
    }

    #[test]
    fn inflight_card_shows_spinner_that_changes_on_tick() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::load_from(dir.path().join("board.json")).unwrap();
        let mut card = Card::new("Running now");
        card.status = crate::model::Status::InProgress;
        app.board.add_card(card);
        app.selected_id = Some(app.board.cards[0].id.clone());
        app.focused = crate::model::Status::InProgress;
        app.inflight = Some(hung_inflight(app.board.cards[0].id.clone()));
        app.tick = 0;

        let backend = TestBackend::new(140, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &app)).unwrap();
        let first = buffer_text(&terminal);
        assert!(first.contains("Running now"), "{first}");
        assert!(first.contains(spinner_glyph(0)), "{first}");
        assert!(first.contains("Dispatching"), "{first}");

        app.pulse();
        terminal.draw(|f| draw(f, &app)).unwrap();
        let second = buffer_text(&terminal);
        assert!(second.contains(spinner_glyph(1)), "{second}");
        assert_ne!(
            spinner_glyph(0),
            spinner_glyph(1),
            "pulse must change the glyph without a keypress"
        );
    }

    #[test]
    fn failed_card_error_is_visible_in_board_ui() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::load_from(dir.path().join("board.json")).unwrap();
        let mut card = Card::new("Broken card");
        card.status = crate::model::Status::Todo;
        card.revision_count = 1;
        card.log("error", "missing binary: grok");
        app.board.add_card(card);
        app.selected_id = Some(app.board.cards[0].id.clone());
        app.focused = crate::model::Status::Todo;
        app.status_message = "Agent failed → To Do (rev 1) — missing binary: grok".into();

        let backend = TestBackend::new(140, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &app)).unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("Broken card"), "{text}");
        assert!(
            text.contains("missing binary: grok"),
            "error must be readable without JSON, got {text}"
        );
        assert!(text.contains("error") || text.contains("!"), "{text}");
    }

    #[test]
    fn editor_is_fullscreen_not_a_small_popup() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::load_from(dir.path().join("board.json")).unwrap();
        app.board.add_card(Card::new("Edit me"));
        app.selected_id = Some(app.board.cards[0].id.clone());
        app.mode = Mode::Editor(EditorState {
            card_id: app.board.cards[0].id.clone(),
            title: "Edit me".into(),
            body: "context".into(),
            field: EditorField::Body,
            cursor: 0,
            preferred_col: 0,
            scroll: 0,
            commit: EditorCommit::Save,
        });

        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &app)).unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("Full-screen editor"));
        assert!(text.contains("Body / comments / context") || text.contains("comments"));
        assert!(text.contains("context"));
    }
}
