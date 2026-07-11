//! The shared application core: the task list and FVP mode, independent of any
//! presentation layer. Both the TUI and the web server hold this behind an
//! `Arc<Mutex<Session>>` and apply mutations through the same [`fvp`] state
//! machine, so the two views can never diverge.

use std::path::PathBuf;

use anyhow::Result;

use crate::fvp::{self, Mode};
use crate::storage;
use crate::task::Task;

pub struct Session {
    pub tasks: Vec<Task>,
    pub mode: Mode,
    /// Monotonic change counter; bumped on every mutation so pollers (the web
    /// UI) can cheaply detect "anything changed?" without diffing.
    version: u64,
    path: PathBuf,
}

impl Session {
    /// Build a session from an already-loaded task list.
    pub fn new(path: PathBuf, mut tasks: Vec<Task>) -> Self {
        let mode = fvp::initial_mode(&mut tasks);
        Session {
            tasks,
            mode,
            version: 0,
            path,
        }
    }

    /// Load the task file at `path` (missing file = empty list).
    pub fn load(path: PathBuf) -> Result<Self> {
        let tasks = storage::load(&path)?;
        Ok(Self::new(path, tasks))
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    /// Persist the current task list to disk.
    pub fn save(&self) -> Result<()> {
        storage::save(&self.path, &self.tasks)
    }

    fn bump(&mut self) {
        self.version = self.version.wrapping_add(1);
    }

    // --- Scan navigation (mode-only; nothing to persist) ---

    pub fn move_up(&mut self) {
        self.mode = fvp::move_up(&self.tasks, self.mode);
        self.bump();
    }

    pub fn move_down(&mut self) {
        self.mode = fvp::move_down(&self.tasks, self.mode);
        self.bump();
    }

    pub fn finish_scan(&mut self) {
        self.mode = fvp::finish_scan(self.mode);
        self.bump();
    }

    pub fn resume_scan(&mut self) {
        self.mode = fvp::resume_scan(&self.tasks, self.mode);
        self.bump();
    }

    // --- Mutations (persisted immediately) ---

    /// Dot the current scan candidate.
    pub fn dot(&mut self) -> Result<()> {
        self.mode = fvp::dot(&mut self.tasks, self.mode);
        self.bump();
        self.save()
    }

    /// Complete the action ("DO NOW") task. No-op unless in Action mode.
    pub fn complete(&mut self) -> Result<()> {
        self.mode = fvp::complete(&mut self.tasks, self.mode);
        self.bump();
        self.save()
    }

    /// Append a new open task; if the list was empty, begin a scan.
    pub fn add(&mut self, text: impl Into<String>) -> Result<()> {
        self.tasks.push(Task::new(text));
        if self.mode == Mode::Empty {
            self.mode = fvp::start_scan(&mut self.tasks);
        }
        self.bump();
        self.save()
    }

    /// The "DO NOW" task, if the scan is finished.
    pub fn action_task(&self) -> Option<usize> {
        match self.mode {
            Mode::Action { task } => Some(task),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(names: &[&str]) -> (tempfile::TempDir, Session) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tasks.txt");
        let tasks = names.iter().map(|n| Task::new(*n)).collect();
        (dir, Session::new(path, tasks))
    }

    #[test]
    fn mutations_bump_version_and_persist() {
        let (_dir, mut s) = session(&[]);
        let v0 = s.version();
        s.add("first").unwrap();
        assert!(s.version() > v0);
        // Single task -> auto-dotted by the initial scan.
        assert_eq!(s.mode, Mode::Action { task: 0 });
        // Reload from disk: state round-trips.
        let reloaded = Session::load(s.path.clone()).unwrap();
        assert_eq!(reloaded.tasks, s.tasks);
        assert_eq!(reloaded.mode, Mode::Action { task: 0 });
    }

    #[test]
    fn complete_is_a_noop_outside_action_mode() {
        let (_dir, mut s) = session(&["a", "b"]);
        // Initial scan: Preselect { benchmark: 0, cursor: 1 }.
        let before = s.mode;
        s.complete().unwrap();
        assert_eq!(s.mode, before);
        assert!(s.tasks.iter().all(|t| t.is_active()));
    }

    #[test]
    fn action_task_reports_do_now() {
        let (_dir, mut s) = session(&["a"]);
        assert_eq!(s.action_task(), Some(0));
        s.complete().unwrap();
        assert_eq!(s.action_task(), None);
        assert_eq!(s.mode, Mode::Empty);
    }
}
