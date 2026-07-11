//! Terminal rendering with ratatui.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};

use crate::app::App;
use crate::fvp::Mode;
use crate::task::Status;

/// Render the whole UI for the current frame.
pub fn draw(frame: &mut Frame, app: &App) {
    let chunks = Layout::vertical([
        Constraint::Length(1), // title
        Constraint::Min(1),    // task list
        Constraint::Length(3), // prompt / status bar
    ])
    .split(frame.area());

    draw_title(frame, chunks[0]);
    draw_list(frame, chunks[1], app);
    draw_status(frame, chunks[2], app);

    if app.show_help {
        draw_help(frame);
    }
}

fn draw_title(frame: &mut Frame, area: Rect) {
    let title = Line::from(vec![
        Span::styled(
            " forster-todo ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  Mark Forster's FVP  "),
        Span::styled("(? for help)", Style::default().fg(Color::DarkGray)),
    ]);
    frame.render_widget(Paragraph::new(title), area);
}

fn draw_list(frame: &mut Frame, area: Rect, app: &App) {
    let selected = match app.mode {
        Mode::Preselect { cursor, .. } => Some(cursor),
        Mode::Action { task } => Some(task),
        Mode::Empty => None,
    };
    let benchmark = match app.mode {
        Mode::Preselect { benchmark, .. } => Some(benchmark),
        _ => None,
    };

    let items: Vec<ListItem> = app
        .tasks
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let (marker, mut style) = match t.status {
                Status::Done => (
                    "[x] ",
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::CROSSED_OUT),
                ),
                Status::Dotted => (
                    "[.] ",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Status::Open => ("[ ] ", Style::default().fg(Color::Gray)),
            };
            if Some(i) == benchmark {
                style = style.add_modifier(Modifier::UNDERLINED);
            }
            let text = if t.text.is_empty() {
                "(empty)"
            } else {
                t.text.as_str()
            };
            ListItem::new(Line::from(vec![
                Span::styled(marker, style),
                Span::styled(text, style),
            ]))
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(" Tasks "))
        .highlight_style(
            Style::default()
                .bg(Color::Blue)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    let mut state = ListState::default();
    state.select(selected);
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_status(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::default().borders(Borders::ALL);

    // Adding a task takes over the status bar as an input line.
    if let Some(buf) = &app.input {
        let line = Line::from(vec![
            Span::styled("New task: ", Style::default().fg(Color::Cyan)),
            Span::raw(buf.as_str()),
            Span::styled("▏", Style::default().fg(Color::Cyan)),
            Span::styled(
                "   (Enter save · Esc cancel)",
                Style::default().fg(Color::DarkGray),
            ),
        ]);
        frame.render_widget(Paragraph::new(line).block(block), area);
        return;
    }

    let line = match app.mode {
        Mode::Empty => Line::from(vec![
            Span::styled("No active tasks. ", Style::default().fg(Color::Green)),
            Span::styled("a", key()),
            Span::raw(" add · "),
            Span::styled("q", key()),
            Span::raw(" quit"),
        ]),
        Mode::Preselect { benchmark, .. } => Line::from(vec![
            Span::styled("Do more than ", Style::default().fg(Color::White)),
            Span::styled(
                format!("«{}»", app.tasks[benchmark].text),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("?  "),
            Span::styled("↑/↓", key()),
            Span::raw(" move · "),
            Span::styled("Enter/→", key()),
            Span::raw(" dot · "),
            Span::styled("Esc", key()),
            Span::raw(" finish · "),
            Span::styled("a", key()),
            Span::raw(" add"),
        ]),
        Mode::Action { task } => Line::from(vec![
            Span::styled(
                "▶ DO NOW: ",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                app.tasks[task].text.clone(),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("   "),
            Span::styled("Space", key()),
            Span::raw(" done · "),
            Span::styled("a", key()),
            Span::raw(" add · "),
            Span::styled("q", key()),
            Span::raw(" quit"),
        ]),
    };
    frame.render_widget(Paragraph::new(line).block(block), area);
}

/// Style for a highlighted key hint.
fn key() -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
}

fn draw_help(frame: &mut Frame) {
    let area = centered_rect(60, 60, frame.area());
    let text = vec![
        Line::from(Span::styled(
            "FVP — Final Version Perfected",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("During a scan, ask of each task: do I want to"),
        Line::from("do it MORE than the underlined benchmark?"),
        Line::from("The last dotted task is always done next."),
        Line::from(""),
        Line::from(vec![Span::styled(
            "Keys:",
            Style::default().add_modifier(Modifier::BOLD),
        )]),
        Line::from("  ↑/↓        move between candidates"),
        Line::from("  Enter / →  dot the current task"),
        Line::from("  Esc        finish the scan"),
        Line::from("  Space      mark the current task done"),
        Line::from("  a          add a task"),
        Line::from("  ?          toggle this help"),
        Line::from("  q          save & quit"),
        Line::from(""),
        Line::from(Span::styled(
            "Press ? or Esc to close",
            Style::default().fg(Color::DarkGray),
        )),
    ];
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Help ")
        .style(Style::default().bg(Color::Black));
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(text).block(block).wrap(Wrap { trim: true }),
        area,
    );
}

/// A rectangle centered within `area`, sized as a percentage of it.
fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::vertical([
        Constraint::Percentage((100 - percent_y) / 2),
        Constraint::Percentage(percent_y),
        Constraint::Percentage((100 - percent_y) / 2),
    ])
    .split(area);
    Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2),
    ])
    .split(vertical[1])[1]
}
