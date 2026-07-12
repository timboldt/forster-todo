//! Pure FVP (Final Version Perfected) state machine.
//!
//! This module contains no I/O and no terminal code so the algorithm can be
//! unit-tested in isolation. It operates on a `&mut [Task]` and produces the
//! current [`Mode`].
//!
//! ## The algorithm
//!
//! FVP keeps one ordered list. A *scan* dots (pre-selects) a chain of tasks by
//! asking, for each candidate top-to-bottom, "do I want to do this more than the
//! last dotted task (the *benchmark*)?". The **last dotted** task is always the
//! one worked next. When it is completed you re-scan from its position to the end
//! of the list and continue.

use crate::task::{Status, Task};

/// The current position in the FVP cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// No active tasks remain.
    Empty,
    /// A scan is in progress. `benchmark` is the last dotted task; `cursor` is
    /// the candidate currently being compared against it.
    Preselect { benchmark: usize, cursor: usize },
    /// The scan is complete; `task` is the last dotted task, to be worked now.
    Action { task: usize },
}

/// First open, un-dotted candidate strictly after `after`.
fn next_candidate(tasks: &[Task], after: usize) -> Option<usize> {
    tasks
        .iter()
        .enumerate()
        .skip(after + 1)
        .find(|(_, t)| t.is_open())
        .map(|(i, _)| i)
}

/// Last open candidate strictly between `floor` and `before` (both exclusive).
/// Used for non-destructive upward navigation during a scan.
fn prev_candidate(tasks: &[Task], before: usize, floor: usize) -> Option<usize> {
    (floor + 1..before).rev().find(|&i| tasks[i].is_open())
}

/// Index of the last (bottom-most) dotted task, if any.
fn last_dotted(tasks: &[Task]) -> Option<usize> {
    tasks
        .iter()
        .enumerate()
        .rev()
        .find(|(_, t)| t.is_dotted())
        .map(|(i, _)| i)
}

/// Index of the first active (not done) task, if any.
fn first_active(tasks: &[Task]) -> Option<usize> {
    tasks.iter().position(Task::is_active)
}

/// Begin a fresh scan: dot the first active task and position the cursor at the
/// first candidate below it. Returns [`Mode::Empty`] when nothing is active.
pub fn start_scan(tasks: &mut [Task]) -> Mode {
    match first_active(tasks) {
        None => Mode::Empty,
        Some(b) => {
            tasks[b].status = Status::Dotted;
            match next_candidate(tasks, b) {
                Some(c) => Mode::Preselect {
                    benchmark: b,
                    cursor: c,
                },
                None => Mode::Action { task: b },
            }
        }
    }
}

/// Establish the mode when (re)loading a list: resume the last dotted task if
/// any dots exist, otherwise start a fresh scan (or report empty).
pub fn initial_mode(tasks: &mut [Task]) -> Mode {
    if let Some(t) = last_dotted(tasks) {
        Mode::Action { task: t }
    } else {
        start_scan(tasks)
    }
}

/// Dot the current candidate: it becomes the new benchmark and the cursor
/// advances to the next candidate (or the scan completes).
pub fn dot(tasks: &mut [Task], mode: Mode) -> Mode {
    let Mode::Preselect { cursor, .. } = mode else {
        return mode;
    };
    tasks[cursor].status = Status::Dotted;
    match next_candidate(tasks, cursor) {
        Some(c) => Mode::Preselect {
            benchmark: cursor,
            cursor: c,
        },
        None => Mode::Action { task: cursor },
    }
}

/// Undo the most recent dot: un-dot the benchmark, restore the previous dotted
/// task as the benchmark, and put the cursor on the just-undotted task so the
/// question can be re-answered. No-op when the benchmark is the only dot — a
/// scan always has a benchmark.
pub fn undot(tasks: &mut [Task], mode: Mode) -> Mode {
    let Mode::Preselect { benchmark, .. } = mode else {
        return mode;
    };
    let Some(prev) = tasks[..benchmark].iter().rposition(Task::is_dotted) else {
        return mode;
    };
    tasks[benchmark].status = Status::Open;
    Mode::Preselect {
        benchmark: prev,
        cursor: benchmark,
    }
}

/// Move the cursor down to the next candidate. Bounded like [`move_up`]: at the
/// last candidate this is a no-op — the scan ends only explicitly (via
/// [`finish_scan`]) or by dotting the last candidate.
pub fn move_down(tasks: &[Task], mode: Mode) -> Mode {
    let Mode::Preselect { benchmark, cursor } = mode else {
        return mode;
    };
    match next_candidate(tasks, cursor) {
        Some(c) => Mode::Preselect {
            benchmark,
            cursor: c,
        },
        None => mode,
    }
}

/// Move the cursor up to the previous candidate (never above the benchmark).
/// Non-destructive; stays put at the first candidate.
pub fn move_up(tasks: &[Task], mode: Mode) -> Mode {
    let Mode::Preselect { benchmark, cursor } = mode else {
        return mode;
    };
    match prev_candidate(tasks, cursor, benchmark) {
        Some(c) => Mode::Preselect {
            benchmark,
            cursor: c,
        },
        None => mode,
    }
}

/// Finish the scan immediately without dotting further: the benchmark becomes
/// the action task.
pub fn finish_scan(mode: Mode) -> Mode {
    match mode {
        Mode::Preselect { benchmark, .. } => Mode::Action { task: benchmark },
        other => other,
    }
}

/// Resume scanning from an Action state: continue dotting candidates below the
/// current last-dotted task (the action task). Stays in Action when there are no
/// un-dotted candidates left to consider.
pub fn resume_scan(tasks: &[Task], mode: Mode) -> Mode {
    let Mode::Action { task } = mode else {
        return mode;
    };
    match next_candidate(tasks, task) {
        Some(c) => Mode::Preselect {
            benchmark: task,
            cursor: c,
        },
        None => mode,
    }
}

/// Complete the action task. If other dots remain, stay in Action mode on the
/// new last-dotted task (the previous link in the chain) so work can continue
/// without re-scanning; FVP's post-completion re-scan is available on demand
/// via [`resume_scan`]. Only when no dots remain does a fresh scan begin.
pub fn complete(tasks: &mut [Task], mode: Mode) -> Mode {
    let Mode::Action { task: done_pos } = mode else {
        return mode;
    };
    tasks[done_pos].status = Status::Done;

    match last_dotted(tasks) {
        Some(nb) => Mode::Action { task: nb },
        None => start_scan(tasks),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tasks(names: &[&str]) -> Vec<Task> {
        names.iter().map(|n| Task::new(*n)).collect()
    }

    #[test]
    fn start_scan_dots_first_open_task() {
        let mut t = tasks(&["a", "b", "c"]);
        let mode = start_scan(&mut t);
        assert!(t[0].is_dotted());
        assert_eq!(
            mode,
            Mode::Preselect {
                benchmark: 0,
                cursor: 1
            }
        );
    }

    #[test]
    fn start_scan_single_task_goes_straight_to_action() {
        let mut t = tasks(&["only"]);
        let mode = start_scan(&mut t);
        assert_eq!(mode, Mode::Action { task: 0 });
    }

    #[test]
    fn empty_list_is_empty_mode() {
        let mut t: Vec<Task> = vec![];
        assert_eq!(start_scan(&mut t), Mode::Empty);
        assert_eq!(initial_mode(&mut t), Mode::Empty);
    }

    #[test]
    fn all_done_is_empty_mode() {
        let mut t = tasks(&["a"]);
        t[0].status = Status::Done;
        assert_eq!(start_scan(&mut t), Mode::Empty);
    }

    #[test]
    fn dotting_updates_benchmark_and_advances_cursor() {
        let mut t = tasks(&["a", "b", "c", "d"]);
        let mode = start_scan(&mut t); // dots a, cursor=1(b)
        let mode = dot(&mut t, mode); // dot b -> benchmark=1, cursor=2(c)
        assert!(t[1].is_dotted());
        assert_eq!(
            mode,
            Mode::Preselect {
                benchmark: 1,
                cursor: 2
            }
        );
    }

    #[test]
    fn undot_restores_previous_benchmark_and_reoffers_the_task() {
        let mut t = tasks(&["a", "b", "c", "d"]);
        let mode = start_scan(&mut t); // dot a, cursor=1(b)
        let mode = move_down(&t, mode); // cursor=2(c)
        let mode = dot(&mut t, mode); // dot c -> benchmark=2, cursor=3(d)
        let mode = undot(&mut t, mode); // change of mind: un-dot c
        assert!(t[2].is_open());
        // Benchmark is a again and c is the candidate under the cursor.
        assert_eq!(
            mode,
            Mode::Preselect {
                benchmark: 0,
                cursor: 2
            }
        );
        // b (skipped earlier) is back in the running via up-navigation.
        assert_eq!(
            move_up(&t, mode),
            Mode::Preselect {
                benchmark: 0,
                cursor: 1
            }
        );
    }

    #[test]
    fn undot_is_a_noop_on_the_only_dot() {
        let mut t = tasks(&["a", "b"]);
        let mode = start_scan(&mut t); // dot a (the only dot), cursor=1
        let mode = undot(&mut t, mode);
        assert!(t[0].is_dotted());
        assert_eq!(
            mode,
            Mode::Preselect {
                benchmark: 0,
                cursor: 1
            }
        );
    }

    #[test]
    fn move_down_is_bounded_at_last_candidate() {
        let mut t = tasks(&["a", "b", "c"]);
        let mode = start_scan(&mut t); // dot a, cursor=1
        let mode = dot(&mut t, mode); // dot b, cursor=2 (last candidate)
        let mode = move_down(&t, mode); // no next candidate -> stay put
        assert_eq!(
            mode,
            Mode::Preselect {
                benchmark: 1,
                cursor: 2
            }
        );
        // The scan ends only explicitly.
        assert_eq!(finish_scan(mode), Mode::Action { task: 1 });
    }

    #[test]
    fn move_up_is_non_destructive_and_bounded() {
        let mut t = tasks(&["a", "b", "c", "d"]);
        let mode = start_scan(&mut t); // benchmark=0, cursor=1
        let mode = move_down(&t, mode); // cursor=2
        let mode = move_up(&t, mode); // back to cursor=1
        assert_eq!(
            mode,
            Mode::Preselect {
                benchmark: 0,
                cursor: 1
            }
        );
        // Can't go above the first candidate.
        let mode = move_up(&t, mode);
        assert_eq!(
            mode,
            Mode::Preselect {
                benchmark: 0,
                cursor: 1
            }
        );
        // Nothing got dotted by navigating.
        assert_eq!(t.iter().filter(|x| x.is_dotted()).count(), 1);
    }

    #[test]
    fn finish_scan_actions_the_benchmark() {
        let mut t = tasks(&["a", "b", "c"]);
        let mode = start_scan(&mut t); // benchmark=0, cursor=1
        let mode = dot(&mut t, mode); // dot b -> benchmark=1, cursor=2
        assert_eq!(finish_scan(mode), Mode::Action { task: 1 });
    }

    #[test]
    fn complete_stays_in_action_on_remaining_dot() {
        // Dot a and c; skip b. Complete c -> stay in Action on a (the previous
        // link in the chain); no automatic re-scan.
        let mut t = tasks(&["a", "b", "c", "d"]);
        let mode = start_scan(&mut t); // dot a, cursor=1(b)
        let mode = move_down(&t, mode); // cursor=2(c)
        let mode = dot(&mut t, mode); // dot c -> benchmark=2, cursor=3(d)
        let mode = finish_scan(mode); // skip d, end scan -> Action{task:2}
        assert_eq!(mode, Mode::Action { task: 2 });
        let mode = complete(&mut t, mode); // finish c
        assert_eq!(t[2].status, Status::Done);
        assert_eq!(mode, Mode::Action { task: 0 });
        // Scanning is available on demand: resume offers candidates below a.
        let mode = resume_scan(&t, mode);
        assert_eq!(
            mode,
            Mode::Preselect {
                benchmark: 0,
                cursor: 1
            }
        );
    }

    #[test]
    fn resume_after_complete_picks_up_newly_added_task() {
        let mut t = tasks(&["a", "b"]);
        let mode = start_scan(&mut t); // dot a, cursor=1(b)
        let mode = dot(&mut t, mode); // dot b -> Action{task:1}
        assert_eq!(mode, Mode::Action { task: 1 });
        // Add a new task at the end (as the app does).
        t.push(Task::new("c"));
        let mode = complete(&mut t, mode); // finish b -> stay in Action on a
        assert_eq!(mode, Mode::Action { task: 0 });
        // An explicit resume offers the newly added task.
        let mode = resume_scan(&t, mode);
        assert_eq!(
            mode,
            Mode::Preselect {
                benchmark: 0,
                cursor: 2
            }
        );
    }

    #[test]
    fn completing_only_dot_starts_fresh_scan() {
        let mut t = tasks(&["a", "b"]);
        let mode = start_scan(&mut t); // dot a, cursor=1
        let mode = finish_scan(mode); // skip b, end scan -> Action{task:0}
        assert_eq!(mode, Mode::Action { task: 0 });
        let mode = complete(&mut t, mode); // finish a, no dots left -> fresh scan dots b
        assert!(t[1].is_dotted());
        assert_eq!(mode, Mode::Action { task: 1 });
    }

    #[test]
    fn resume_scan_reopens_scanning_below_current_task() {
        // Dot a, skip to end -> Action{0}. Resume should offer b as candidate.
        let mut t = tasks(&["a", "b", "c"]);
        let mode = start_scan(&mut t); // dot a, cursor=1(b)
        let mode = finish_scan(mode); // Action{task:0}
        assert_eq!(mode, Mode::Action { task: 0 });
        let mode = resume_scan(&t, mode);
        assert_eq!(
            mode,
            Mode::Preselect {
                benchmark: 0,
                cursor: 1
            }
        );
    }

    #[test]
    fn resume_scan_stays_in_action_when_no_candidates() {
        let mut t = tasks(&["a", "b"]);
        let mode = start_scan(&mut t); // dot a, cursor=1(b)
        let mode = dot(&mut t, mode); // dot b -> Action{task:1}, nothing below
        assert_eq!(mode, Mode::Action { task: 1 });
        assert_eq!(resume_scan(&t, mode), Mode::Action { task: 1 });
    }

    #[test]
    fn initial_mode_resumes_last_dotted() {
        let mut t = tasks(&["a", "b", "c"]);
        t[0].status = Status::Dotted;
        t[2].status = Status::Dotted;
        assert_eq!(initial_mode(&mut t), Mode::Action { task: 2 });
    }
}
