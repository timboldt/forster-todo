//! The shared application core: the task list and FVP mode, independent of any
//! presentation layer. Both the TUI and the web server hold this behind an
//! `Arc<Mutex<Session>>` and apply mutations through the same [`fvp`] state
//! machine, so the two views can never diverge.

use std::path::PathBuf;

use anyhow::Result;

use crate::fvp::{self, Mode};
use crate::storage;
use crate::task::{Status, Task};

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

    /// Undo the most recent dot during a scan (no-op on the only dot).
    pub fn undot(&mut self) -> Result<()> {
        self.mode = fvp::undot(&mut self.tasks, self.mode);
        self.bump();
        self.save()
    }

    /// Complete the action ("DO NOW") task. Stays in Action mode on the next
    /// dotted task when one remains. No-op unless in Action mode.
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

    // --- Direct manipulation (browse mode) ---
    //
    // These act on an arbitrary index. Status changes abandon any scan in
    // progress and re-derive the mode from the dots ("the dots define the
    // state" — the same rule as an app restart).

    /// Toggle done/undone on the task at `i`: done -> open, open/dotted -> done.
    pub fn toggle_done_at(&mut self, i: usize) -> Result<()> {
        let Some(task) = self.tasks.get_mut(i) else {
            return Ok(());
        };
        task.status = match task.status {
            Status::Done => Status::Open,
            Status::Open | Status::Dotted => Status::Done,
        };
        self.rederive_mode();
        self.bump();
        self.save()
    }

    /// Toggle the dot on the task at `i`: open <-> dotted. No-op on done tasks.
    pub fn toggle_dot_at(&mut self, i: usize) -> Result<()> {
        let Some(task) = self.tasks.get_mut(i) else {
            return Ok(());
        };
        task.status = match task.status {
            Status::Open => Status::Dotted,
            Status::Dotted => Status::Open,
            Status::Done => return Ok(()),
        };
        self.rederive_mode();
        self.bump();
        self.save()
    }

    /// Replace the text of the task at `i`. Status and mode are unaffected.
    pub fn edit_text_at(&mut self, i: usize, text: impl Into<String>) -> Result<()> {
        let Some(task) = self.tasks.get_mut(i) else {
            return Ok(());
        };
        task.text = text.into();
        self.bump();
        self.save()
    }

    /// Re-derive the mode from the dots (Action on the last dotted task, else a
    /// fresh scan, else Empty) after a direct status change.
    fn rederive_mode(&mut self) {
        self.mode = fvp::initial_mode(&mut self.tasks);
    }

    /// Back up the task file to a dated copy, then remove all done tasks.
    /// Returns the number purged; when nothing is done this is a no-op (no
    /// backup is written). Mode indices are remapped to the compacted list.
    pub fn purge_done(&mut self) -> Result<usize> {
        let purged = self.tasks.iter().filter(|t| !t.is_active()).count();
        if purged == 0 {
            return Ok(0);
        }
        storage::backup(&self.path)?;

        // New index of each retained task (None for the removed ones).
        let mut new_index = vec![None; self.tasks.len()];
        let mut next = 0;
        for (i, t) in self.tasks.iter().enumerate() {
            if t.is_active() {
                new_index[i] = Some(next);
                next += 1;
            }
        }
        self.tasks.retain(Task::is_active);

        // The mode only ever points at active tasks, so remapping always
        // succeeds; fall back to a fresh initial mode defensively.
        let remapped = match self.mode {
            Mode::Empty => Some(Mode::Empty),
            Mode::Action { task } => new_index[task].map(|task| Mode::Action { task }),
            Mode::Preselect { benchmark, cursor } => new_index[benchmark]
                .zip(new_index[cursor])
                .map(|(benchmark, cursor)| Mode::Preselect { benchmark, cursor }),
        };
        self.mode = remapped.unwrap_or_else(|| fvp::initial_mode(&mut self.tasks));

        self.bump();
        self.save()?;
        Ok(purged)
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
    fn toggle_done_completes_arbitrary_task_and_rederives_mode() {
        let (_dir, mut s) = session(&["a", "b", "c"]);
        // Initial scan dots a: Preselect { benchmark: 0, cursor: 1 }.
        // Complete b (not the action task, mid-scan) directly.
        s.toggle_done_at(1).unwrap();
        assert_eq!(s.tasks[1].status, Status::Done);
        // Scan abandoned; mode re-derived from dots: Action on a.
        assert_eq!(s.mode, Mode::Action { task: 0 });

        // Un-complete it: b back to open; dots unchanged -> still Action on a.
        s.toggle_done_at(1).unwrap();
        assert_eq!(s.tasks[1].status, Status::Open);
        assert_eq!(s.mode, Mode::Action { task: 0 });
    }

    #[test]
    fn toggle_done_on_last_dot_starts_fresh_scan() {
        let (_dir, mut s) = session(&["a", "b"]);
        // a is dotted (initial scan). Complete a directly.
        s.toggle_done_at(0).unwrap();
        // No dots left -> fresh scan dots b (only active task) -> Action.
        assert!(s.tasks[1].is_dotted());
        assert_eq!(s.mode, Mode::Action { task: 1 });
    }

    #[test]
    fn toggle_dot_flips_open_and_dotted_but_not_done() {
        let (_dir, mut s) = session(&["a", "b", "c"]);
        // Dot b manually -> it becomes the last dotted -> Action on b.
        s.toggle_dot_at(1).unwrap();
        assert!(s.tasks[1].is_dotted());
        assert_eq!(s.mode, Mode::Action { task: 1 });
        // Un-dot b -> a is the only dot again.
        s.toggle_dot_at(1).unwrap();
        assert!(s.tasks[1].is_open());
        assert_eq!(s.mode, Mode::Action { task: 0 });

        // Done tasks are untouched by dot toggling.
        s.toggle_done_at(2).unwrap();
        let v = s.version();
        s.toggle_dot_at(2).unwrap();
        assert_eq!(s.tasks[2].status, Status::Done);
        assert_eq!(s.version(), v); // true no-op
    }

    #[test]
    fn edit_text_preserves_status_and_persists() {
        let (dir, mut s) = session(&["old text", "b"]);
        // a is dotted from the initial scan.
        let mode_before = s.mode;
        s.edit_text_at(0, "new text").unwrap();
        assert_eq!(s.tasks[0].text, "new text");
        assert!(s.tasks[0].is_dotted());
        assert_eq!(s.mode, mode_before);
        let live = std::fs::read_to_string(dir.path().join("tasks.txt")).unwrap();
        assert!(live.contains("[.] new text"));
    }

    #[test]
    fn direct_ops_out_of_range_are_noops() {
        let (_dir, mut s) = session(&["a"]);
        let v = s.version();
        s.toggle_done_at(9).unwrap();
        s.toggle_dot_at(9).unwrap();
        s.edit_text_at(9, "x").unwrap();
        assert_eq!(s.version(), v);
        assert_eq!(s.tasks.len(), 1);
    }

    #[test]
    fn purge_removes_done_backs_up_and_remaps_scan_indices() {
        let (dir, mut s) = session(&["a", "b", "c", "d"]);
        // Build state: dot a, dot c, then complete c -> Action on a.
        // Initial: Preselect { benchmark: 0(a), cursor: 1(b) }.
        s.move_down(); // cursor -> 2(c)
        s.dot().unwrap(); // dot c -> benchmark 2, cursor 3(d)
        s.finish_scan(); // Action { task: 2 }
        s.complete().unwrap(); // c done -> Action { task: 0 }
        s.resume_scan(); // Preselect { benchmark: 0, cursor: 1(b) }
        assert_eq!(
            s.mode,
            Mode::Preselect {
                benchmark: 0,
                cursor: 1
            }
        );

        let purged = s.purge_done().unwrap();
        assert_eq!(purged, 1); // c removed
        assert_eq!(
            s.tasks.iter().map(|t| t.text.as_str()).collect::<Vec<_>>(),
            vec!["a", "b", "d"]
        );
        // Indices before c were unaffected; the scan state survives intact.
        assert_eq!(
            s.mode,
            Mode::Preselect {
                benchmark: 0,
                cursor: 1
            }
        );
        // A dated backup exists next to the file and preserves the done task.
        let backup = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .find(|e| e.file_name().to_string_lossy().starts_with("tasks.txt-"))
            .expect("backup file created");
        assert!(
            std::fs::read_to_string(backup.path())
                .unwrap()
                .contains("[x] c")
        );
        // The live file no longer has c.
        let live = std::fs::read_to_string(dir.path().join("tasks.txt")).unwrap();
        assert!(!live.contains('c'));
    }

    #[test]
    fn purge_remaps_action_index_past_removed_tasks() {
        let (_dir, mut s) = session(&["a", "b"]);
        // Dot both; complete a's chain partner first. Initial Preselect{0,1}.
        s.dot().unwrap(); // dot b -> Action { task: 1 }
        // Complete b -> Action { task: 0 }; b done at index 1.
        s.complete().unwrap();
        // Manually check: mode Action{0}, b done. Now make done task come FIRST
        // in file order by completing a and rescanning... simpler: purge now
        // (done task is after the action task -> index unchanged).
        assert_eq!(s.purge_done().unwrap(), 1);
        assert_eq!(s.mode, Mode::Action { task: 0 });
        assert_eq!(s.tasks.len(), 1);

        // Now the done task precedes the survivor: add c, complete a.
        s.add("c").unwrap(); // Action stays on a; c open at index 1
        s.complete().unwrap(); // a done -> fresh scan? a was only dot -> start_scan dots c
        assert_eq!(s.mode, Mode::Action { task: 1 }); // c, after done a
        assert_eq!(s.purge_done().unwrap(), 1);
        // c shifted from index 1 to 0 and the mode followed it.
        assert_eq!(s.mode, Mode::Action { task: 0 });
        assert_eq!(s.tasks[0].text, "c");
    }

    #[test]
    fn purge_with_nothing_done_is_a_noop_without_backup() {
        let (dir, mut s) = session(&["a", "b"]);
        let v = s.version();
        assert_eq!(s.purge_done().unwrap(), 0);
        assert_eq!(s.version(), v);
        // No backup file appeared.
        let backups = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("tasks.txt-"))
            .count();
        assert_eq!(backups, 0);
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
