//! The stops of a debugging session, apart from the protocol and the window: the stages of a
//! script's statements through the target's statement, the binding of breakpoints to them, and
//! the moves between them.
//!
//! The session walks the script in order: each statement with rows through the target's
//! statement, and in each statement's query, the rows that enter each of its top-level operators,
//! then the statement's result. A statement without rows, such as `DECLARE`, takes no stop. In pipe
//! syntax every prefix of a query is a query, so a stop runs its prefix, and stepping back costs
//! what stepping forward does.

use std::collections::BTreeSet;

use crate::statement::{spans, staged, Key, Script, Staged};

/// Where a stop is in its statement: the rows that enter its query's operator of an index, or
/// the statement's result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Place {
    Stage(usize),
    Result,
}

/// A stop: its statement, by its index among the session's statements, its place in it, the line
/// of the script's text that it shows, from 1, and its name, as `|> WHERE`, `result`, or
/// `table heavy_by_origin`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Stop {
    pub(crate) statement: usize,
    pub(crate) place: Place,
    pub(crate) line: usize,
    pub(crate) name: String,
}

/// A move from a stop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Move {
    /// The next stop, as `next` and `stepIn` move.
    Next,
    /// The current statement's result.
    Out,
    /// The next bound stop, or the target's result.
    Continue,
    /// The stop before.
    Back,
    /// The bound stop before, or the first stop.
    ReverseContinue,
}

/// Why the session stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Reason {
    Entry,
    Breakpoint,
    Step,
}

impl Reason {
    /// The reason as the protocol's `stopped` event names it.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Reason::Entry => "entry",
            Reason::Breakpoint => "breakpoint",
            Reason::Step => "step",
        }
    }
}

/// The stops of a script: its statements with rows through the target's statement, and their
/// stops in order.
#[derive(Clone, Debug)]
pub(crate) struct Stops {
    statements: Vec<Staged>,
    stops: Vec<Stop>,
    /// The first and last lines of each of the script's statements, with rows or not, for the
    /// reasons of breakpoints that bind to none.
    spans: Vec<(usize, usize)>,
}

impl Stops {
    /// The stops of the script `text`, of the kind `script`, through its statement that starts at
    /// the offset `target`, or through its value, its last query, without one. `None` when the
    /// script has no such statement.
    pub(crate) fn new(text: &str, script: Script, target: Option<usize>) -> Option<Stops> {
        let mut statements = staged(text, script);
        let end = match target {
            Some(start) => (statements.iter()).position(|at| at.statement.start == start)?,
            None => (statements.iter()).position(|at| at.statement.key == Key::Value)?,
        };
        statements.truncate(end + 1);
        let mut stops = Vec::new();
        for (index, at) in statements.iter().enumerate() {
            for (place, operator) in at.operators.iter().enumerate() {
                stops.push(Stop {
                    statement: index,
                    place: Place::Stage(place),
                    line: operator.line,
                    name: operator.name.clone(),
                });
            }
            stops.push(Stop {
                statement: index,
                place: Place::Result,
                line: at.statement.line,
                name: at.statement.shows.describe(),
            });
        }
        Some(Stops {
            statements,
            stops,
            spans: spans(text, script),
        })
    }

    pub(crate) fn len(&self) -> usize {
        self.stops.len()
    }

    pub(crate) fn get(&self, index: usize) -> &Stop {
        &self.stops[index]
    }

    /// The text of a chart script that runs the stop at `index` of the script `text`: the rows
    /// that enter its operator, or its statement's result.
    pub(crate) fn text(&self, index: usize, text: &str) -> String {
        let stop = &self.stops[index];
        let at = &self.statements[stop.statement];
        match stop.place {
            Place::Stage(place) => at.stage_text(text, &at.operators[place]),
            Place::Result => at.statement.text(text),
        }
    }

    /// The stop that a breakpoint at the line `line` of the script's text binds to, from 1, with
    /// the stop's line, or why it binds to none. A line with a top-level `|>` binds to the stage
    /// of its first `|>`, the first line of a statement with rows binds to its result, and another
    /// line of a statement binds to the next line of the statement that binds.
    pub(crate) fn bind(&self, line: usize) -> Result<usize, String> {
        let holds = |at: &Staged| at.statement.line <= line && line <= at.last;
        let Some(statement) = self.statements.iter().position(holds) else {
            let last = self.statements.last().map(|at| at.last).unwrap_or(0);
            let span = self
                .spans
                .iter()
                .find(|(first, last)| *first <= line && line <= *last);
            return Err(match span {
                Some(_) if line > last => {
                    format!("the session ends at its target's statement, which ends at line {last}")
                }
                Some((first, _)) => format!("the statement at line {first} has no rows"),
                None => "no statement holds the line".to_string(),
            });
        };
        let in_statement = |stop: &&Stop| stop.statement == statement;
        let stops: Vec<(usize, &Stop)> = (self.stops.iter().enumerate())
            .filter(|(_, stop)| in_statement(stop))
            .collect();
        let stage = |(_, stop): &&(usize, &Stop)| matches!(stop.place, Place::Stage(_));
        if let Some((index, _)) = (stops.iter().filter(stage)).find(|(_, stop)| stop.line == line) {
            return Ok(*index);
        }
        if line == self.statements[statement].statement.line {
            let result = stops.iter().find(|(_, stop)| stop.place == Place::Result);
            return Ok(result.expect("a statement's result").0);
        }
        match (stops.iter().filter(stage)).find(|(_, stop)| stop.line > line) {
            Some((index, _)) => Ok(*index),
            None => Err(format!(
                "no stage of the statement at line {} starts after the line; set the breakpoint on a \
                 `|>` line or on the statement's first line",
                self.statements[statement].statement.line
            )),
        }
    }

    /// The first stop of a session, and why it stops there: its first stop on entry, its first
    /// bound stop, or else its target's result.
    pub(crate) fn entry(&self, bound: &BTreeSet<usize>, stop_on_entry: bool) -> (usize, Reason) {
        if stop_on_entry {
            return (0, Reason::Entry);
        }
        match bound.iter().next() {
            Some(index) => (*index, Reason::Breakpoint),
            None => (self.len() - 1, Reason::Step),
        }
    }

    /// The stop that `step` moves to from the stop at `from`, with the bound stops `bound`, and
    /// why it stops there. A move past the target's result stays there.
    pub(crate) fn moved(
        &self,
        from: usize,
        step: Move,
        bound: &BTreeSet<usize>,
    ) -> (usize, Reason) {
        let last = self.len() - 1;
        let reason = |index: usize, step_reason: Reason| match bound.contains(&index) {
            true => Reason::Breakpoint,
            false => step_reason,
        };
        match step {
            Move::Next => {
                let to = (from + 1).min(last);
                (to, reason(to, Reason::Step))
            }
            Move::Out => {
                let statement = self.stops[from].statement;
                let to = (from..=last)
                    .find(|index| {
                        let stop = &self.stops[*index];
                        stop.statement == statement && stop.place == Place::Result
                    })
                    .expect("a statement's result");
                (to, reason(to, Reason::Step))
            }
            Move::Continue => match bound.range(from + 1..).next() {
                Some(to) => (*to, Reason::Breakpoint),
                None => (last, reason(last, Reason::Step)),
            },
            Move::Back => {
                let to = from.saturating_sub(1);
                (to, reason(to, Reason::Step))
            }
            Move::ReverseContinue => match bound.range(..from).next_back() {
                Some(to) => (*to, Reason::Breakpoint),
                None => (0, reason(0, Reason::Step)),
            },
        }
    }
}
