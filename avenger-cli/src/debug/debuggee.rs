//! The script of a debugging session and its runs, apart from the protocol and the window: the
//! target's project, kept by a `view` session so it follows the project's files as `view` does,
//! its script's text, its stops, the values that the debugger gives its variables, and the run of
//! a stop, which plans the stop's text as a chart script of its own, at the same offsets as the
//! script, so its errors have their places in the file.

use std::path::{Path, PathBuf};

use avenger_sql::analysis::Overrides;
use avenger_sql::types::googlesql_name;
use avenger_sql_project::ProjectStylesheet;
use datafusion::{
    arrow::{array::RecordBatch, datatypes::DataType, datatypes::SchemaRef},
    common::ScalarValue,
    dataframe::DataFrame,
    physical_plan::SendableRecordBatchStream,
};
use futures::StreamExt;

use super::stops::{Place, Stops};
use crate::render::{plan_text, step_text, text_variables, Failure, Target};
use crate::statement::Shows;
use crate::view::{canonical, lines, outcome, Outcome, Session};

/// The target of a session and its stops.
pub(crate) struct Debuggee {
    /// The target as the session launched it, which `restart` opens again.
    name: String,
    session: Session,
    /// The text of the target's script, or for a cell, its document's script masked to what the
    /// cell's statement reads, whose lines are the document's.
    text: String,
    stops: Stops,
    /// The values that the debugger gives the script's variables, by name.
    overrides: Overrides,
}

/// A stop's run: its rows and its variables, which the debugger's panes show.
pub(crate) struct Ran {
    pub(crate) rows: Result<Rows, Vec<String>>,
    pub(crate) variables: Result<Vec<(String, ScalarValue)>, Vec<String>>,
}

/// The text of `target`'s script and its stops.
fn stops_of(target: &Target, name: &str) -> Result<(String, Stops), String> {
    let text = target.script_text();
    let statement = target.statement.as_ref().map(|statement| statement.start);
    let stops = Stops::new(&text, target.script_kind(), statement)
        .ok_or_else(|| format!("{name}: the script has no query, so no stage to stop at"))?;
    Ok((text, stops))
}

impl Debuggee {
    /// Loads the project of `target`, as `view` takes it: a chart script, a cell `doc.qmd#NAME`,
    /// or the statement at a line of either, `FILE:LINE`, of a chart script or a data script.
    pub(crate) async fn open(target: &str) -> Result<Debuggee, String> {
        let (session, _) = Session::open(target).await?;
        let (text, stops) = stops_of(session.target(), target)?;
        Ok(Debuggee {
            name: target.to_string(),
            session,
            text,
            stops,
            overrides: Overrides::new(),
        })
    }

    /// Opens the target's project again, as at launch, keeping the values of the variables.
    pub(crate) async fn reopen(&mut self) -> Result<(), String> {
        let overrides = std::mem::take(&mut self.overrides);
        *self = Debuggee::open(&self.name).await?;
        self.overrides = overrides;
        Ok(())
    }

    /// What the changes of the files at `paths`, canonical paths on disk, do, as they do to a
    /// `view` window: the outcomes of the target's own run, after which the script's text and its
    /// stops are read again.
    pub(crate) async fn changed(&mut self, paths: &[PathBuf]) -> Result<Vec<Outcome>, String> {
        let outcomes = self.session.changed(paths).await;
        (self.text, self.stops) = stops_of(self.session.target(), &self.name)?;
        Ok(outcomes)
    }

    fn target(&self) -> &Target {
        self.session.target()
    }

    /// The directory of the project's files, which the session watches.
    pub(crate) fn watched_directory(&self) -> PathBuf {
        self.session.watched_directory()
    }

    pub(crate) fn stops(&self) -> &Stops {
        &self.stops
    }

    /// The file on disk of the target.
    pub(crate) fn file(&self) -> &Path {
        &self.target().file
    }

    /// The name of the target's file, as the debugger's frames name it.
    pub(crate) fn display_name(&self) -> String {
        self.target().display_name()
    }

    pub(crate) fn stylesheet(&self) -> &ProjectStylesheet {
        &self.target().stylesheet
    }

    /// The window's title at the stop at `index`: the target's file, and the stop's name and line.
    pub(crate) fn title(&self, index: usize) -> String {
        let stop = self.stops.get(index);
        format!(
            "avenger dap {}: {} at line {}",
            self.display_name(),
            stop.name,
            stop.line
        )
    }

    /// The line of the script's text of the line `line` of the file, both from 1, or why the
    /// line takes no stop: a line after the script's text, a line of a document in no cell, or
    /// a statement of an earlier cell that the cell's statement doesn't read.
    pub(crate) fn script_line(&self, line: usize) -> Result<usize, String> {
        let nth = |text: &str| {
            text.lines()
                .nth(line.wrapping_sub(1))
                .unwrap_or("")
                .trim()
                .to_string()
        };
        if let Some(document) = &self.target().document {
            let line_of = |offset: usize| document.text[..offset].matches('\n').count() + 1;
            let in_cell = (document.cells.iter())
                .any(|cell| (line_of(cell.range.start)..=line_of(cell.range.end)).contains(&line));
            if !in_cell {
                return Err("the line is in no `{avenger-sql}` cell of the document".into());
            }
            if !nth(&document.script.text).is_empty() && nth(&self.text).is_empty() {
                return Err(format!(
                    "the statement that the session debugs reads no statement at line {line}"
                ));
            }
        }
        let lines = self.text.matches('\n').count() + 1;
        match (1..=lines).contains(&line) {
            true => Ok(line),
            false => Err(format!("the session's script ends at line {lines}")),
        }
    }

    /// Whether `path`, a file of a breakpoint, is the target's file.
    pub(crate) fn is_file(&self, path: &Path) -> bool {
        canonical(path) == canonical(&self.target().file)
    }

    /// Plans `text`, a part of the script, with the debugger's values of its variables.
    async fn plan(&mut self, text: &str) -> Result<DataFrame, Failure> {
        plan_text(self.session.target_mut(), text, &self.overrides).await
    }

    /// Plans the stop at `index`. A chart script's stop runs its text as a chart script. A data
    /// script's stage runs its statement with the query cut before the operator, through the
    /// loader's step, and its result is its table, from the project's session or from a step.
    async fn plan_stop(&mut self, index: usize) -> Result<DataFrame, Failure> {
        if !self.target().data {
            let text = self.stops.text(index, &self.text);
            return self.plan(&text).await;
        }
        let at = self.stops.staged(index).clone();
        match (self.stops.get(index).place, &at.statement.shows) {
            (Place::Stage(stage), _) => {
                let text = at.data_stage_text(&self.text, &at.operators[stage]);
                step_text(self.target(), &text, at.statement.start).await
            }
            (Place::Result, Shows::Loaded(path)) => self.plan(&format!("FROM {path}\n")).await,
            (Place::Result, _) => step_text(self.target(), &self.text, at.statement.start).await,
        }
    }

    /// Runs the stop at `index`: plans its text and gives what the window shows, and its rows,
    /// which read as the debugger's panes ask for them, and its variables.
    pub(crate) async fn run(&mut self, index: usize) -> (Outcome, Ran) {
        let failed = |failure: Failure| match failure {
            Failure::Usage(error) => vec![error],
            Failure::Reports(reports) => lines(&reports),
        };
        let frame = match self.plan_stop(index).await {
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
        // A data script's statements read no variables.
        let variables = match self.target().data {
            true => Ok(Vec::new()),
            false => {
                let text = self.stops.text(index, &self.text);
                let variables = text_variables(self.target(), &text, &self.overrides).await;
                variables.map_err(failed)
            }
        };
        let rows = Ok(Rows::new(frame.clone()));
        (outcome(self.target(), frame).await, Ran { rows, variables })
    }

    /// What the window shows of `frame`, the frame of a part of the script.
    pub(crate) async fn outcome(&self, frame: DataFrame) -> Outcome {
        outcome(self.target(), frame).await
    }

    /// Gives the variable `name`, of the type `ty`, the value that `value` writes, as a `SET`
    /// would, from the next run on: a number, a quoted or a bare string, `TRUE`, a date, or `NULL`.
    /// A value that isn't of the type is an error that names it.
    pub(crate) fn set_variable(
        &mut self,
        name: &str,
        value: &str,
        ty: &DataType,
    ) -> Result<ScalarValue, String> {
        let text = value.trim();
        let wrong = || format!("`{name}` takes a value of type {}", googlesql_name(ty));
        let scalar = match text.eq_ignore_ascii_case("NULL") {
            true => ScalarValue::try_from(ty).map_err(|_| wrong())?,
            false => {
                let unquoted = (text.strip_prefix('\'').and_then(|t| t.strip_suffix('\'')))
                    .or_else(|| text.strip_prefix('"').and_then(|t| t.strip_suffix('"')))
                    .unwrap_or(text);
                let scalar = ScalarValue::try_from_string(unquoted.to_string(), ty);
                scalar
                    .ok()
                    .filter(|scalar| !scalar.is_null())
                    .ok_or_else(wrong)?
            }
        };
        self.overrides.insert(name.to_string(), scalar.clone());
        Ok(scalar)
    }

    /// The rows of `expression`, which the debug console runs at the stop at `index`: pipe
    /// operators, as `|> WHERE amount > 100`, after the stop's rows, or a query that starts with
    /// `FROM`, after the statements before the stop. The errors are their messages.
    pub(crate) async fn evaluate(
        &mut self,
        index: usize,
        expression: &str,
    ) -> Result<DataFrame, String> {
        let expression = expression.trim();
        let from = (expression.get(..4)).is_some_and(|word| word.eq_ignore_ascii_case("FROM"));
        let at = self.stops.staged(index).clone();
        let place = self.stops.get(index).place;
        // A data script's console runs queries of the project's tables, as a chart script does.
        let text = if self.target().data {
            if !from {
                return Err(
                    "in a data script, the console runs queries that start with FROM".into(),
                );
            }
            expression.to_string()
        } else if expression.starts_with("|>") {
            let rows = match (place, &at.statement.shows) {
                (Place::Stage(stage), _) => at.stage_text(&self.text, &at.operators[stage]),
                (Place::Result, Shows::Table(path)) => {
                    format!("{};\nFROM {path}", &self.text[..at.statement.end])
                }
                (Place::Result, _) => self.text[..at.statement.end].to_string(),
            };
            format!("{rows}\n{expression}")
        } else if from {
            match place {
                Place::Stage(_) => format!("{}{expression}", &self.text[..at.statement.start]),
                Place::Result => format!("{};\n{expression}", &self.text[..at.statement.end]),
            }
        } else {
            return Err(
                "the console takes pipe operators, as `|> WHERE x > 0`, which run after the stop's \
                 rows, or a query that starts with FROM"
                    .into(),
            );
        };
        match self.plan(&text).await {
            Ok(frame) => Ok(frame),
            Err(Failure::Reports(reports)) => {
                let messages: Vec<String> =
                    reports.into_iter().map(|report| report.message).collect();
                Err(messages.join("\n"))
            }
            Err(Failure::Usage(error)) => Err(error),
        }
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
    pub(crate) fn new(frame: DataFrame) -> Rows {
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
