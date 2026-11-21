//! A text of the editor and its last run: the query of the editor without a project, or the
//! file of a tab of a project.

use avenger_common::time::{Duration, Instant};

use crate::editor::CodeEditor;
use crate::exec::Task;
use crate::query::{LiftedText, QueryError};
use crate::result::QueryResult;

/// A text, its code editor, which keeps its cursor and its undo history, and its last run.
pub struct Buffer {
    pub text: String,
    pub editor: CodeEditor,
    /// The run of the text that has not ended.
    pub running: Option<Running>,
    /// The id of the last run that started, whose lifted form the buffer takes.
    pub latest: u64,
    /// The last result of a run, which the result pane shows.
    pub shown: Option<Shown>,
    /// The lifted form of the text of the last run, once it arrives.
    pub lifted: Option<Result<LiftedText, QueryError>>,
    /// The error of the last run, while the result pane keeps the last result that ran.
    pub error: Option<QueryError>,
}

/// A run of a buffer's text, which a newer run replaces.
pub struct Running {
    pub id: u64,
    pub task: Task,
    pub started: Instant,
}

/// The result of a run: its id, which names the cells of its charts, its result and plan, and
/// how long it ran.
pub struct Shown {
    pub id: u64,
    pub result: QueryResult,
    pub plan: String,
    pub elapsed: Duration,
}

impl Buffer {
    /// The buffer of `text`, whose code editor keeps its state in egui's memory under `id`.
    pub fn new(text: String, id: impl std::hash::Hash) -> Self {
        Self {
            text,
            editor: CodeEditor::new(id),
            running: None,
            latest: 0,
            shown: None,
            lifted: None,
            error: None,
        }
    }

    /// Stops the run that has not ended, and whether there was one.
    pub fn cancel(&mut self) -> bool {
        match self.running.take() {
            Some(running) => {
                running.task.abort();
                true
            }
            None => false,
        }
    }
}
