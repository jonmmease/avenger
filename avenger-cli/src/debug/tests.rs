//! The stops of a session without a window or a run: the stages of a script's statements, the
//! binding of breakpoints, and the moves between stops.

use std::collections::BTreeSet;

use super::stops::{Move, Place, Reason, Stops};
use crate::statement::Script;

/// A script with a statement without rows, a temporary table, an earlier query on one line, and
/// the chart's query, whose last operator spans lines.
const SCRIPT: &str = "\
DECLARE min_weight INT64 DEFAULT 3000;
CREATE TEMP TABLE heavy AS (
    FROM vega_datasets.cars
    -- the heavy cars
    |> WHERE Weight_in_lbs > min_weight
);
FROM heavy |> LIMIT 3;
-- the chart
FROM heavy
|> AGGREGATE count(*) AS cars GROUP BY Origin
|> AGGREGATE mark_bar(
    x => Origin,
    y => cars,
   ) AS chart
";

fn stops() -> Stops {
    Stops::new(SCRIPT, Script::Chart, None).expect("the script's value")
}

/// The name and the line of each stop.
fn names(stops: &Stops) -> Vec<(String, usize)> {
    (0..stops.len())
        .map(|index| (stops.get(index).name.clone(), stops.get(index).line))
        .collect()
}

#[test]
fn a_session_stops_at_each_stage_and_each_result_through_its_target() {
    let stops = stops();
    let expected = [
        ("|> WHERE", 5),
        ("table heavy", 2),
        ("|> LIMIT", 7),
        ("query 1", 7),
        ("|> AGGREGATE", 10),
        ("|> AGGREGATE", 11),
        ("result", 9),
    ];
    let expected: Vec<(String, usize)> = (expected.iter())
        .map(|(name, line)| (name.to_string(), *line))
        .collect();
    assert_eq!(names(&stops), expected);
    // A target's statement ends the session: the earlier query's.
    let earlier = SCRIPT.find("FROM heavy |>").unwrap();
    let through = Stops::new(SCRIPT, Script::Chart, Some(earlier)).unwrap();
    assert_eq!(names(&through), expected[..4].to_vec());
    assert_eq!(through.get(3).place, Place::Result);
}

#[test]
fn a_stage_runs_the_text_before_its_pipe_at_the_same_offsets() {
    let stops = stops();
    // Inside a `CREATE TEMP TABLE`, the statement's head becomes spaces, and the comment before
    // the `|>` is cut.
    let text = stops.text(0, SCRIPT);
    let cut = SCRIPT.find("vega_datasets.cars").unwrap() + "vega_datasets.cars".len();
    assert_eq!(text.len(), cut);
    assert!(text.ends_with("\n    FROM vega_datasets.cars"));
    let head = &text[SCRIPT.find("CREATE").unwrap()..SCRIPT.find("FROM vega").unwrap()];
    assert!(head.chars().all(char::is_whitespace), "{head:?}");
    assert_eq!(head.matches('\n').count(), 1);
    assert!(text.starts_with("DECLARE min_weight INT64 DEFAULT 3000;\n"));
    // The table of a statement, the earlier query's stage on its own line, and the value's.
    assert!(stops.text(1, SCRIPT).ends_with(");\nFROM heavy\n"));
    assert!(stops.text(2, SCRIPT).ends_with(";\nFROM heavy"));
    assert_eq!(stops.text(6, SCRIPT), SCRIPT);
    let stage = stops.text(5, SCRIPT);
    assert!(stage.ends_with("GROUP BY Origin"), "{stage}");
}

#[test]
fn breakpoints_bind_to_stages_results_and_the_next_stage_line() {
    let stops = stops();
    // A `|>` line, the stage of its operator, and the first line of a statement, its result.
    assert_eq!(stops.bind(5), Ok(0));
    assert_eq!(stops.bind(2), Ok(1));
    assert_eq!(stops.bind(9), Ok(6));
    assert_eq!(stops.bind(10), Ok(4));
    // A line inside a statement binds to the next `|>` line.
    assert_eq!(stops.bind(3), Ok(0));
    assert_eq!(stops.bind(4), Ok(0));
    // A one-line statement's `|>` wins over its result.
    assert_eq!(stops.bind(7), Ok(2));
    // A line with no stage after it in its statement, a statement without rows, and a line in no
    // statement bind to none, with their reasons.
    let unbound = |line| stops.bind(line).unwrap_err();
    assert!(unbound(6).contains("no stage of the statement at line 2"));
    assert!(unbound(12).contains("no stage of the statement at line 9"));
    assert_eq!(unbound(1), "the statement at line 1 has no rows");
    assert_eq!(unbound(8), "no statement holds the line");
    // Past the target's statement.
    let earlier = SCRIPT.find("FROM heavy |>").unwrap();
    let through = Stops::new(SCRIPT, Script::Chart, Some(earlier)).unwrap();
    assert!(through.bind(10).unwrap_err().contains("the session ends"));
}

#[test]
fn a_session_enters_and_moves_between_its_stops() {
    let stops = stops();
    let none = BTreeSet::new();
    let bound = BTreeSet::from([2, 5]);
    assert_eq!(stops.entry(&none, true), (0, Reason::Entry));
    assert_eq!(stops.entry(&bound, false), (2, Reason::Breakpoint));
    assert_eq!(stops.entry(&none, false), (6, Reason::Step));
    // `next` and `stepOut`, from a stage and past the last statement.
    assert_eq!(stops.moved(0, Move::Next, &none), (1, Reason::Step));
    assert_eq!(stops.moved(1, Move::Next, &bound), (2, Reason::Breakpoint));
    assert_eq!(stops.moved(6, Move::Next, &none), (6, Reason::Step));
    assert_eq!(stops.moved(4, Move::Out, &none), (6, Reason::Step));
    assert_eq!(stops.moved(6, Move::Out, &none), (6, Reason::Step));
    // `continue` to the next breakpoint, or the target's result.
    assert_eq!(
        stops.moved(0, Move::Continue, &bound),
        (2, Reason::Breakpoint)
    );
    assert_eq!(stops.moved(5, Move::Continue, &bound), (6, Reason::Step));
    // Back, and back to a breakpoint or the first stop.
    assert_eq!(stops.moved(6, Move::Back, &none), (5, Reason::Step));
    assert_eq!(stops.moved(0, Move::Back, &none), (0, Reason::Step));
    let reverse = Move::ReverseContinue;
    assert_eq!(stops.moved(6, reverse, &bound), (5, Reason::Breakpoint));
    assert_eq!(stops.moved(2, reverse, &bound), (0, Reason::Step));
}

/// A writer that keeps what the adapter writes, for the test to read.
#[derive(Clone, Default)]
struct Written(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for Written {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Written {
    /// The messages written since the last call, as JSON.
    fn take(&self) -> Vec<serde_json::Value> {
        let bytes = std::mem::take(&mut *self.0.lock().unwrap());
        let text = String::from_utf8(bytes).unwrap();
        (text
            .split("Content-Length: ")
            .filter(|part| !part.is_empty()))
        .map(|part| serde_json::from_str(part.split_once("\r\n\r\n").unwrap().1).unwrap())
        .collect()
    }

    /// The events of the messages written since the last call, by name.
    fn events(&self) -> Vec<String> {
        (self.take().into_iter())
            .filter_map(|message| message["event"].as_str().map(str::to_string))
            .collect()
    }
}

/// An adapter at the result stop of the fixture's chart, without a window, whose client supports
/// `invalidated` or not, and what it writes.
fn at_result(invalidated: bool) -> (super::adapter::Adapter<Written>, Written) {
    let written = Written::default();
    let mut adapter = super::adapter::Adapter::new(written.clone(), None).unwrap();
    let request =
        |seq: i64, command: &str, arguments: serde_json::Value| super::protocol::Request {
            seq,
            command: command.to_string(),
            arguments,
        };
    let chart = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../avenger-sql-project/tests/projects/vega/charts/origins.av.sql");
    let initialize = serde_json::json!({"supportsInvalidatedEvent": invalidated});
    adapter.answer(&request(1, "initialize", initialize));
    let launch = serde_json::json!({"target": chart.to_str().unwrap()});
    adapter.answer(&request(2, "launch", launch));
    adapter.answer(&request(3, "configurationDone", serde_json::json!({})));
    let stopped: Vec<serde_json::Value> = (written.take().into_iter())
        .filter(|message| message["event"] == "stopped")
        .collect();
    assert_eq!(stopped[0]["body"]["description"], "result at line 2");
    (adapter, written)
}

fn state(brush: &str) -> avenger_results_egui::ChartState {
    avenger_results_egui::ChartState {
        selections: vec![("brush".into(), vec![brush.into()])],
        ..Default::default()
    }
}

#[test]
fn the_chart_s_state_refreshes_a_client_once_for_each_settled_gesture() {
    // A client without `invalidated` stops again at the result, once when the chart first
    // reports its state, and once for each settled gesture, with a line in the console.
    let (mut adapter, written) = at_result(false);
    adapter.chart_changed(state("clear"), None);
    assert_eq!(written.events(), ["stopped"]);
    adapter.chart_changed(state("clear"), None);
    assert!(written.events().is_empty());
    let brush = "brush: Horsepower 80 to 140".to_string();
    adapter.chart_changed(state("Horsepower 80 to 140"), Some(brush.clone()));
    let messages = written.take();
    assert_eq!(messages[0]["event"], "output");
    assert_eq!(messages[0]["body"]["output"], format!("{brush}\n"));
    assert_eq!(messages[1]["event"], "stopped");
    assert_eq!(messages[1]["body"]["reason"], "pause");
    // The result's scopes hold the chart's state.
    let scopes = super::protocol::Request {
        seq: 9,
        command: "scopes".into(),
        arguments: serde_json::json!({"frameId": 1}),
    };
    adapter.answer(&scopes);
    let response = written.take().pop().unwrap();
    let names: Vec<&str> = (response["body"]["scopes"].as_array().unwrap().iter())
        .map(|scope| scope["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "Result",
            "Script",
            "Selections",
            "Widgets",
            "Scale Domains",
            "Modes"
        ]
    );
    // A client with `invalidated` hears it in place of a stop.
    let (mut adapter, written) = at_result(true);
    adapter.chart_changed(state("clear"), Some(brush));
    assert_eq!(written.events(), ["output", "invalidated"]);
}
