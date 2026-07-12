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
    /// `Some` while the user is typing in the input overlay (add or edit).
    pub input: Option<String>,
    /// `Some(i)` while the input overlay is editing task `i`; `None` = adding.
    pub input_target: Option<usize>,
    /// `Some(cursor)` while browse mode is active (free cursor over all tasks).
    pub browse: Option<usize>,
    pub show_help: bool,
    pub should_quit: bool,
}

impl App {
    pub fn new(session: Arc<Mutex<Session>>) -> Self {
        App {
            session,
            input: None,
            input_target: None,
            browse: None,
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
            KeyCode::Char('a') | KeyCode::Char('A') => {
                self.input = Some(String::new());
                self.input_target = None;
            }
            // Back up the file, then drop done tasks from the list.
            KeyCode::Char('p') => {
                self.session().purge_done()?;
                self.clamp_browse_cursor();
            }
            _ if self.browse.is_some() => self.on_browse_key(key)?,
            _ => self.on_mode_key(key)?,
        }
        Ok(())
    }

    /// Enter browse mode with the cursor on the action task (or the top).
    fn enter_browse(&mut self) {
        let session = self.session();
        let len = session.tasks.len();
        let cursor = session
            .action_task()
            .unwrap_or(0)
            .min(len.saturating_sub(1));
        drop(session);
        self.browse = Some(cursor);
    }

    /// Keep the browse cursor in bounds after the list shrinks (e.g. purge).
    fn clamp_browse_cursor(&mut self) {
        if let Some(cursor) = self.browse {
            let len = self.session().tasks.len();
            self.browse = Some(cursor.min(len.saturating_sub(1)));
        }
    }

    /// Keys while browsing the full list with a free cursor.
    fn on_browse_key(&mut self, key: KeyEvent) -> Result<()> {
        let Some(cursor) = self.browse else {
            return Ok(());
        };
        let mut session = self.session.lock().expect("session lock poisoned");
        let len = session.tasks.len();
        match key.code {
            KeyCode::Up => {
                self.browse = Some(cursor.saturating_sub(1));
            }
            KeyCode::Down => {
                if cursor + 1 < len {
                    self.browse = Some(cursor + 1);
                }
            }
            KeyCode::Char(' ') => session.toggle_done_at(cursor)?,
            KeyCode::Char('.') => session.toggle_dot_at(cursor)?,
            KeyCode::Char('e') | KeyCode::Enter => {
                if let Some(task) = session.tasks.get(cursor) {
                    self.input = Some(task.text.clone());
                    self.input_target = Some(cursor);
                }
            }
            // Back out to Do (action) mode.
            KeyCode::Esc => self.browse = None,
            // Dive back into scanning.
            KeyCode::Char('s') => {
                self.browse = None;
                session.resume_scan();
            }
            _ => {}
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
                KeyCode::Right => session.dot()?,
                KeyCode::Left => session.undot()?,
                KeyCode::Esc => session.finish_scan(),
                _ => {}
            },
            Mode::Action { .. } => match key.code {
                KeyCode::Char(' ') => session.complete()?,
                // Resume scanning to dot more candidates below the current task.
                KeyCode::Char('s') => session.resume_scan(),
                // Zoom out to browse the full list.
                KeyCode::Esc => {
                    drop(session);
                    self.enter_browse();
                }
                _ => {}
            },
            Mode::Empty => {
                if key.code == KeyCode::Esc {
                    drop(session);
                    self.enter_browse();
                }
            }
        }
        Ok(())
    }

    /// Keys while the input overlay is open. Adding is "sticky": Enter commits
    /// the entry and clears the buffer for the next one; Esc leaves. Editing
    /// (`input_target` set) saves on Enter and closes immediately.
    fn on_input_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                self.input = None;
                self.input_target = None;
            }
            KeyCode::Enter => {
                let text = self.input.as_deref().unwrap_or_default().trim().to_string();
                if let Some(i) = self.input_target.take() {
                    self.input = None;
                    // An emptied buffer cancels rather than blanking the task.
                    if !text.is_empty() {
                        self.session().edit_text_at(i, text)?;
                    }
                    return Ok(());
                }
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
        press(&mut app, KeyCode::Right);
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
    fn esc_zooms_out_scan_to_do_to_browse_and_back_in() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with(
            dir.path(),
            vec![Task::new("A"), Task::new("B"), Task::new("C")],
        );
        // Scan in progress.
        assert!(matches!(mode(&app), Mode::Preselect { .. }));
        // Esc: Scan -> Do.
        press(&mut app, KeyCode::Esc);
        assert_eq!(mode(&app), Mode::Action { task: 0 });
        assert!(app.browse.is_none());
        // Esc: Do -> Browse, cursor on the action task.
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.browse, Some(0));
        // Esc: Browse -> Do.
        press(&mut app, KeyCode::Esc);
        assert!(app.browse.is_none());
        assert_eq!(mode(&app), Mode::Action { task: 0 });
    }

    #[test]
    fn left_undoes_the_last_dot_during_scan() {
        let dir = tempfile::tempdir().unwrap();
        // Initial scan dots A: Preselect { benchmark: 0, cursor: 1 }.
        let mut app = app_with(
            dir.path(),
            vec![Task::new("A"), Task::new("B"), Task::new("C")],
        );
        press(&mut app, KeyCode::Right); // dot B -> benchmark 1, cursor 2
        assert!(app.session().tasks[1].is_dotted());
        press(&mut app, KeyCode::Left); // change of mind: un-dot B
        assert!(app.session().tasks[1].is_open());
        assert_eq!(
            mode(&app),
            Mode::Preselect {
                benchmark: 0,
                cursor: 1
            }
        );
        // The undo persisted.
        assert_eq!(
            std::fs::read_to_string(dir.path().join("tasks.txt")).unwrap(),
            "[.] A\n[ ] B\n[ ] C\n"
        );
        // The first (automatic) dot can't be undone.
        press(&mut app, KeyCode::Left);
        assert!(app.session().tasks[0].is_dotted());
    }

    #[test]
    fn enter_no_longer_dots_during_scan() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with(dir.path(), vec![Task::new("A"), Task::new("B")]);
        press(&mut app, KeyCode::Enter);
        assert!(app.session().tasks[1].is_open());
        assert_eq!(
            mode(&app),
            Mode::Preselect {
                benchmark: 0,
                cursor: 1
            }
        );
    }

    #[test]
    fn enter_in_browse_opens_edit() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with(dir.path(), vec![Task::new("A"), Task::new("B")]);
        press(&mut app, KeyCode::Esc); // -> Do
        press(&mut app, KeyCode::Esc); // -> Browse at 0
        press(&mut app, KeyCode::Enter); // Enter edits, same as `e`
        assert_eq!(app.input.as_deref(), Some("A"));
        assert_eq!(app.input_target, Some(0));
        // Still in browse behind the overlay; Esc cancels the edit only.
        press(&mut app, KeyCode::Esc);
        assert!(app.input.is_none());
        assert_eq!(app.browse, Some(0));
    }

    #[test]
    fn browse_navigates_and_toggles_done_and_dot() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with(
            dir.path(),
            vec![Task::new("A"), Task::new("B"), Task::new("C")],
        );
        press(&mut app, KeyCode::Esc); // -> Do
        press(&mut app, KeyCode::Esc); // -> Browse at 0
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Down);
        assert_eq!(app.browse, Some(2));
        press(&mut app, KeyCode::Down); // bounded
        assert_eq!(app.browse, Some(2));

        // Complete C directly (not the action task).
        press(&mut app, KeyCode::Char(' '));
        assert!(!app.session().tasks[2].is_active());
        // Un-complete it.
        press(&mut app, KeyCode::Char(' '));
        assert!(app.session().tasks[2].is_open());

        // Manually dot B: it becomes the last dotted -> action task.
        press(&mut app, KeyCode::Up);
        press(&mut app, KeyCode::Char('.'));
        assert!(app.session().tasks[1].is_dotted());
        assert_eq!(mode(&app), Mode::Action { task: 1 });
        // Still browsing (mode change doesn't kick us out).
        assert_eq!(app.browse, Some(1));
    }

    #[test]
    fn browse_edit_prefills_saves_and_is_not_sticky() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with(dir.path(), vec![Task::new("old"), Task::new("B")]);
        press(&mut app, KeyCode::Esc); // -> Do
        press(&mut app, KeyCode::Esc); // -> Browse at 0
        press(&mut app, KeyCode::Char('e'));
        assert_eq!(app.input.as_deref(), Some("old"));
        assert_eq!(app.input_target, Some(0));

        // Append " text" and save.
        type_text(&mut app, " text");
        press(&mut app, KeyCode::Enter);
        // Overlay closed (not sticky), still browsing, text saved, status kept.
        assert!(app.input.is_none());
        assert!(app.input_target.is_none());
        assert_eq!(app.browse, Some(0));
        assert_eq!(app.session().tasks[0].text, "old text");
        assert!(app.session().tasks[0].is_dotted());
    }

    #[test]
    fn s_from_browse_resumes_scan() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with(dir.path(), vec![Task::new("A"), Task::new("B")]);
        press(&mut app, KeyCode::Esc); // -> Do (Action on A)
        press(&mut app, KeyCode::Esc); // -> Browse
        press(&mut app, KeyCode::Char('s'));
        assert!(app.browse.is_none());
        assert_eq!(
            mode(&app),
            Mode::Preselect {
                benchmark: 0,
                cursor: 1
            }
        );
    }

    #[test]
    fn down_is_bounded_during_scan() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with(dir.path(), vec![Task::new("A"), Task::new("B")]);
        // Preselect { benchmark: 0, cursor: 1 } — B is the last candidate.
        press(&mut app, KeyCode::Down);
        assert_eq!(
            mode(&app),
            Mode::Preselect {
                benchmark: 0,
                cursor: 1
            }
        );
    }

    #[test]
    fn esc_from_empty_mode_enters_browse_over_history() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with(dir.path(), vec![Task::new("only")]);
        press(&mut app, KeyCode::Char(' ')); // complete the only task -> Empty
        assert_eq!(mode(&app), Mode::Empty);
        press(&mut app, KeyCode::Esc); // -> Browse over the done history
        assert_eq!(app.browse, Some(0));
        // Un-complete it from browse; mode re-derives to a fresh scan.
        press(&mut app, KeyCode::Char(' '));
        assert!(app.session().tasks[0].is_dotted());
        assert_eq!(mode(&app), Mode::Action { task: 0 });
    }

    #[test]
    fn p_key_purges_done_tasks() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with(dir.path(), vec![Task::new("A"), Task::new("B")]);
        // Finish the scan on A and complete it (B remains, freshly dotted).
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Char(' '));
        assert_eq!(app.session().tasks.len(), 2);

        press(&mut app, KeyCode::Char('p'));
        let session = app.session();
        assert_eq!(session.tasks.len(), 1);
        assert_eq!(session.tasks[0].text, "B");
        assert_eq!(session.mode, Mode::Action { task: 0 });
        drop(session);
        // Backup captured the pre-purge state.
        let backup = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .find(|e| e.file_name().to_string_lossy().starts_with("tasks.txt-"))
            .expect("backup created");
        assert!(
            std::fs::read_to_string(backup.path())
                .unwrap()
                .contains("[x] A")
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
