//! The adapter's side of a session: the requests of the Debug Adapter Protocol, answered in the
//! order they come, the stops that they move to, and what each stop shows: the window's rows, the
//! frame that the editor marks, and the scopes and variables of its panes.
//!
//! A session has one thread, `query`, with one frame, the current stop's statement. The client
//! sends `launch` with the target, as `view` takes it, and the adapter answers with `initialized`
//! once the target's project loads, so the breakpoints that follow bind to its stops. After
//! `configurationDone`, the session stops at its first stop, and each step stops again.

use std::collections::{BTreeSet, HashMap};
use std::io::Write;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

use avenger_chart::proto::encoding::Kind;
use avenger_chart::results::{is_chart_field, QueryResult};
use avenger_sql::types::googlesql_name;
use datafusion::arrow::{
    array::RecordBatch,
    datatypes::SchemaRef,
    util::display::{ArrayFormatter, FormatOptions},
};
use datafusion::prelude::SessionContext;
use serde_json::{json, Value};

use super::debuggee::{Debuggee, Ran, Rows};
use super::protocol::{Output, Request};
use super::stops::{Move, Place, Reason, StopKey};
use avenger_results_egui::{channel_name, plot_name, ChartState};

use avenger_sql_project::watch::{Burst, Watch};

use crate::view::{canonical, Message, Outcome, Sink, WindowEvent};

/// The id of the session's thread, and of its frame.
const THREAD: i64 = 1;

/// The rows of a stop that a child of its rows holds, when it has more: its rows are chunks of
/// this many, since a client may not page a variable's children.
const CHUNK: usize = 100;

/// The longest value of a row, as a summary of its fields.
const SUMMARY: usize = 120;

/// The rows of a console answer that the console prints as a grid.
const GRID_ROWS: usize = 10;

/// The longest value of a cell of the console's grid.
const GRID_CELL: usize = 40;

/// What a variable reference holds, among a stop's variables.
#[derive(Clone, Copy, Debug)]
enum Var {
    /// The scope of rows, a stop's or a console result's: its columns and its rows.
    Rows(Source),
    Columns(Source),
    AllRows(Source),
    /// The rows `start..end`.
    Chunk(Source, usize, usize),
    /// A row's fields.
    Row(Source, usize),
    /// The layers of the chart in a row's column, by the row and the column's index.
    Chart(Source, usize, usize),
    /// The script's variables.
    Script,
    /// The state of the chart at a result stop: its selections, widgets, scales, and modes.
    Selections,
    Widgets,
    Scales,
    Modes,
}

/// Whose rows a variable holds: the stop's, or rows that the stop keeps, by their order: a result
/// of the console's, or the table of a chart's layer.
#[derive(Clone, Copy, Debug)]
enum Source {
    Stop,
    Kept(usize),
}

/// Rows that a stop keeps: a result of the console's, or the table of a chart's layer, with the
/// channels that encode each of the layer's columns, by the column's name, as `x, color`.
struct Kept {
    rows: Rows,
    channels: HashMap<String, String>,
}

/// The rows of `source`: the stop's, of `ran`, or rows that the stop keeps, of `kept`.
fn rows_of<'a>(
    ran: &'a mut Option<Ran>,
    kept: &'a mut [Kept],
    source: Source,
) -> Result<&'a mut Rows, String> {
    match source {
        Source::Stop => {
            let ran = ran.as_mut().ok_or("the session is not stopped")?;
            ran.rows.as_mut().map_err(|errors| errors.join("\n"))
        }
        Source::Kept(index) => (kept.get_mut(index))
            .map(|kept| &mut kept.rows)
            .ok_or_else(|| "no such result".into()),
    }
}

/// A breakpoint of the target's file: its id, its line in the file, and the stop that it binds
/// to, if any.
struct Breakpoint {
    id: i64,
    line: usize,
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
    /// The rows that the current stop keeps: the console's results, and the tables of the layers
    /// of the charts that the reader opened.
    kept: Vec<Kept>,
    /// The watch of the project's files, which the session follows as `view` does.
    watch: Option<Watch>,
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
            kept: Vec::new(),
            watch: None,
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
                    self.watch_files();
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
            "setVariable" => self.set_variable(request),
            "evaluate" => self.evaluate(request),
            "restart" => self.restart(request),
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
        // The session follows the project's files as a `view` window does.
        self.watch = Watch::new(&debuggee.watched_directory(), || {}).ok();
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
        let ids = lines.iter().map(|_| self.next_id()).collect();
        let breakpoints = self.bind(&path, &lines, ids);
        self.out
            .respond(request, json!({"breakpoints": breakpoints}));
        self.notes();
    }

    fn next_id(&mut self) -> i64 {
        self.ids += 1;
        self.ids
    }

    /// Binds the breakpoints at `lines` of the file `path`, with their ids `ids`, and gives them as
    /// the protocol's breakpoints. The breakpoints of the target's file replace its breakpoints
    /// before.
    fn bind(&mut self, path: &std::path::Path, lines: &[usize], ids: Vec<i64>) -> Vec<Value> {
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
                true => (debuggee.script_line(*line))
                    .and_then(|script_line| debuggee.stops().bind(script_line)),
            };
            // A client may draw a breakpoint at its own line and keep no message, as Zed does,
            // so the console says where a breakpoint of the target's file stops, or why not.
            breakpoints.push(match &bound {
                Ok(index) => {
                    let stop = debuggee.stops().get(*index);
                    let at = stop.line;
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
                line: *line,
            });
        }
        if ours {
            self.breakpoints = kept;
        }
        breakpoints
    }

    /// Binds the breakpoints of the target's file again, by their lines, after its stops changed,
    /// and tells the client of each.
    fn rebind(&mut self) {
        let Some(debuggee) = &self.debuggee else {
            return;
        };
        let path = debuggee.file().to_path_buf();
        let (lines, ids): (Vec<usize>, Vec<i64>) = (self.breakpoints.iter())
            .map(|breakpoint| (breakpoint.line, breakpoint.id))
            .unzip();
        for breakpoint in self.bind(&path, &lines, ids) {
            self.out.event(
                "breakpoint",
                json!({"reason": "changed", "breakpoint": breakpoint}),
            );
        }
        self.notes();
    }

    /// Follows the changes of the project's files once a burst of them ends: the session runs its
    /// stop again, found by its key, as `view` runs its chart again.
    fn watch_files(&mut self) {
        let Some(watch) = self.watch.as_mut() else {
            return;
        };
        let Burst::Ended(paths) = watch.changed() else {
            return;
        };
        let paths: Vec<PathBuf> = paths.iter().map(|path| canonical(path)).collect();
        self.files_changed(&paths);
    }

    /// What the changes of the files at `paths` do to the session: a restyle of the window, and
    /// when they change what the target runs, a run of the stop again, at its key, or else at the
    /// target's result, with a line in the console.
    fn files_changed(&mut self, paths: &[PathBuf]) {
        let Some(debuggee) = self.debuggee.as_mut() else {
            return;
        };
        let key = self.stop.map(|index| debuggee.stops().key(index));
        let outcomes = match self.runtime.block_on(debuggee.changed(paths)) {
            Ok(outcomes) => outcomes,
            Err(error) => {
                self.out.console(&error);
                return;
            }
        };
        let mut runs = false;
        for outcome in outcomes {
            match outcome {
                Outcome::Restyled(stylesheet) => {
                    if let Some(sink) = &self.sink {
                        sink.send(Message::Outcome(Outcome::Restyled(stylesheet)));
                    }
                }
                _ => runs = true,
            }
        }
        if !runs {
            return;
        }
        let names: Vec<String> = (paths.iter())
            .filter_map(|path| path.file_name())
            .map(|name| name.to_string_lossy().to_string())
            .collect();
        self.out.console(&format!("{} changed", names.join(", ")));
        self.rebind();
        self.go_to(key);
    }

    /// Stops again at the stop that `key` finds, or at the target's result when none does, as
    /// after the script changed.
    fn go_to(&mut self, key: Option<StopKey>) {
        let Some(debuggee) = &self.debuggee else {
            return;
        };
        let Some(key) = key else {
            return;
        };
        let stops = debuggee.stops();
        let index = stops.find(&key).unwrap_or(stops.len() - 1);
        self.stop_at(index, Reason::Pause);
    }

    /// `restart`: opens the target's project again and stops where the session was.
    fn restart(&mut self, request: &Request) {
        let Some(debuggee) = self.debuggee.as_mut() else {
            self.out.fail(request, "the session has not launched");
            return;
        };
        let key = self.stop.map(|index| debuggee.stops().key(index));
        if let Err(error) = self.runtime.block_on(debuggee.reopen()) {
            self.out.fail(request, &error);
            return;
        }
        self.out.respond(request, json!({}));
        self.rebind();
        self.go_to(key);
    }

    /// `setVariable`: gives a variable of the `Script` scope a value of its type, and runs the
    /// stop again with it.
    fn set_variable(&mut self, request: &Request) {
        let args = &request.arguments;
        let reference = args["variablesReference"].as_u64().unwrap_or(0) as usize;
        let var = reference
            .checked_sub(1)
            .and_then(|at| self.vars.get(at).copied());
        if !matches!(var, Some(Var::Script)) {
            self.out.fail(
                request,
                "only the variables of the Script scope take values",
            );
            return;
        }
        let (name, value) = (
            args["name"].as_str().unwrap_or_default(),
            args["value"].as_str(),
        );
        let ty = (self.ran.as_ref())
            .and_then(|ran| ran.variables.as_ref().ok())
            .and_then(|variables| variables.iter().find(|(other, _)| other == name))
            .map(|(_, value)| value.data_type());
        let (Some(debuggee), Some(index), Some(ty), Some(value)) =
            (self.debuggee.as_mut(), self.stop, ty, value)
        else {
            self.out.fail(
                request,
                &format!("the script has no variable `{name}` at this stop"),
            );
            return;
        };
        match debuggee.set_variable(name, value, &ty) {
            Ok(scalar) => {
                let body = json!({"value": scalar_text(&scalar), "type": googlesql_name(&ty)});
                self.out.respond(request, body);
                self.stop_at(index, Reason::Pause);
            }
            Err(error) => self.out.fail(request, &error),
        }
    }

    /// `evaluate`, in the console: runs pipe operators after the stop's rows, or a query that
    /// starts with `FROM`, shows the rows in the window, and answers with their size and their
    /// rows as a variable.
    fn evaluate(&mut self, request: &Request) {
        let args = &request.arguments;
        if args["context"].as_str() == Some("hover") {
            self.out
                .fail(request, "avenger dap evaluates in the debug console only");
            return;
        }
        let expression = args["expression"].as_str().unwrap_or_default().to_string();
        let (Some(debuggee), Some(index)) = (self.debuggee.as_mut(), self.stop) else {
            self.out.fail(request, "the session is not stopped");
            return;
        };
        let planned = match self.runtime.block_on(debuggee.evaluate(index, &expression)) {
            Ok(planned) => planned,
            Err(error) => {
                self.out.fail(request, &error);
                return;
            }
        };
        if let Some(sink) = &self.sink {
            let outcome = self.runtime.block_on(debuggee.outcome(planned.clone()));
            let name = debuggee.display_name();
            sink.send(Message::Title(format!("avenger dap {name}: {expression}")));
            sink.send(Message::Outcome(outcome));
        }
        let mut rows = Rows::new(planned.into_frame());
        let columns = rows.schema().fields().len();
        let count = self.runtime.block_on(rows.count());
        let summary = match &count {
            Ok(count) => format!("{count} rows, {columns} columns"),
            Err(error) => error.clone(),
        };
        // The first rows, as a grid under the answer, since the console has the room for a table
        // that the variables' tree does not.
        let first = self.runtime.block_on(rows.rows(0, GRID_ROWS));
        let grid = first.and_then(|batches| grid(&rows.schema(), &batches));
        self.kept.push(Kept {
            rows,
            channels: HashMap::new(),
        });
        let reference = self.reference(Var::AllRows(Source::Kept(self.kept.len() - 1)));
        self.out.respond(
            request,
            json!({"result": summary, "variablesReference": reference}),
        );
        if let Ok(grid) = grid {
            self.out.console(&grid);
        }
        if let Ok(count) = count {
            if count > GRID_ROWS {
                self.out
                    .console(&format!("… {} more rows", count - GRID_ROWS));
            }
        }
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
        self.kept.clear();
        let debuggee = self.debuggee.as_ref().expect("a loaded target");
        let stop = debuggee.stops().get(self.stop.expect("a stop"));
        let line = stop.line;
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
            "line": stop.line,
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
        let mut scopes = vec![(name, Var::Rows(Source::Stop)), ("Script", Var::Script)];
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
        let mut variables = match self.children(var) {
            Ok(variables) => variables,
            Err(error) => vec![leaf("error", &error, None)],
        };
        // Only the script's variables take values, so a client may offer to edit only them.
        if !matches!(var, Var::Script) {
            for variable in &mut variables {
                variable["presentationHint"] = json!({"attributes": ["readOnly"]});
            }
        }
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
        match var {
            Var::Selections | Var::Widgets | Var::Scales | Var::Modes => {
                Ok(self.chart_children(var))
            }
            Var::Script => {
                let Some(ran) = self.ran.as_ref() else {
                    return Ok(Vec::new());
                };
                let variables = ran.variables.as_ref().map_err(|errors| errors.join("\n"))?;
                Ok((variables.iter())
                    .map(|(name, value)| {
                        let ty = googlesql_name(&value.data_type());
                        leaf(name, &scalar_text(value), Some(&ty))
                    })
                    .collect())
            }
            Var::Rows(source) => {
                let rows = rows_of(&mut self.ran, &mut self.kept, source)?;
                let columns = rows.schema().fields().len();
                let count = match self.runtime.block_on(rows.count()) {
                    Ok(count) => format!("{count} rows"),
                    Err(error) => error,
                };
                let columns_ref = self.reference(Var::Columns(source));
                let rows_ref = self.reference(Var::AllRows(source));
                Ok(vec![
                    parent("columns", &format!("{columns} columns"), columns_ref),
                    parent("rows", &count, rows_ref),
                ])
            }
            Var::Columns(source) => {
                let channels = match source {
                    Source::Kept(index) => self.kept.get(index).map(|kept| kept.channels.clone()),
                    Source::Stop => None,
                };
                let channels = channels.unwrap_or_default();
                let rows = rows_of(&mut self.ran, &mut self.kept, source)?;
                Ok((rows.schema().fields().iter())
                    .map(|field| {
                        let ty = column_type(field);
                        // A column of a layer's table names the channels that encode it.
                        let name = match channels.get(field.name()) {
                            Some(channels) => format!("{channels}: {}", field.name()),
                            None => field.name().clone(),
                        };
                        leaf(&name, &ty, Some(&ty))
                    })
                    .collect())
            }
            Var::AllRows(source) => {
                let rows = rows_of(&mut self.ran, &mut self.kept, source)?;
                let count = self.runtime.block_on(rows.count())?;
                if count <= CHUNK {
                    return self.row_variables(source, 0, count);
                }
                let mut chunks = Vec::new();
                for start in (0..count).step_by(CHUNK) {
                    let end = (start + CHUNK).min(count);
                    let reference = self.reference(Var::Chunk(source, start, end));
                    chunks.push(parent(&format!("rows {}-{end}", start + 1), "", reference));
                }
                Ok(chunks)
            }
            Var::Chunk(source, start, end) => self.row_variables(source, start, end),
            Var::Row(source, row) => {
                let rows = rows_of(&mut self.ran, &mut self.kept, source)?;
                let batches = self.runtime.block_on(rows.rows(row, row + 1))?;
                let Some(batch) = batches.first() else {
                    return Ok(Vec::new());
                };
                let charts: Vec<bool> = (batch.schema().fields().iter())
                    .map(|field| is_chart_field(field))
                    .collect();
                let fields = fields(batch, 0)?.into_iter().enumerate();
                Ok(fields
                    .map(|(column, (name, value, ty))| {
                        // A chart opens into its layers.
                        if charts[column] && !batch.column(column).is_null(0) {
                            let reference = self.reference(Var::Chart(source, row, column));
                            let mut variable = parent(&name, &value, reference);
                            variable["type"] = json!(ty);
                            return variable;
                        }
                        leaf(&name, &value, Some(&ty))
                    })
                    .collect())
            }
            Var::Chart(source, row, column) => {
                let rows = rows_of(&mut self.ran, &mut self.kept, source)?;
                let batches = self.runtime.block_on(rows.rows(row, row + 1))?;
                let batch = batches.into_iter().next().ok_or("the row is gone")?;
                let mut variables = Vec::new();
                for (name, table, channels) in chart_layers(&batch, column)? {
                    let count = table.num_rows();
                    let frame = (SessionContext::new().read_batch(table))
                        .map_err(|error| error.to_string())?;
                    self.kept.push(Kept {
                        rows: Rows::new(frame),
                        channels,
                    });
                    let reference = self.reference(Var::Rows(Source::Kept(self.kept.len() - 1)));
                    variables.push(parent(&name, &format!("{count} rows"), reference));
                }
                Ok(variables)
            }
        }
    }

    /// The rows `start..end` of `source`, each a variable whose children are its fields.
    fn row_variables(
        &mut self,
        source: Source,
        start: usize,
        end: usize,
    ) -> Result<Vec<Value>, String> {
        let rows = rows_of(&mut self.ran, &mut self.kept, source)?;
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
                let reference = self.reference(Var::Row(source, row));
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
        "supportsSetVariable": true,
        "supportsRestartRequest": true,
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

/// A layer of a chart: its name, its table, the rows that its mark draws, and the channels that
/// encode each column of the table, by the column's name, as `x, color`.
type ChartLayer = (String, RecordBatch, HashMap<String, String>);

/// The layers of the chart in the column `column` of the one row of `batch`. A layer is named by
/// its id, or by its place among its plot's layers, with its mark, as `layer 1 (bar)`, and after
/// its plot's name when the chart has several plots, as `plot 2 › layer 1 (bar)` or
/// `plot 2 (Origin = Europe) › layer 1 (bar)` for a facet's cell.
fn chart_layers(batch: &RecordBatch, column: usize) -> Result<Vec<ChartLayer>, String> {
    let result =
        QueryResult::new(batch.schema(), vec![batch.clone()]).map_err(|error| error.to_string())?;
    let chart = result.chart(0, column).ok_or("the cell holds no chart")?;
    let resolved = chart.resolve();
    let tree = &resolved.tree;
    Ok((tree.layers.iter())
        .map(|layer| {
            let mark = layer.mark.name();
            let name = match layer.identity.given_id() {
                Some(id) => format!("{id} ({mark})"),
                None => format!("layer {} ({mark})", layer.position + 1),
            };
            // A plot is named as the chart's state names it: by its id, or by its place among the
            // chart's plots, with its facet values, as `plot 2 (Origin = Europe)`.
            let plot = plot_name(layer.plot, &tree.plots[layer.plot].identity);
            let name = match tree.plots.len() > 1 {
                true => format!("{plot} › {name}"),
                false => name,
            };
            let mut channels: HashMap<String, Vec<String>> = HashMap::new();
            for channel in layer.mark.channels() {
                let field = match layer.mark.encoding(*channel).and_then(|e| e.kind.as_ref()) {
                    Some(Kind::ScaledField(field) | Kind::DirectField(field)) => field,
                    _ => continue,
                };
                (channels.entry(field.clone()).or_default()).push(channel_name(*channel));
            }
            let channels = (channels.into_iter())
                .map(|(field, channels)| (field, channels.join(", ")))
                .collect();
            (name, layer.table.clone(), channels)
        })
        .collect())
}

/// The rows `batches`, of the schema `schema`, as a grid of text: a header of the columns' names,
/// and a line for each row, each value cut to [`GRID_CELL`] characters.
fn grid(schema: &SchemaRef, batches: &[RecordBatch]) -> Result<String, String> {
    let cut = |text: String| match text.chars().count() > GRID_CELL {
        true => text.chars().take(GRID_CELL - 1).collect::<String>() + "…",
        false => text,
    };
    let header: Vec<String> = (schema.fields().iter())
        .map(|field| cut(field.name().clone()))
        .collect();
    let mut lines = vec![header];
    for batch in batches {
        for row in 0..batch.num_rows() {
            let values = fields(batch, row)?.into_iter();
            lines.push(values.map(|(_, value, _)| cut(value)).collect());
        }
    }
    let widths: Vec<usize> = (0..schema.fields().len())
        .map(|column| {
            (lines.iter())
                .map(|line| line[column].chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    let rule = (widths.iter())
        .map(|width| "-".repeat(width + 2))
        .collect::<Vec<_>>();
    let rule = format!("+{}+", rule.join("+"));
    let line = |cells: &[String]| {
        let cells: Vec<String> = (cells.iter().zip(&widths))
            .map(|(cell, width)| format!(" {cell}{} ", " ".repeat(width - cell.chars().count())))
            .collect();
        format!("|{}|", cells.join("|"))
    };
    let mut text = vec![rule.clone(), line(&lines[0]), rule.clone()];
    text.extend(lines[1..].iter().map(|cells| line(cells)));
    text.push(rule);
    Ok(text.join("\n"))
}

/// A variable's value as text, `NULL` for a null.
fn scalar_text(value: &datafusion::common::ScalarValue) -> String {
    match value.is_null() {
        true => "NULL".to_string(),
        false => value.to_string(),
    }
}
