//! The task data model.

/// A task is always in exactly one of these states, mirroring the file markers
/// `[ ]` (Open), `[.]` (Dotted / pre-selected), and `[x]` (Done).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Not yet selected in the current scan.
    Open,
    /// "Dotted" — pre-selected in the current FVP scan.
    Dotted,
    /// Completed.
    Done,
}

/// A single task on the FVP list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    pub text: String,
    pub status: Status,
}

impl Task {
    /// Create a fresh open task.
    pub fn new(text: impl Into<String>) -> Self {
        Task {
            text: text.into(),
            status: Status::Open,
        }
    }

    /// Open and un-dotted — i.e. a scan candidate.
    pub fn is_open(&self) -> bool {
        self.status == Status::Open
    }

    pub fn is_dotted(&self) -> bool {
        self.status == Status::Dotted
    }

    /// Not done — still on the working list (Open or Dotted).
    pub fn is_active(&self) -> bool {
        self.status != Status::Done
    }
}
