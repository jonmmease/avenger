//! The adapter's side of a session: the requests of the Debug Adapter Protocol, answered in the
//! order they come, the stops that they move to, and what each stop shows: the window's rows, the
//! frame that the editor marks, and the scopes and variables of its panes.
//!
//! A session has one thread, `query`, with one frame, the current stop's statement. The client
//! sends `launch` with the target, as `view` takes it, and the adapter answers with `initialized`
//! once the target's project loads, so the breakpoints that follow bind to its stops. After
//! `configurationDone`, the session stops at its first stop, and each step stops again.

use std::collections::BTreeSet;
use std::io::Write;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

use avenger_chart::results::is_chart_field;
use avenger_sql::types::googlesql_name;
use datafusion::arrow::{
    array::RecordBatch,
    util::display::{ArrayFormatter, FormatOptions},
};
use serde_json::{json, Value};

use super::debuggee::{Debuggee, Ran, Rows};
use super::protocol::{Output, Request};
use super::stops::{Move, Place, Reason};
use avenger_results_egui::ChartState;

use crate::view::{Message, Sink, WindowEvent};

/// The id of the session's thread, and of its frame.
const THREAD: i64 = 1;

/// The rows of a stop that a child of its rows holds, when it has more: its rows are chunks of
/// this many, since a client may not page a variable's children.
const CHUNK: usize = 100;

/// The longest value of a row, as a summary of its fields.
const SUMMARY: usize = 120;

/// What a variable reference holds, among a stop's variables.
#[derive(Clone, Copy, Debug)]
enum Var {
    /// The scope of a stop's rows: its columns and its rows.
    Rows,
    Columns,
    AllRows,
    /// The rows `start..end`.
    Chunk(usize, usize),
    /// A row's fields.
    Row(usize),
    /// The script's variables.
    Script,
    /// The state of the chart at a result stop: its selections, widgets, scales, and modes.
    Selections,
    Widgets,
    Scales,
    Modes,
}

/// A breakpoint of the target's file: its id, and the stop that it binds to, if any.
struct Breakpoint {
    id: i64,
    bound: Option<usize>,
}

/// The window's side of a session: a request to open the window, titled as the request says,
/// which answers with the window's sink.
pub(crate) struct Open {
    pub(crate) title: String,
    pub(crate) sink: Sender<Sink>,
}

/// The adapter of a session.
pub(crate) struct Adapter<W: Write> {
    out: Output<W>,
    runtime: tokio::runtime::Runtime,
    /// The request to open the window, without `--headless`.
    window: Option<Sender<Open>>,
    sink: Option<Sink>,
    debuggee: Option<Debuggee>,
    stop_on_entry: bool,
    /// The breakpoints of the target's file, and the ids of breakpoints so far.
    breakpoints: Vec<Breakpoint>,
    ids: i64,
    /// The console's lines about breakpoints that moved or bound to no stop, which go out after
    /// the response that binds them.
    notes: Vec<String>,
    /// The current stop, its run, and what the variable references of its panes hold.
    stop: Option<usize>,
    /// The state of the chart that fills the window, as it last reported it.
    chart: Option<ChartState>,
    /// Whether the client refreshes its panes on an `invalidated` event.
    invalidated: bool,
    ran: Option<Ran>,
    vars: Vec<Var>,
}

impl<W: Write> Adapter<W> {
    /// An adapter that writes to `out`, and opens a window through `window`, or none without one.
    pub(crate) fn new(out: W, window: Option<Sender<Open>>) -> Result<Self, String> {
        // A loaded project's futures are not `Send`, so they run on this thread.
        let runtime = (tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build())
        .map_err(|error| error.to_string())?;
        Ok(Adapter {
            out: Output::new(out),
            runtime,
            window,
            sink: None,
            debuggee: None,
            stop_on_entry: false,
            breakpoints: Vec::new(),
            ids: 0,
            notes: Vec::new(),
            stop: None,
            chart: None,
            invalidated: false,
            ran: None,
            vars: Vec::new(),
        })
    }

    /// Answers the messages of `messages`, `None` at the end of the input, until the client
    /// disconnects or the input ends. When the window closes, the session ends with `terminated`.
    pub(crate) fn serve(mut self, messages: Receiver<Option<Result<Value, String>>>) {
        let mut terminated = false;
        loop {
            let message = match messages.recv_timeout(Duration::from_millis(200)) {
                Ok(Some(message)) => message,
                Ok(None) | Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => {
                    if !terminated && self.sink.as_ref().is_some_and(Sink::closed) {
                        terminated = true;
                        self.out.event("terminated", json!({}));
                    }
                    self.window_events();
                    continue;
                }
            };
            let request = match message.and_then(|message| {
                eprintln!("avenger dap <- {message}");
                serde_json::from_value::<Request>(message).map_err(|error| error.to_string())
            }) {
                Ok(request) => request,
                Err(error) => {
                    eprintln!("avenger dap: {error}");
                    continue;
                }
            };
            if !self.answer(&request) {
                break;
            }
        }
        if let Some(sink) = &self.sink {
            sink.close();
        }
    }

    /// Hears the state of the chart that fills the window, as the window reports it.
    fn window_events(&mut self) {
        let Some(sink) = &self.sink else {
            return;
        };
        let events: Vec<WindowEvent> = sink.events().try_iter().collect();
        for WindowEvent { state, change } in events {
            self.chart_changed(state, change);
        }
    }

    /// The chart that fills the window reported its state: when it first shows, or after a
    /// gesture that settled, named by `change`. At a result stop, the client refreshes its panes:
    /// with `invalidated`, when it supports the event, or else with a stop at the same frame, and
    /// the console names a gesture's change.
    pub(crate) fn chart_changed(&mut self, state: ChartState, change: Option<String>) {
        let first = self.chart.is_none();
        self.chart = Some(state);
        let at_result = match (&self.debuggee, self.stop) {
            (Some(debuggee), Some(index)) => debuggee.stops().get(index).place == Place::Result,
            _ => false,
        };
        if !at_result || (change.is_none() && !first) {
            return;
        }
        if let Some(change) = &change {
            self.out.console(change);
        }
        match self.invalidated {
            true => self.out.event(
                "invalidated",
                json!({"areas": ["variables"], "threadId": THREAD, "stackFrameId": THREAD}),
            ),
            false => self.stopped(Reason::Pause),
        }
    }

    /// Answers `request`, and whether the session goes on.
    pub(crate) fn answer(&mut self, request: &Request) -> bool {
        match request.command.as_str() {
            "initialize" => {
                self.invalidated = request.arguments["supportsInvalidatedEvent"]
                    .as_bool()
                    .unwrap_or(false);
                self.out.respond(request, capabilities());
            }
            "launch" => self.launch(request),
            "setBreakpoints" => self.set_breakpoints(request),
            "setExceptionBreakpoints" => self.out.respond(request, json!({"breakpoints": []})),
            "configurationDone" => self.configuration_done(request),
            "threads" => self.out.respond(
                request,
                json!({"threads": [{"id": THREAD, "name": "query"}]}),
            ),
            "stackTrace" => self.stack_trace(request),
            "scopes" => self.scopes(request),
            "variables" => self.variables(request),
            "next" | "stepIn" => self.step(request, Move::Next),
            "stepOut" => self.step(request, Move::Out),
            "continue" => self.step(request, Move::Continue),
            "stepBack" => self.step(request, Move::Back),
            "reverseContinue" => self.step(request, Move::ReverseContinue),
            // The session stops at each stage and never runs on, so a pause has nothing to do.
            "pause" => self.out.respond(request, json!({})),
            "disconnect" | "terminate" => {
                self.out.respond(request, json!({}));
                if request.command == "terminate" {
                    self.out.event("terminated", json!({}));
                }
                return false;
            }
            command => {
                self.out.fail(
                    request,
                    &format!("avenger dap does not support `{command}`"),
                );
            }
        }
        true
    }

    /// `launch`: loads the target's project, opens the window on it, and says `initialized`, so
    /// the client sends its breakpoints.
    fn launch(&mut self, request: &Request) {
        let args = &request.arguments;
        let Some(target) = args["target"].as_str() else {
            self.out.fail(
                request,
                "the launch configuration names no `target`: a chart script, `doc.qmd#NAME`, or \
                 `FILE:LINE`",
            );
            return;
        };
        if let Some(cwd) = args["cwd"].as_str() {
            if let Err(error) = std::env::set_current_dir(cwd) {
                self.out.fail(request, &format!("{cwd}: {error}"));
                return;
            }
        }
        self.stop_on_entry = args["stopOnEntry"].as_bool().unwrap_or(false);
        let debuggee = match self.runtime.block_on(Debuggee::open(target)) {
            Ok(debuggee) => debuggee,
            Err(error) => {
                self.out.fail(request, &error);
                return;
            }
        };
        if let Some(window) = self.window.take() {
            let (sink, sinks) = std::sync::mpsc::channel();
            let title = format!("avenger dap {}", debuggee.display_name());
            if window.send(Open { title, sink }).is_ok() {
                if let Ok(sink) = sinks.recv() {
                    let sheet = debuggee.stylesheet().sheet.clone();
                    sink.send(Message::Opened(Ok(sheet)));
                    self.sink = Some(sink);
                }
            }
        }
        self.debuggee = Some(debuggee);
        self.out.respond(request, json!({}));
        self.out.event("initialized", json!({}));
    }

    /// `setBreakpoints`: binds each line of a file to a stop of the target's script.
    fn set_breakpoints(&mut self, request: &Request) {
        let args = &request.arguments;
        let path = PathBuf::from(args["source"]["path"].as_str().unwrap_or_default());
        let lines: Vec<usize> = (args["breakpoints"].as_array().into_iter().flatten())
            .filter_map(|breakpoint| breakpoint["line"].as_u64())
            .map(|line| line as usize)
            .collect();
        // A client sends its breakpoints after `initialized`, which follows the target's load.
        if self.debuggee.is_none() {
            self.out.fail(
                request,
                "the session takes breakpoints once `launch` loads its target",
            );
            return;
        }
        let breakpoints = self.bind(&path, &lines);
        self.out
            .respond(request, json!({"breakpoints": breakpoints}));
        self.notes();
    }

    fn next_id(&mut self) -> i64 {
        self.ids += 1;
        self.ids
    }

    /// Binds the breakpoints at `lines` of the file `path`, with new ids, and gives them as the
    /// protocol's breakpoints. The breakpoints of the target's file replace its breakpoints before.
    fn bind(&mut self, path: &std::path::Path, lines: &[usize]) -> Vec<Value> {
        let ids: Vec<i64> = lines.iter().map(|_| self.next_id()).collect();
        let debuggee = self.debuggee.as_ref().expect("a loaded target");
        let ours = debuggee.is_file(path);
        let mut breakpoints = Vec::new();
        let mut kept = Vec::new();
        let name = debuggee.display_name();
        for (line, id) in lines.iter().zip(ids) {
            let bound = match ours {
                false => Err(format!(
                    "the session debugs `{}`; a breakpoint of another file takes no stop",
                    debuggee.display_name()
                )),
                true => match debuggee.script_line(*line) {
                    None => Err("the line is outside the cell that the session debugs".into()),
                    Some(script_line) => debuggee.stops().bind(script_line),
                },
            };
            // A client may draw a breakpoint at its own line and keep no message, as Zed does,
            // so the console says where a breakpoint of the target's file stops, or why not.
            breakpoints.push(match &bound {
                Ok(index) => {
                    let stop = debuggee.stops().get(*index);
                    let at = debuggee.file_line(stop.line);
                    if at != *line {
                        self.notes.push(format!(
                            "{name}:{line}: the breakpoint stops at line {at}, at {}",
                            stop.name
                        ));
                    }
                    json!({"id": id, "verified": true, "line": at})
                }
                Err(message) => {
                    if ours {
                        self.notes.push(format!(
                            "{name}:{line}: the breakpoint takes no stop: {message}"
                        ));
                    }
                    json!({"id": id, "verified": false, "line": line, "message": message})
                }
            });
            kept.push(Breakpoint {
                id,
                bound: bound.ok(),
            });
        }
        if ours {
            self.breakpoints = kept;
        }
        breakpoints
    }

    /// Writes the console's lines about the breakpoints that the last binding moved or left
    /// unbound.
    fn notes(&mut self) {
        for note in std::mem::take(&mut self.notes) {
            self.out.console(&note);
        }
    }

    /// The stops that the breakpoints bind to.
    fn bound(&self) -> BTreeSet<usize> {
        (self.breakpoints.iter())
            .filter_map(|breakpoint| breakpoint.bound)
            .collect()
    }

    /// `configurationDone`: the session runs to its first stop.
    fn configuration_done(&mut self, request: &Request) {
        let Some(debuggee) = &self.debuggee else {
            self.out.fail(request, "the session has not launched");
            return;
        };
        let (index, reason) = debuggee.stops().entry(&self.bound(), self.stop_on_entry);
        self.out.respond(request, json!({}));
        self.stop_at(index, reason);
    }

    /// A step: moves the session by `step` and stops again. A move that stays where it is, as
    /// `continue` at the target's result, says that the session continued, then stops at the
    /// same frame, so the client's panes stay filled.
    fn step(&mut self, request: &Request, step: Move) {
        let (Some(debuggee), Some(from)) = (&self.debuggee, self.stop) else {
            self.out.fail(request, "the session is not stopped");
            return;
        };
        let (to, reason) = debuggee.stops().moved(from, step, &self.bound());
        match step {
            Move::Continue | Move::ReverseContinue => self
                .out
                .respond(request, json!({"allThreadsContinued": true})),
            _ => self.out.respond(request, json!({})),
        }
        if to == from {
            self.out.event(
                "continued",
                json!({"threadId": THREAD, "allThreadsContinued": true}),
            );
            self.stopped(reason);
            return;
        }
        self.stop_at(to, reason);
    }

    /// Runs the stop at `index`, shows it in the window, and says that the session stopped there.
    fn stop_at(&mut self, index: usize, reason: Reason) {
        let debuggee = self.debuggee.as_mut().expect("a loaded target");
        let (outcome, ran) = self.runtime.block_on(debuggee.run(index));
        let title = debuggee.title(index);
        if let Some(sink) = &self.sink {
            sink.send(Message::Title(title));
            sink.send(Message::Outcome(outcome));
            // The window reports the new result's chart, if it has one.
            self.chart = None;
        }
        if let Err(errors) = &ran.rows {
            for error in errors {
                self.out.console(error);
            }
        }
        self.stop = Some(index);
        self.ran = Some(ran);
        self.stopped(reason);
    }

    /// Says that the session stopped, which makes the client ask for its frame and its variables
    /// again, whose references start afresh.
    fn stopped(&mut self, reason: Reason) {
        self.vars.clear();
        let debuggee = self.debuggee.as_ref().expect("a loaded target");
        let stop = debuggee.stops().get(self.stop.expect("a stop"));
        let line = debuggee.file_line(stop.line);
        let description = format!("{} at line {line}", stop.name);
        let ids: Vec<i64> = (self.breakpoints.iter())
            .filter(|breakpoint| breakpoint.bound == self.stop)
            .map(|breakpoint| breakpoint.id)
            .collect();
        let mut body = json!({
            "reason": reason.name(),
            "description": description,
            "threadId": THREAD,
            "allThreadsStopped": true,
        });
        if reason == Reason::Breakpoint && !ids.is_empty() {
            body["hitBreakpointIds"] = json!(ids);
        }
        self.out.event("stopped", body);
    }

    /// `stackTrace`: one frame, the current stop's statement, at the stop's line.
    fn stack_trace(&mut self, request: &Request) {
        let (Some(debuggee), Some(index)) = (&self.debuggee, self.stop) else {
            self.out
                .respond(request, json!({"stackFrames": [], "totalFrames": 0}));
            return;
        };
        let stop = debuggee.stops().get(index);
        let frame = json!({
            "id": THREAD,
            "name": stop.name,
            "line": debuggee.file_line(stop.line),
            "column": 1,
            "source": {
                "name": debuggee.display_name(),
                "path": debuggee.file().display().to_string(),
            },
        });
        self.out
            .respond(request, json!({"stackFrames": [frame], "totalFrames": 1}));
    }

    /// The reference of `var` among the current stop's variables.
    fn reference(&mut self, var: Var) -> usize {
        self.vars.push(var);
        self.vars.len()
    }

    /// `scopes`: the stop's rows, as `Stage` or at its result `Result`, and `Script`.
    fn scopes(&mut self, request: &Request) {
        let (Some(debuggee), Some(index)) = (&self.debuggee, self.stop) else {
            self.out.respond(request, json!({"scopes": []}));
            return;
        };
        let place = debuggee.stops().get(index).place;
        let name = match place {
            Place::Stage(_) => "Stage",
            Place::Result => "Result",
        };
        let mut scopes = vec![(name, Var::Rows), ("Script", Var::Script)];
        // At a result, the state of the chart that the window shows, once it reports it.
        if place == Place::Result && self.chart.is_some() {
            scopes.extend([
                ("Selections", Var::Selections),
                ("Widgets", Var::Widgets),
                ("Scale Domains", Var::Scales),
                ("Modes", Var::Modes),
            ]);
        }
        let scopes: Vec<Value> = (scopes.into_iter())
            .map(|(name, var)| {
                let reference = self.reference(var);
                json!({"name": name, "variablesReference": reference, "expensive": false})
            })
            .collect();
        self.out.respond(request, json!({"scopes": scopes}));
    }

    /// `variables`: the children of a variable reference of the current stop.
    fn variables(&mut self, request: &Request) {
        let reference = request.arguments["variablesReference"]
            .as_u64()
            .unwrap_or(0) as usize;
        let Some(var) = reference
            .checked_sub(1)
            .and_then(|at| self.vars.get(at).copied())
        else {
            self.out.respond(request, json!({"variables": []}));
            return;
        };
        let variables = match self.children(var) {
            Ok(variables) => variables,
            Err(error) => vec![leaf("error", &error, None)],
        };
        self.out.respond(request, json!({"variables": variables}));
    }

    /// The children of a scope of the state of the chart that the window shows: none before the
    /// chart reports its state.
    fn chart_children(&self, var: Var) -> Vec<Value> {
        let Some(state) = &self.chart else {
            return Vec::new();
        };
        let pairs: Vec<(&str, String)> = match var {
            Var::Selections => (state.selections.iter())
                .map(|(name, clauses)| match clauses.is_empty() {
                    true => (name.as_str(), "clear".to_string()),
                    false => (name.as_str(), clauses.join("; ")),
                })
                .collect(),
            Var::Widgets => (state.widgets.iter())
                .map(|(key, value)| (key.as_str(), value.clone()))
                .collect(),
            Var::Scales => (state.scales.iter())
                .map(|scale| (scale.name.as_str(), scale.domain.clone()))
                .collect(),
            Var::Modes => (state.modes.iter())
                .map(|mode| (mode.as_str(), "on".to_string()))
                .collect(),
            _ => Vec::new(),
        };
        (pairs.into_iter())
            .map(|(name, value)| leaf(name, &value, None))
            .collect()
    }

    /// The children of `var`, as the protocol's variables.
    fn children(&mut self, var: Var) -> Result<Vec<Value>, String> {
        let Some(ran) = self.ran.as_mut() else {
            return Ok(Vec::new());
        };
        match var {
            Var::Selections | Var::Widgets | Var::Scales | Var::Modes => {
                Ok(self.chart_children(var))
            }
            Var::Script => {
                let variables = ran.variables.as_ref().map_err(|errors| errors.join("\n"))?;
                Ok((variables.iter())
                    .map(|(name, value)| {
                        let ty = googlesql_name(&value.data_type());
                        leaf(name, &scalar_text(value), Some(&ty))
                    })
                    .collect())
            }
            Var::Rows => {
                let rows = ran.rows.as_mut().map_err(|errors| errors.join("\n"))?;
                let columns = rows.schema().fields().len();
                let count = self.runtime.block_on(rows.count());
                let count = match count {
                    Ok(count) => format!("{count} rows"),
                    Err(error) => error,
                };
                let columns_ref = self.reference(Var::Columns);
                let rows_ref = self.reference(Var::AllRows);
                Ok(vec![
                    parent("columns", &format!("{columns} columns"), columns_ref),
                    parent("rows", &count, rows_ref),
                ])
            }
            Var::Columns => {
                let rows = ran.rows.as_ref().map_err(|errors| errors.join("\n"))?;
                Ok((rows.schema().fields().iter())
                    .map(|field| {
                        let ty = column_type(field);
                        leaf(field.name(), &ty, Some(&ty))
                    })
                    .collect())
            }
            Var::AllRows => {
                let rows = ran.rows.as_mut().map_err(|errors| errors.join("\n"))?;
                let count = self.runtime.block_on(rows.count())?;
                if count <= CHUNK {
                    return self.row_variables(0, count);
                }
                let mut chunks = Vec::new();
                for start in (0..count).step_by(CHUNK) {
                    let end = (start + CHUNK).min(count);
                    let reference = self.reference(Var::Chunk(start, end));
                    chunks.push(parent(&format!("rows {}-{end}", start + 1), "", reference));
                }
                Ok(chunks)
            }
            Var::Chunk(start, end) => self.row_variables(start, end),
            Var::Row(row) => {
                let rows = ran.rows.as_mut().map_err(|errors| errors.join("\n"))?;
                let batches = self.runtime.block_on(rows.rows(row, row + 1))?;
                let Some(batch) = batches.first() else {
                    return Ok(Vec::new());
                };
                Ok(fields(batch, 0)?
                    .into_iter()
                    .map(|(name, value, ty)| leaf(&name, &value, Some(&ty)))
                    .collect())
            }
        }
    }

    /// The rows `start..end` of the current stop, each a variable whose children are its fields.
    fn row_variables(&mut self, start: usize, end: usize) -> Result<Vec<Value>, String> {
        let ran = self.ran.as_mut().expect("a stop's run");
        let rows: &mut Rows = ran.rows.as_mut().map_err(|errors| errors.join("\n"))?;
        let batches = self.runtime.block_on(rows.rows(start, end))?;
        let mut variables = Vec::new();
        let mut row = start;
        for batch in &batches {
            for index in 0..batch.num_rows() {
                let summary: Vec<String> = (fields(batch, index)?.into_iter())
                    .map(|(name, value, _)| format!("{name}: {value}"))
                    .collect();
                let mut summary = summary.join(", ");
                if summary.chars().count() > SUMMARY {
                    summary = summary.chars().take(SUMMARY).collect::<String>() + "…";
                }
                let reference = self.reference(Var::Row(row));
                variables.push(parent(&(row + 1).to_string(), &summary, reference));
                row += 1;
            }
        }
        Ok(variables)
    }
}

/// The capabilities of the adapter.
fn capabilities() -> Value {
    json!({
        "supportsConfigurationDoneRequest": true,
        "supportsStepBack": true,
        "supportsTerminateRequest": true,
        "supportsEvaluateForHovers": false,
    })
}

/// A variable without children.
fn leaf(name: &str, value: &str, ty: Option<&str>) -> Value {
    let mut variable = json!({"name": name, "value": value, "variablesReference": 0});
    if let Some(ty) = ty {
        variable["type"] = json!(ty);
    }
    variable
}

/// A variable whose children are at `reference`.
fn parent(name: &str, value: &str, reference: usize) -> Value {
    json!({"name": name, "value": value, "variablesReference": reference})
}

/// The type of a column as GoogleSQL names it, or `CHART` for a column of charts.
fn column_type(field: &datafusion::arrow::datatypes::Field) -> String {
    match is_chart_field(field) {
        true => "CHART".to_string(),
        false => googlesql_name(field.data_type()),
    }
}

/// The fields of the row `row` of `batch`: each column's name, its value as text, and its type.
fn fields(batch: &RecordBatch, row: usize) -> Result<Vec<(String, String, String)>, String> {
    let options = FormatOptions::default().with_null("NULL");
    let schema = batch.schema();
    (schema.fields().iter().zip(batch.columns()))
        .map(|(field, column)| {
            let value = match (is_chart_field(field), column.is_null(row)) {
                (_, true) => "NULL".to_string(),
                (true, false) => "<chart>".to_string(),
                (false, false) => ArrayFormatter::try_new(column.as_ref(), &options)
                    .map_err(|error| error.to_string())?
                    .value(row)
                    .to_string(),
            };
            Ok((field.name().clone(), value, column_type(field)))
        })
        .collect()
}

/// A variable's value as text, `NULL` for a null.
fn scalar_text(value: &datafusion::common::ScalarValue) -> String {
    match value.is_null() {
        true => "NULL".to_string(),
        false => value.to_string(),
    }
}
