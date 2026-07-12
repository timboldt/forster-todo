//! Terminal rendering with ratatui.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};

use crate::app::App;
use crate::fvp::Mode;
use crate::session::Session;
use crate::task::{Status, Task};

// Palette: GitHub-dark accents, matching the web view (src/web/index.html).
const GREEN: Color = Color::Rgb(126, 231, 135); // #7ee787 — selection, DO NOW
const GREEN_BG: Color = Color::Rgb(20, 56, 30); // #14381e — selected row bg
const AMBER: Color = Color::Rgb(227, 179, 65); // #e3b341 — dotted tasks
const BLUE: Color = Color::Rgb(121, 192, 255); // #79c0ff — key hints
const FG: Color = Color::Rgb(230, 237, 243); // #e6edf3 — primary text
const MUTED: Color = Color::Rgb(139, 148, 158); // #8b949e — done, hints
const BORDER: Color = Color::Rgb(48, 54, 61); // #30363d — panel borders
const BG: Color = Color::Rgb(13, 17, 23); // #0d1117 — overlay background

/// Original indices of the tasks to display, per mode:
/// - **Preselect**: hide open tasks above the benchmark (the bottom-most dotted
///   task) — they were skipped and are out of the running for this scan.
/// - **Action**: show only the dotted chain (the queue); the last one is "DO NOW".
/// - **Empty**: show everything (i.e. any completed-task history).
fn visible_indices(tasks: &[Task], mode: Mode) -> Vec<usize> {
    tasks
        .iter()
        .enumerate()
        .filter(|(i, t)| match mode {
            Mode::Preselect { benchmark, .. } => !(t.is_open() && *i < benchmark),
            Mode::Action { .. } => t.is_dotted(),
            Mode::Empty => true,
        })
        .map(|(i, _)| i)
        .collect()
}

/// Render the whole UI for the current frame.
pub fn draw(frame: &mut Frame, session: &Session, app: &App) {
    let chunks = Layout::vertical([
        Constraint::Length(1), // title
        Constraint::Min(1),    // task list
        Constraint::Length(3), // prompt / status bar
    ])
    .split(frame.area());

    draw_title(frame, chunks[0]);
    draw_list(frame, chunks[1], session, app);
    draw_status(frame, chunks[2], session, app);

    if app.show_help {
        draw_help(frame);
    }
}

fn draw_title(frame: &mut Frame, area: Rect) {
    let title = Line::from(vec![
        Span::styled(
            " Mark Forster's FVP ",
            Style::default()
                .fg(FG)
                .bg(BG)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  ? for help", Style::default().fg(MUTED)),
    ]);
    frame.render_widget(Paragraph::new(title), area);
}

fn draw_list(frame: &mut Frame, area: Rect, session: &Session, app: &App) {
    let browsing = app.browse.is_some();
    let selected = if browsing {
        app.browse.filter(|_| !session.tasks.is_empty())
    } else {
        match session.mode {
            Mode::Preselect { cursor, .. } => Some(cursor),
            Mode::Action { task } => Some(task),
            Mode::Empty => None,
        }
    };
    let benchmark = match session.mode {
        Mode::Preselect { benchmark, .. } if !browsing => Some(benchmark),
        _ => None,
    };

    // Browse mode and the add/edit overlay show the whole list for context
    // rather than the (possibly narrow) filtered view of the underlying mode.
    let filter_mode = if app.input.is_some() || browsing {
        Mode::Empty
    } else {
        session.mode
    };
    let visible = visible_indices(&session.tasks, filter_mode);

    let items: Vec<ListItem> = visible
        .iter()
        .map(|&i| {
            let t = &session.tasks[i];
            let (marker, mut style) = match t.status {
                Status::Done => (
                    "[x] ",
                    Style::default()
                        .fg(MUTED)
                        .add_modifier(Modifier::CROSSED_OUT),
                ),
                Status::Dotted => (
                    "[.] ",
                    Style::default().fg(AMBER).add_modifier(Modifier::BOLD),
                ),
                Status::Open => ("[ ] ", Style::default().fg(FG)),
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

    // Map the selected task's original index to its row in the filtered list.
    let selected = selected.and_then(|s| visible.iter().position(|&i| i == s));

    let list = List::new(items)
        .block(panel().title(Span::styled(
            " Tasks ",
            Style::default().fg(GREEN).add_modifier(Modifier::BOLD),
        )))
        // Bright green on a dark green background, like a diff "added" line.
        .highlight_style(
            Style::default()
                .bg(GREEN_BG)
                .fg(GREEN)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    let mut state = ListState::default();
    state.select(selected);
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_status(frame: &mut Frame, area: Rect, session: &Session, app: &App) {
    let block = panel();

    // The input overlay (add or edit) takes over the status bar.
    if let Some(buf) = &app.input {
        let (title, hint) = if app.input_target.is_some() {
            ("Edit task: ", "   (Enter save · Esc cancel)")
        } else {
            ("New task: ", "   (Enter add another · Esc done)")
        };
        let line = Line::from(vec![
            Span::styled(
                title,
                Style::default().fg(GREEN).add_modifier(Modifier::BOLD),
            ),
            Span::styled(buf.as_str(), Style::default().fg(FG)),
            Span::styled("▏", Style::default().fg(GREEN)),
            Span::styled(hint, Style::default().fg(MUTED)),
        ]);
        frame.render_widget(Paragraph::new(line).block(block), area);
        return;
    }

    // Browse mode has its own bar regardless of the underlying FVP mode.
    if app.browse.is_some() {
        let line = Line::from(vec![
            Span::styled(
                "Browse  ",
                Style::default().fg(GREEN).add_modifier(Modifier::BOLD),
            ),
            Span::styled("↑/↓", key()),
            Span::raw(" move · "),
            Span::styled("Space", key()),
            Span::raw(" done/undone · "),
            Span::styled(".", key()),
            Span::raw(" dot · "),
            Span::styled("Enter", key()),
            Span::raw(" edit · "),
            Span::styled("Esc", key()),
            Span::raw(" back · "),
            Span::styled("s", key()),
            Span::raw(" scan"),
        ]);
        frame.render_widget(Paragraph::new(line).block(block), area);
        return;
    }

    let line = match session.mode {
        Mode::Empty => Line::from(vec![
            Span::styled("No active tasks. ", Style::default().fg(GREEN)),
            Span::styled("a", key()),
            Span::raw(" add · "),
            Span::styled("Esc", key()),
            Span::raw(" browse · "),
            Span::styled("q", key()),
            Span::raw(" quit"),
        ]),
        Mode::Preselect { benchmark, cursor } => {
            let mut spans = vec![
                Span::styled("Do more than ", Style::default().fg(FG)),
                Span::styled(
                    format!("«{}»", session.tasks[benchmark].text),
                    Style::default().fg(AMBER).add_modifier(Modifier::BOLD),
                ),
                Span::raw("?  "),
                Span::styled("↑/↓", key()),
                Span::raw(" move · "),
                Span::styled("→", key()),
                Span::raw(" dot · "),
                Span::styled("←", key()),
                Span::raw(" undo dot · "),
                Span::styled("Esc", key()),
                Span::raw(" finish · "),
                Span::styled("a", key()),
                Span::raw(" add"),
            ];
            // At the last candidate, remind that the scan ends explicitly.
            let at_end = !session.tasks.iter().skip(cursor + 1).any(|t| t.is_open());
            if at_end {
                spans.push(Span::styled(
                    "  · end of list — Esc finishes",
                    Style::default().fg(AMBER),
                ));
            }
            Line::from(spans)
        }
        Mode::Action { task } => Line::from(vec![
            Span::styled(
                "▶ DO NOW: ",
                Style::default().fg(GREEN).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                session.tasks[task].text.clone(),
                Style::default().fg(FG).add_modifier(Modifier::BOLD),
            ),
            Span::raw("   "),
            Span::styled("Space", key()),
            Span::raw(" done · "),
            Span::styled("s", key()),
            Span::raw(" scan · "),
            Span::styled("Esc", key()),
            Span::raw(" browse · "),
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
    Style::default().fg(BLUE).add_modifier(Modifier::BOLD)
}

/// Bordered panel in the standard border color.
fn panel() -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(BORDER))
}

fn draw_help(frame: &mut Frame) {
    let area = centered_rect(70, 90, frame.area());
    let head = |s: &'static str| {
        Line::from(Span::styled(
            s,
            Style::default().fg(GREEN).add_modifier(Modifier::BOLD),
        ))
    };
    let bind = |k: &'static str, desc: &'static str| {
        Line::from(vec![
            Span::styled(format!("  {k:<11}"), key()),
            Span::styled(desc, Style::default().fg(FG)),
        ])
    };
    let text = vec![
        Line::from(Span::styled(
            "FVP — Final Version Perfected",
            Style::default().fg(AMBER).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "During a scan, ask of each task: do I want to",
            Style::default().fg(FG),
        )),
        Line::from(Span::styled(
            "do it MORE than the underlined benchmark?",
            Style::default().fg(FG),
        )),
        Line::from(Span::styled(
            "The last dotted task is always done next.",
            Style::default().fg(FG),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("Esc", key()),
            Span::styled(" zooms out: Scan → Do → Browse. ", Style::default().fg(FG)),
            Span::styled("s", key()),
            Span::styled(" dives back in.", Style::default().fg(FG)),
        ]),
        Line::from(""),
        head("Scan (pre-select):"),
        bind("↑/↓", "move between candidates (bounded)"),
        bind("→", "dot the current task"),
        bind("←", "undo the last dot"),
        bind("Esc", "finish the scan → Do"),
        Line::from(""),
        head("Do (action):"),
        bind("Space", "mark the DO NOW task done"),
        bind("s", "resume scanning (dot more tasks)"),
        bind("Esc", "browse the full list"),
        Line::from(""),
        head("Browse:"),
        bind("↑/↓", "move over all tasks (incl. done)"),
        bind("Space", "toggle done/undone"),
        bind(".", "toggle the dot"),
        bind("Enter / e", "edit the task text"),
        bind("Esc", "back to Do · s resume scanning"),
        Line::from(""),
        head("Anywhere:"),
        bind("a", "add tasks (Enter adds another, Esc done)"),
        bind("p", "purge done tasks (backs up first)"),
        bind("? / q", "help · save & quit"),
        Line::from(""),
        Line::from(Span::styled(
            "Press ? or Esc to close",
            Style::default().fg(MUTED),
        )),
    ];
    let block = panel()
        .title(Span::styled(
            " Help ",
            Style::default().fg(GREEN).add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(BG));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn task(text: &str, status: Status) -> Task {
        Task {
            text: text.into(),
            status,
        }
    }

    #[test]
    fn empty_mode_shows_everything() {
        let tasks = vec![
            task("a", Status::Done),
            task("b", Status::Open),
            task("c", Status::Dotted),
        ];
        assert_eq!(visible_indices(&tasks, Mode::Empty), vec![0, 1, 2]);
    }

    #[test]
    fn scanning_hides_skipped_open_tasks_above_benchmark() {
        // Chain: a dotted(0), b skipped(1), c dotted+benchmark(2),
        // d open candidate(3), e open candidate below benchmark shown(4).
        let tasks = vec![
            task("a", Status::Dotted),
            task("b", Status::Open), // skipped, above benchmark -> hidden
            task("c", Status::Dotted),
            task("d", Status::Open), // candidate below benchmark -> shown
            task("e", Status::Open),
        ];
        let mode = Mode::Preselect {
            benchmark: 2,
            cursor: 3,
        };
        assert_eq!(visible_indices(&tasks, mode), vec![0, 2, 3, 4]);
    }

    #[test]
    fn scanning_keeps_dotted_and_done_above_benchmark() {
        // Only *open* skipped tasks are hidden; dotted and done stay visible.
        let tasks = vec![
            task("done-above", Status::Done),
            task("dotted-above", Status::Dotted),
            task("skipped", Status::Open), // hidden
            task("benchmark", Status::Dotted),
        ];
        let mode = Mode::Preselect {
            benchmark: 3,
            cursor: 3,
        };
        assert_eq!(visible_indices(&tasks, mode), vec![0, 1, 3]);
    }

    #[test]
    fn action_mode_shows_only_the_dotted_chain() {
        let tasks = vec![
            task("done", Status::Done),
            task("dotted-1", Status::Dotted),
            task("open", Status::Open),
            task("dotted-2 (DO NOW)", Status::Dotted),
        ];
        let mode = Mode::Action { task: 3 };
        assert_eq!(visible_indices(&tasks, mode), vec![1, 3]);
    }
}
