//! The script of a debugging session and its runs, apart from the protocol and the window: the
//! target's project, loaded as `view` loads it, its script's text, its stops, and the run of a
//! stop, which plans the stop's text as a chart script of its own, at the same offsets as the
//! script, so its errors have their places in the file.

use std::path::Path;

use avenger_sql_project::ProjectStylesheet;
use datafusion::{
    arrow::{array::RecordBatch, datatypes::SchemaRef},
    common::ScalarValue,
    dataframe::DataFrame,
    physical_plan::SendableRecordBatchStream,
};
use futures::StreamExt;

use super::stops::Stops;
use crate::render::{load, plan_text, text_variables, Failure, Target};
use crate::statement::Script;
use crate::view::{lines, outcome, Outcome};

/// The target of a session and its stops.
pub(crate) struct Debuggee {
    target: Target,
    /// The text of the target's script, or of its cell.
    text: String,
    /// The lines of the file before the text: those of a cell's document before the cell.
    before: usize,
    stops: Stops,
}

/// A stop's run: its rows and its variables, which the debugger's panes show.
pub(crate) struct Ran {
    pub(crate) rows: Result<Rows, Vec<String>>,
    pub(crate) variables: Result<Vec<(String, ScalarValue)>, Vec<String>>,
}

impl Debuggee {
    /// Loads the project of `target`, as `view` takes it: a chart script, a cell `doc.qmd#NAME`,
    /// or the statement at a line of either, `FILE:LINE`. A data script's target takes no session
    /// yet.
    pub(crate) async fn open(target: &str) -> Result<Debuggee, String> {
        let loaded = load(target).await.map_err(|failure| match failure {
            Failure::Usage(error) => error,
            Failure::Reports(reports) => lines(&reports).join("\n"),
        })?;
        if loaded.data {
            return Err(format!(
                "{target}: a data script's statements take no debugging session yet; debug a chart \
                 script, a cell, or a statement of either"
            ));
        }
        let text = loaded.script_text();
        let statement = loaded.statement.as_ref().map(|statement| statement.start);
        let stops = Stops::new(&text, Script::Chart, statement)
            .ok_or_else(|| format!("{target}: the script has no query, so no stage to stop at"))?;
        Ok(Debuggee {
            before: loaded.lines_before(),
            target: loaded,
            text,
            stops,
        })
    }

    pub(crate) fn stops(&self) -> &Stops {
        &self.stops
    }

    /// The file on disk of the target.
    pub(crate) fn file(&self) -> &Path {
        &self.target.file
    }

    /// The name of the target's file, as the debugger's frames name it.
    pub(crate) fn display_name(&self) -> String {
        self.target.display_name()
    }

    pub(crate) fn stylesheet(&self) -> &ProjectStylesheet {
        &self.target.stylesheet
    }

    /// The window's title at the stop at `index`: the target's file, and the stop's name and line.
    pub(crate) fn title(&self, index: usize) -> String {
        let stop = self.stops.get(index);
        format!(
            "avenger dap {}: {} at line {}",
            self.display_name(),
            stop.name,
            self.file_line(stop.line)
        )
    }

    /// The line of the file of the line `line` of the script's text, both from 1.
    pub(crate) fn file_line(&self, line: usize) -> usize {
        line + self.before
    }

    /// The line of the script's text of the line `line` of the file, both from 1, or `None` for a
    /// line of a document outside the target's cell.
    pub(crate) fn script_line(&self, line: usize) -> Option<usize> {
        let line = line.checked_sub(self.before).filter(|line| *line >= 1)?;
        let lines = self.text.matches('\n').count() + 1;
        (line <= lines).then_some(line)
    }

    /// Whether `path`, a file of a breakpoint, is the target's file.
    pub(crate) fn is_file(&self, path: &Path) -> bool {
        let canonical = |path: &Path| std::fs::canonicalize(path).unwrap_or(path.to_path_buf());
        canonical(path) == canonical(&self.target.file)
    }

    /// Runs the stop at `index`: plans its text and gives what the window shows, and its rows,
    /// which read as the debugger's panes ask for them, and its variables.
    pub(crate) async fn run(&mut self, index: usize) -> (Outcome, Ran) {
        let text = self.stops.text(index, &self.text);
        let failed = |failure: Failure| match failure {
            Failure::Usage(error) => vec![error],
            Failure::Reports(reports) => lines(&reports),
        };
        let frame = match plan_text(&mut self.target, &text).await {
            Ok(frame) => frame,
            Err(failure) => {
                let errors = failed(failure);
                let ran = Ran {
                    rows: Err(errors.clone()),
                    variables: Err(errors.clone()),
                };
                return (Outcome::Failed(errors), ran);
            }
        };
        let variables = text_variables(&self.target, &text).await.map_err(failed);
        let rows = Ok(Rows::new(frame.clone()));
        (outcome(&self.target, frame).await, Ran { rows, variables })
    }
}

/// The rows of a stop, read forward from one run of its frame as the debugger asks for them and
/// kept, and their count, which runs once when it is first asked for.
pub(crate) struct Rows {
    frame: DataFrame,
    stream: Option<SendableRecordBatchStream>,
    batches: Vec<RecordBatch>,
    read: usize,
    done: bool,
    count: Option<usize>,
}

impl Rows {
    fn new(frame: DataFrame) -> Rows {
        Rows {
            frame,
            stream: None,
            batches: Vec::new(),
            read: 0,
            done: false,
            count: None,
        }
    }

    pub(crate) fn schema(&self) -> SchemaRef {
        self.frame.schema().inner().clone()
    }

    /// The count of the rows, which runs the frame's count the first time.
    pub(crate) async fn count(&mut self) -> Result<usize, String> {
        if let Some(count) = self.count {
            return Ok(count);
        }
        let count = (self.frame.clone().count().await).map_err(|error| error.to_string())?;
        self.count = Some(count);
        Ok(count)
    }

    /// The rows `start..end`, or as many of them as there are, as batches of the rows kept,
    /// reading on from the stream as far as `end`.
    pub(crate) async fn rows(
        &mut self,
        start: usize,
        end: usize,
    ) -> Result<Vec<RecordBatch>, String> {
        while self.read < end && !self.done {
            if self.stream.is_none() {
                let stream = self.frame.clone().execute_stream().await;
                self.stream = Some(stream.map_err(|error| error.to_string())?);
            }
            let stream = self.stream.as_mut().expect("the stream of the rows");
            match stream.next().await {
                Some(batch) => {
                    let batch = batch.map_err(|error| error.to_string())?;
                    self.read += batch.num_rows();
                    self.batches.push(batch);
                }
                None => self.done = true,
            }
        }
        let mut rows = Vec::new();
        let mut offset = 0;
        for batch in &self.batches {
            let (first, last) = (offset, offset + batch.num_rows());
            offset = last;
            let (from, to) = (start.max(first), end.min(last));
            if from < to {
                rows.push(batch.slice(from - first, to - from));
            }
        }
        Ok(rows)
    }
}
