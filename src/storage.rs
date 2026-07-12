//! Plain-text, hand-editable persistence.
//!
//! One task per line, list order = file order. A three-char status prefix leads
//! each line:
//!
//! ```text
//! [x] Call dentist          done
//! [.] Write report          dotted / pre-selected
//! [ ] Buy milk              open
//! Pick up dry cleaning       freeform line -> implied open task
//! ```
//!
//! Parsing is lenient: blank lines are ignored and any line that does not begin
//! with a known prefix is treated as an implied open task (so a task can be
//! added by hand just by typing it). On save every task is written with its
//! explicit prefix, normalizing implied lines to `[ ] ...`.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::task::{Status, Task};

/// Parse the file contents into a task list.
pub fn parse(contents: &str) -> Vec<Task> {
    contents.lines().filter_map(parse_line).collect()
}

fn parse_line(line: &str) -> Option<Task> {
    if line.trim().is_empty() {
        return None;
    }
    let (status, rest) = if let Some(rest) = line.strip_prefix("[x]") {
        (Status::Done, rest)
    } else if let Some(rest) = line.strip_prefix("[.]") {
        (Status::Dotted, rest)
    } else if let Some(rest) = line.strip_prefix("[ ]") {
        (Status::Open, rest)
    } else {
        // Freeform line -> implied open task; keep the whole line as text.
        (Status::Open, line)
    };
    // Drop a single separating space after the prefix, then trim trailing space.
    let text = rest.strip_prefix(' ').unwrap_or(rest).trim_end();
    Some(Task {
        text: text.to_string(),
        status,
    })
}

/// Serialize a task list back to the plain-text format.
pub fn serialize(tasks: &[Task]) -> String {
    let mut out = String::new();
    for t in tasks {
        let marker = match t.status {
            Status::Open => "[ ]",
            Status::Dotted => "[.]",
            Status::Done => "[x]",
        };
        out.push_str(marker);
        out.push(' ');
        out.push_str(&t.text);
        out.push('\n');
    }
    out
}

/// Load tasks from `path`. A missing file yields an empty list.
pub fn load(path: &Path) -> Result<Vec<Task>> {
    match fs::read_to_string(path) {
        Ok(contents) => Ok(parse(&contents)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

/// Copy the task file to a dated sibling (`tasks.txt-20260712`) and return the
/// backup's path. Same-day collisions get a numeric suffix (`-2`, `-3`, …) so
/// earlier backups are never overwritten.
pub fn backup(path: &Path) -> Result<PathBuf> {
    let stamp = chrono::Local::now().format("%Y%m%d").to_string();
    backup_as(path, &stamp)
}

fn backup_as(path: &Path, stamp: &str) -> Result<PathBuf> {
    let mut base = path.as_os_str().to_os_string();
    base.push(format!("-{stamp}"));
    let mut candidate = PathBuf::from(&base);
    let mut n = 1;
    while candidate.exists() {
        n += 1;
        let mut next = base.clone();
        next.push(format!("-{n}"));
        candidate = PathBuf::from(next);
    }
    fs::copy(path, &candidate)
        .with_context(|| format!("backing up {} to {}", path.display(), candidate.display()))?;
    Ok(candidate)
}

/// Save tasks to `path`, creating parent directories as needed.
pub fn save(path: &Path, tasks: &[Task]) -> Result<()> {
    if let Some(dir) = path.parent()
        && !dir.as_os_str().is_empty()
    {
        fs::create_dir_all(dir).with_context(|| format!("creating directory {}", dir.display()))?;
    }
    fs::write(path, serialize(tasks)).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_markers_and_freeform() {
        let input = "\
[x] Call dentist
[.] Write report
[ ] Buy milk
Pick up dry cleaning

   \t
";
        let tasks = parse(input);
        assert_eq!(tasks.len(), 4); // blank/whitespace lines ignored
        assert_eq!(tasks[0].status, Status::Done);
        assert_eq!(tasks[0].text, "Call dentist");
        assert_eq!(tasks[1].status, Status::Dotted);
        assert_eq!(tasks[2].status, Status::Open);
        assert_eq!(tasks[3].status, Status::Open);
        assert_eq!(tasks[3].text, "Pick up dry cleaning");
    }

    #[test]
    fn freeform_line_that_looks_like_a_bracket_is_kept_verbatim() {
        let tasks = parse("[urgent] fix the thing\n");
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].status, Status::Open);
        assert_eq!(tasks[0].text, "[urgent] fix the thing");
    }

    #[test]
    fn round_trip_after_normalization_is_stable() {
        // Freeform input normalizes on first serialize; a second round trip is identical.
        let original = parse("[x] a\n[.] b\n[ ] c\nfreeform d\n");
        let once = serialize(&original);
        let twice = serialize(&parse(&once));
        assert_eq!(once, twice);
        assert_eq!(once, "[x] a\n[.] b\n[ ] c\n[ ] freeform d\n");
    }

    #[test]
    fn backup_copies_and_numbers_collisions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tasks.txt");
        fs::write(&path, "[x] a\n").unwrap();

        let b1 = backup_as(&path, "20260712").unwrap();
        assert_eq!(b1, dir.path().join("tasks.txt-20260712"));
        assert_eq!(fs::read_to_string(&b1).unwrap(), "[x] a\n");

        // Same-day second backup gets -2; a third gets -3.
        fs::write(&path, "[x] b\n").unwrap();
        let b2 = backup_as(&path, "20260712").unwrap();
        assert_eq!(b2, dir.path().join("tasks.txt-20260712-2"));
        assert_eq!(fs::read_to_string(&b2).unwrap(), "[x] b\n");
        let b3 = backup_as(&path, "20260712").unwrap();
        assert_eq!(b3, dir.path().join("tasks.txt-20260712-3"));
        // The originals are untouched.
        assert_eq!(fs::read_to_string(&b1).unwrap(), "[x] a\n");
    }

    #[test]
    fn empty_task_text_survives_round_trip() {
        let tasks = parse("[ ] \n");
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].text, "");
        assert_eq!(serialize(&tasks), "[ ] \n");
    }
}
