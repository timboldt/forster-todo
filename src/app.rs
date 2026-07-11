//! TUI presentation state and key handling. All domain state lives in the
//! shared [`Session`]; this layer only owns terminal-specific concerns (the
//! add-task input buffer, the help overlay, and the quit flag).

use std::sync::{Arc, Mutex, MutexGuard};

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};

use crate::fvp::Mode;
use crate::session::Session;

pub struct App {
    pub session: Arc<Mutex<Session>>,
    /// `Some` while the user is typing a new task in the add overlay.
    pub input: Option<String>,
    pub show_help: bool,
    pub should_quit: bool,
}

impl App {
    pub fn new(session: Arc<Mutex<Session>>) -> Self {
        App {
            session,
            input: None,
            show_help: false,
            should_quit: false,
        }
    }

    fn session(&self) -> MutexGuard<'_, Session> {
        self.session.lock().expect("session lock poisoned")
    }

    /// Handle a key press, mutating state and saving when the list changes.
    pub fn on_key(&mut self, key: KeyEvent) -> Result<()> {
        // Ignore key-release events (Windows / some terminals emit them).
        if key.kind == KeyEventKind::Release {
            return Ok(());
        }

        if self.input.is_some() {
            self.on_input_key(key)?;
            return Ok(());
        }

        // Help overlay swallows most keys; ? or Esc/q closes it.
        if self.show_help {
            match key.code {
                KeyCode::Char('?') | KeyCode::Esc | KeyCode::Char('q') => self.show_help = false,
                _ => {}
            }
            return Ok(());
        }

        match key.code {
            KeyCode::Char('q') => {
                self.session().save()?;
                self.should_quit = true;
            }
            KeyCode::Char('?') => self.show_help = true,
            KeyCode::Char('a') | KeyCode::Char('A') => self.input = Some(String::new()),
            _ => self.on_mode_key(key)?,
        }
        Ok(())
    }

    /// Keys that depend on the current FVP mode.
    fn on_mode_key(&mut self, key: KeyEvent) -> Result<()> {
        let mut session = self.session.lock().expect("session lock poisoned");
        match session.mode {
            Mode::Preselect { .. } => match key.code {
                KeyCode::Up => session.move_up(),
                KeyCode::Down => session.move_down(),
                KeyCode::Enter | KeyCode::Right => session.dot()?,
                KeyCode::Esc => session.finish_scan(),
                _ => {}
            },
            Mode::Action { .. } => match key.code {
                KeyCode::Char(' ') => session.complete()?,
                // Resume scanning to dot more candidates below the current task.
                KeyCode::Char('s') => session.resume_scan(),
                _ => {}
            },
            Mode::Empty => {}
        }
        Ok(())
    }

    /// Keys while the add-task input overlay is open. Add mode is "sticky":
    /// Enter commits the current entry and clears the buffer for the next one;
    /// Esc leaves add mode and returns to the previous mode.
    fn on_input_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => self.input = None,
            KeyCode::Enter => {
                let text = self.input.as_deref().unwrap_or_default().trim().to_string();
                if !text.is_empty() {
                    self.session().add(text)?;
                }
                // Stay in add mode with a cleared buffer to enter another task.
                self.input = Some(String::new());
            }
            KeyCode::Backspace => {
                if let Some(buf) = self.input.as_mut() {
                    buf.pop();
                }
            }
            KeyCode::Char(c) => {
                if let Some(buf) = self.input.as_mut() {
                    buf.push(c);
                }
            }
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::Task;
    use crossterm::event::{KeyCode, KeyModifiers};
    use std::path::Path;

    fn app_with(dir: &Path, tasks: Vec<Task>) -> App {
        let session = Session::new(dir.join("tasks.txt"), tasks);
        App::new(Arc::new(Mutex::new(session)))
    }

    fn mode(app: &App) -> Mode {
        app.session().mode
    }

    fn press(app: &mut App, code: KeyCode) {
        app.on_key(KeyEvent::new(code, KeyModifiers::NONE)).unwrap();
    }

    fn type_text(app: &mut App, text: &str) {
        for c in text.chars() {
            press(app, KeyCode::Char(c));
        }
    }

    /// Enter the add overlay, type `text`, commit it, and leave add mode.
    fn add(app: &mut App, text: &str) {
        press(app, KeyCode::Char('a'));
        type_text(app, text);
        press(app, KeyCode::Enter);
        press(app, KeyCode::Esc); // exit sticky add mode
    }

    #[test]
    fn adding_to_empty_list_starts_a_scan_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with(dir.path(), vec![]);
        assert_eq!(mode(&app), Mode::Empty);

        add(&mut app, "first task");
        // Single active task -> dotted, and it is the action task.
        assert_eq!(mode(&app), Mode::Action { task: 0 });
        assert!(app.session().tasks[0].is_dotted());
        // Persisted with the dotted marker.
        assert_eq!(
            std::fs::read_to_string(dir.path().join("tasks.txt")).unwrap(),
            "[.] first task\n"
        );
    }

    #[test]
    fn empty_input_is_not_added() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with(dir.path(), vec![]);
        press(&mut app, KeyCode::Char('a'));
        type_text(&mut app, "   ");
        press(&mut app, KeyCode::Enter);
        assert!(app.session().tasks.is_empty());
        assert_eq!(mode(&app), Mode::Empty);
    }

    #[test]
    fn add_mode_is_sticky_until_esc() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with(dir.path(), vec![]);

        press(&mut app, KeyCode::Char('a'));
        type_text(&mut app, "one");
        press(&mut app, KeyCode::Enter);
        // Still in add mode with a cleared buffer, ready for the next task.
        assert_eq!(app.input.as_deref(), Some(""));
        assert_eq!(app.session().tasks.len(), 1);

        type_text(&mut app, "two");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.session().tasks.len(), 2);
        assert_eq!(app.input.as_deref(), Some(""));

        // Esc leaves add mode.
        press(&mut app, KeyCode::Esc);
        assert!(app.input.is_none());
        assert_eq!(app.session().tasks[0].text, "one");
        assert_eq!(app.session().tasks[1].text, "two");
    }

    #[test]
    fn full_cycle_persists_and_survives_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tasks.txt");
        let mut app = app_with(dir.path(), vec![]);

        add(&mut app, "A");
        add(&mut app, "B");
        add(&mut app, "C");
        // A got dotted on first add; B and C are open. Action is A.
        assert_eq!(mode(&app), Mode::Action { task: 0 });

        // Complete A -> only dot removed -> fresh scan dots B, cursor at C.
        press(&mut app, KeyCode::Char(' '));
        assert_eq!(
            mode(&app),
            Mode::Preselect {
                benchmark: 1,
                cursor: 2
            }
        );

        // Dot C -> it becomes the last dotted and the action task.
        press(&mut app, KeyCode::Enter);
        assert_eq!(mode(&app), Mode::Action { task: 2 });
        assert!(app.session().tasks[2].is_dotted());

        // File reflects: A done, B dotted, C dotted.
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "[x] A\n[.] B\n[.] C\n"
        );

        // Reloading resumes on the last dotted task (C).
        let reloaded = Session::load(path).unwrap();
        assert_eq!(reloaded.mode, Mode::Action { task: 2 });
    }

    #[test]
    fn s_key_resumes_scan_from_action() {
        let dir = tempfile::tempdir().unwrap();
        // Three open tasks: initial scan dots A, offers B.
        let mut app = app_with(
            dir.path(),
            vec![Task::new("A"), Task::new("B"), Task::new("C")],
        );
        // Esc out of the scan to land in Action on A.
        press(&mut app, KeyCode::Esc);
        assert_eq!(mode(&app), Mode::Action { task: 0 });
        // 's' re-opens scanning below A.
        press(&mut app, KeyCode::Char('s'));
        assert_eq!(
            mode(&app),
            Mode::Preselect {
                benchmark: 0,
                cursor: 1
            }
        );
    }

    #[test]
    fn esc_finishes_scan_to_benchmark() {
        let dir = tempfile::tempdir().unwrap();
        // Two open tasks; initial scan dots A and offers B as candidate.
        let mut app = app_with(dir.path(), vec![Task::new("A"), Task::new("B")]);
        assert_eq!(
            mode(&app),
            Mode::Preselect {
                benchmark: 0,
                cursor: 1
            }
        );
        press(&mut app, KeyCode::Esc);
        assert_eq!(mode(&app), Mode::Action { task: 0 });
    }
}
