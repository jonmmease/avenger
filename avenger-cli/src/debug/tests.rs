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
