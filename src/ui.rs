use ratatui::layout::{Alignment, Constraint, Direction, Flex, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::{App, EditorField, EditorState, Mode};
use crate::model::{Card, CardStatus, Column};

const TITLE_STYLE: Style = Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD);

pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(6),
            Constraint::Length(2),
            Constraint::Length(1),
        ])
        .split(area);

    draw_header(frame, chunks[0], app);
    draw_columns(frame, chunks[1], app);
    draw_status(frame, chunks[2], app);
    draw_footer(frame, chunks[3]);

    match &app.mode {
        Mode::Help => draw_help(frame, area),
        Mode::Editor(state) => draw_editor(frame, area, app, state),
        Mode::ConfirmDelete { title, .. } => draw_confirm_delete(frame, area, title),
        Mode::Board => {}
    }
}

fn draw_header(frame: &mut Frame, area: Rect, app: &App) {
    let path = app.board_path.display().to_string();
    let header = Line::from(vec![
        Span::styled(" Agent Kanban ", TITLE_STYLE),
        Span::styled("v0.1", Style::new().fg(Color::DarkGray)),
        Span::raw("  "),
        Span::styled(path, Style::new().fg(Color::DarkGray)),
    ]);
    frame.render_widget(Paragraph::new(header), area);
}

fn draw_columns(frame: &mut Frame, area: Rect, app: &App) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(34),
            Constraint::Percentage(33),
            Constraint::Percentage(33),
        ])
        .split(area);

    for (i, column) in Column::ALL.iter().enumerate() {
        draw_column(frame, cols[i], app, *column);
    }
}

fn column_border_style(column: Column, focused: bool) -> Style {
    let color = match column {
        Column::Backlog => Color::Blue,
        Column::Running => Color::Yellow,
        Column::Done => Color::Green,
    };
    if focused {
        Style::new().fg(color).add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(color)
    }
}

fn status_style(status: CardStatus) -> Style {
    match status {
        CardStatus::Idle => Style::new().fg(Color::DarkGray),
        CardStatus::Running => Style::new().fg(Color::Yellow),
        CardStatus::Success => Style::new().fg(Color::Green),
        CardStatus::Failed => Style::new().fg(Color::Red),
    }
}

fn draw_column(frame: &mut Frame, area: Rect, app: &App, column: Column) {
    let focused = app.focused == column;
    let cards = app.board.cards_in(column);
    let title = format!(
        " {} {} ({}) ",
        if focused { "▸" } else { " " },
        column.title(),
        cards.len()
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(column_border_style(column, focused));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if cards.is_empty() {
        let hint = if column == Column::Backlog {
            "empty — press n to create a card"
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
        lines.extend(card_lines(card, selected, inner.width));
        lines.push(Line::from(""));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn card_lines(card: &Card, selected: bool, width: u16) -> Vec<Line<'static>> {
    let marker = if selected { "▶" } else { " " };
    let max = width.saturating_sub(4) as usize;
    let title = crate::model::truncate_chars(&card.title, max.max(8));
    let title_style = if selected {
        Style::new()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD | Modifier::REVERSED)
    } else {
        Style::new().fg(Color::White)
    };
    let status = format!("{} {}", card.status.glyph(), card.status.label());
    let preview = card.summary_preview(max.max(8));
    let mut lines = vec![
        Line::from(vec![
            Span::styled(format!("{marker} "), title_style),
            Span::styled(title, title_style),
        ]),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(status, status_style(card.status)),
        ]),
    ];
    if !preview.is_empty() {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(preview, status_style(card.status)),
        ]));
    }
    lines
}

fn draw_status(frame: &mut Frame, area: Rect, app: &App) {
    let run = if let Some(secs) = app.run_elapsed_secs() {
        format!("run: active ({secs}s)")
    } else {
        "run: idle".to_string()
    };
    let msg = if app.status_message.is_empty() {
        "press ? for keys".to_string()
    } else {
        app.status_message.clone()
    };
    let line = Line::from(vec![
        Span::styled(format!(" {run} "), Style::new().fg(Color::Yellow)),
        Span::styled("│ ", Style::new().fg(Color::DarkGray)),
        Span::raw(msg),
    ]);
    frame.render_widget(
        Paragraph::new(line).block(Block::default().borders(Borders::TOP)),
        area,
    );
}

fn draw_footer(frame: &mut Frame, area: Rect) {
    let hints =
        " h/l cols  j/k cards  n new  Enter edit  r run  1/2/3 move  d del  ? help  q quit ";
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
    let popup = centered(area, 72, 20);
    frame.render_widget(Clear, popup);
    let text = vec![
        Line::from(Span::styled(
            "Keyboard",
            Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("  h / ← , l / →          focus column"),
        Line::from("  j / ↓ , k / ↑          select card in column"),
        Line::from("  Enter                  edit / view card detail"),
        Line::from("  n                      new card (Backlog)"),
        Line::from("  d                      delete selected (confirm y/n)"),
        Line::from("  1 / 2 / 3              move to Backlog / Running / Done"),
        Line::from("  r                      dispatch selected via grok -p"),
        Line::from("  q                      quit (saves the board)"),
        Line::from("  ?                      this help"),
        Line::from(""),
        Line::from("  One grok -p run at a time. Failed runs go to Done (fail)."),
        Line::from("  Esc or ? closes this overlay."),
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

fn draw_editor(frame: &mut Frame, area: Rect, app: &App, state: &EditorState) {
    let popup = centered(area, area.width.min(78), area.height.min(22));
    frame.render_widget(Clear, popup);
    let title = if state.card_id.is_some() {
        " Edit card "
    } else {
        " New card "
    };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::new().fg(Color::Cyan));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(6),
            Constraint::Length(4),
            Constraint::Length(2),
        ])
        .split(inner);

    let title_focused = state.field == EditorField::Title;
    let body_focused = state.field == EditorField::Body;
    frame.render_widget(
        Paragraph::new(with_cursor(&state.title, state.cursor, title_focused)).block(
            Block::default()
                .title(" Title ")
                .borders(Borders::ALL)
                .border_style(field_style(title_focused)),
        ),
        chunks[0],
    );
    frame.render_widget(
        Paragraph::new(with_cursor(&state.body, state.cursor, body_focused))
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .title(" Body / prompt ")
                    .borders(Borders::ALL)
                    .border_style(field_style(body_focused)),
            ),
        chunks[1],
    );

    let detail = match state.card_id.as_deref().and_then(|id| app.board.get(id)) {
        Some(card) => format!(
            "status: {} {}  ·  column: {}  ·  run: {}",
            card.status.glyph(),
            card.status.label(),
            card.column.title(),
            card.run_id.as_deref().unwrap_or("—")
        ),
        None => "new card → Backlog".to_string(),
    };
    let summary = state
        .card_id
        .as_deref()
        .and_then(|id| app.board.get(id))
        .and_then(|c| c.last_summary.clone())
        .unwrap_or_default();
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(detail, Style::new().fg(Color::DarkGray))),
            Line::from(Span::styled(
                crate::model::truncate_chars(&summary, 120),
                Style::new().fg(Color::Gray),
            )),
        ]),
        chunks[2],
    );
    frame.render_widget(
        Paragraph::new("Tab field  ·  Enter title→body / newline  ·  Ctrl+S save  ·  Esc cancel")
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

fn draw_confirm_delete(frame: &mut Frame, area: Rect, title: &str) {
    let popup = centered(area, 56, 7);
    frame.render_widget(Clear, popup);
    let text = vec![
        Line::from(""),
        Line::from(format!("  Delete card “{}”?", title)),
        Line::from(""),
        Line::from("  y confirm   n / Esc cancel"),
    ];
    frame.render_widget(
        Paragraph::new(text).block(
            Block::default()
                .title(" Confirm delete ")
                .borders(Borders::ALL)
                .border_style(Style::new().fg(Color::Red)),
        ),
        popup,
    );
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
    fn renders_three_fixed_columns() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::load_from(dir.path().join("board.json")).unwrap();
        app.board.add_card(Card::new("First card", "do the thing"));
        app.selected_id = Some(app.board.cards[0].id.clone());

        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &app)).unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("Backlog"));
        assert!(text.contains("Running"));
        assert!(text.contains("Done"));
        assert!(text.contains("First card"));
        assert!(text.contains("Agent Kanban"));
    }

    #[test]
    fn help_overlay_lists_keymap() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::load_from(dir.path().join("board.json")).unwrap();
        app.mode = Mode::Help;

        let backend = TestBackend::new(100, 28);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &app)).unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("dispatch selected via grok -p"));
        assert!(text.contains("new card"));
        assert!(text.contains("quit"));
    }
}
