//! Application state and key handling.

use std::path::PathBuf;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};

use crate::fvp::{self, Mode};
use crate::storage;
use crate::task::Task;

/// The full application state.
pub struct App {
    pub tasks: Vec<Task>,
    pub mode: Mode,
    /// `Some` while the user is typing a new task in the add overlay.
    pub input: Option<String>,
    pub show_help: bool,
    pub should_quit: bool,
    path: PathBuf,
}

impl App {
    /// Build the app from a loaded task list, establishing the initial mode.
    pub fn new(path: PathBuf, mut tasks: Vec<Task>) -> Self {
        let mode = fvp::initial_mode(&mut tasks);
        App {
            tasks,
            mode,
            input: None,
            show_help: false,
            should_quit: false,
            path,
        }
    }

    /// Persist the current task list to disk.
    pub fn save(&self) -> Result<()> {
        storage::save(&self.path, &self.tasks)
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
                self.save()?;
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
        match self.mode {
            Mode::Preselect { .. } => match key.code {
                KeyCode::Up => self.mode = fvp::move_up(&self.tasks, self.mode),
                KeyCode::Down => self.mode = fvp::move_down(&self.tasks, self.mode),
                KeyCode::Enter | KeyCode::Right => {
                    self.mode = fvp::dot(&mut self.tasks, self.mode);
                    self.save()?;
                }
                KeyCode::Esc => self.mode = fvp::finish_scan(self.mode),
                _ => {}
            },
            Mode::Action { .. } => {
                if key.code == KeyCode::Char(' ') {
                    self.mode = fvp::complete(&mut self.tasks, self.mode);
                    self.save()?;
                }
            }
            Mode::Empty => {}
        }
        Ok(())
    }

    /// Keys while the add-task input overlay is open.
    fn on_input_key(&mut self, key: KeyEvent) -> Result<()> {
        let Some(buf) = self.input.as_mut() else {
            return Ok(());
        };
        match key.code {
            KeyCode::Esc => self.input = None,
            KeyCode::Enter => {
                let text = buf.trim().to_string();
                self.input = None;
                if !text.is_empty() {
                    self.add_task(text)?;
                }
            }
            KeyCode::Backspace => {
                buf.pop();
            }
            KeyCode::Char(c) => buf.push(c),
            _ => {}
        }
        Ok(())
    }

    /// Append a task and, if the list was empty, begin a scan.
    fn add_task(&mut self, text: String) -> Result<()> {
        self.tasks.push(Task::new(text));
        if self.mode == Mode::Empty {
            self.mode = fvp::start_scan(&mut self.tasks);
        }
        self.save()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};
    use std::path::Path;

    fn press(app: &mut App, code: KeyCode) {
        app.on_key(KeyEvent::new(code, KeyModifiers::NONE)).unwrap();
    }

    fn type_text(app: &mut App, text: &str) {
        for c in text.chars() {
            press(app, KeyCode::Char(c));
        }
    }

    /// Enter the add overlay, type `text`, and commit it.
    fn add(app: &mut App, text: &str) {
        press(app, KeyCode::Char('a'));
        type_text(app, text);
        press(app, KeyCode::Enter);
    }

    fn reload(path: &Path) -> App {
        App::new(path.to_path_buf(), crate::storage::load(path).unwrap())
    }

    #[test]
    fn adding_to_empty_list_starts_a_scan_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tasks.txt");
        let mut app = App::new(path.clone(), vec![]);
        assert_eq!(app.mode, Mode::Empty);

        add(&mut app, "first task");
        // Single active task -> dotted, and it is the action task.
        assert_eq!(app.mode, Mode::Action { task: 0 });
        assert!(app.tasks[0].is_dotted());
        // Persisted with the dotted marker.
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[.] first task\n");
    }

    #[test]
    fn empty_input_is_not_added() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::new(dir.path().join("tasks.txt"), vec![]);
        press(&mut app, KeyCode::Char('a'));
        type_text(&mut app, "   ");
        press(&mut app, KeyCode::Enter);
        assert!(app.tasks.is_empty());
        assert_eq!(app.mode, Mode::Empty);
    }

    #[test]
    fn full_cycle_persists_and_survives_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tasks.txt");
        let mut app = App::new(path.clone(), vec![]);

        add(&mut app, "A");
        add(&mut app, "B");
        add(&mut app, "C");
        // A got dotted on first add; B and C are open. Action is A.
        assert_eq!(app.mode, Mode::Action { task: 0 });

        // Complete A -> only dot removed -> fresh scan dots B, cursor at C.
        press(&mut app, KeyCode::Char(' '));
        assert_eq!(
            app.mode,
            Mode::Preselect {
                benchmark: 1,
                cursor: 2
            }
        );

        // Dot C -> it becomes the last dotted and the action task.
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.mode, Mode::Action { task: 2 });
        assert!(app.tasks[2].is_dotted());

        // File reflects: A done, B dotted, C dotted.
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "[x] A\n[.] B\n[.] C\n"
        );

        // Reloading resumes on the last dotted task (C).
        let reloaded = reload(&path);
        assert_eq!(reloaded.mode, Mode::Action { task: 2 });
    }

    #[test]
    fn esc_finishes_scan_to_benchmark() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tasks.txt");
        // Two open tasks; initial scan dots A and offers B as candidate.
        let mut app = App::new(path, vec![Task::new("A"), Task::new("B")]);
        assert_eq!(
            app.mode,
            Mode::Preselect {
                benchmark: 0,
                cursor: 1
            }
        );
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.mode, Mode::Action { task: 0 });
    }
}
