//! The matching of a chart's predicates over the clauses of its selections, through
//! avenger-selection, and the labels of channels.

use std::collections::BTreeMap;
use std::sync::Arc;

use arrow::array::{ArrayRef, Float64Array, RecordBatch};
use avenger_chart::{proto, Chart, Clause, Condition, Selection};

fn numbers(values: &[Option<f64>]) -> ArrayRef {
    Arc::new(Float64Array::from(values.to_vec()))
}

fn range(ends: [f64; 2], exclude_end: bool) -> Condition {
    Condition::Range {
        column: "x".into(),
        channel: proto::Channel::X.label(),
        ends: numbers(&[Some(ends[0]), Some(ends[1])]),
        exclude_end,
        cells: None,
    }
}

/// A chart of one plot `plot` with the selection `brush` of `resolution`.
fn chart(resolution: proto::SelectionResolution) -> Chart {
    let table = RecordBatch::try_from_iter([
        ("x", numbers(&[Some(1.0), Some(2.0), Some(3.0), Some(4.0)])),
        ("y", numbers(&[Some(1.0), Some(2.0), Some(3.0), Some(4.0)])),
    ])
    .unwrap();
    Chart::unit(table, proto::SymbolMark::default())
        .id("plot")
        .x("x")
        .y("y")
        .selection(Selection::new("brush", resolution))
        .build()
        .unwrap()
}

fn selected(empty_selects_none: bool, exclude_own: bool) -> proto::Predicate {
    proto::Predicate {
        predicate: Some(proto::predicate::Predicate::Selected(proto::Selected {
            selection: "brush".into(),
            empty_selects_none,
            exclude_own,
        })),
    }
}

/// Whether `predicate` selects each row of the column `x` of the values 1 to 4, over `clauses`,
/// for rows whose owners have the ids `owners`.
fn mask(
    chart: &Chart,
    predicate: &proto::Predicate,
    clauses: &BTreeMap<String, Vec<Clause>>,
    owners: &[&str],
) -> Vec<bool> {
    let owners: Vec<String> = owners.iter().map(|owner| owner.to_string()).collect();
    let rows =
        RecordBatch::try_from_iter([("x", numbers(&[Some(1.0), Some(2.0), Some(3.0), Some(4.0)]))])
            .unwrap();
    let states = chart.selection_states(clauses);
    let mask = (chart.predicate(predicate, &owners, false))
        .mask(&states, &rows)
        .unwrap();
    mask.iter().map(|selected| selected == Some(true)).collect()
}

/// A clause of `owner` that selects `x` from `start` through `end`.
fn brushed(owner: &str, start: f64, end: f64) -> Clause {
    Clause::new(owner).conditions([range([start, end], false)])
}

/// A selection without clauses selects every row, or none with `empty_selects_none`, and a
/// selection whose clauses `exclude_own` skips, all of them, selects every row.
#[test]
fn empty_and_skipped_selections_select_every_row_or_none() {
    let chart = chart(proto::SelectionResolution::Union);
    let none = BTreeMap::new();
    let nobody: &[&str] = &[];
    assert_eq!(
        mask(&chart, &selected(false, false), &none, nobody),
        [true; 4]
    );
    assert_eq!(
        mask(&chart, &selected(true, false), &none, nobody),
        [false; 4]
    );

    let own = BTreeMap::from([("brush".to_string(), vec![brushed("plot", 2.0, 3.0)])]);
    let mine: &[&str] = &["plot"];
    assert_eq!(mask(&chart, &selected(true, true), &own, mine), [true; 4]);
    assert_eq!(
        mask(&chart, &selected(true, false), &own, mine),
        [false, true, true, false]
    );
}

/// An intersect selection matches the rows that every owner's clause matches, and a union
/// selection the rows that some clause matches.
#[test]
fn resolutions_combine_clauses() {
    let clauses = BTreeMap::from([(
        "brush".to_string(),
        vec![brushed("left", 1.0, 3.0), brushed("right", 2.0, 4.0)],
    )]);
    let nobody: &[&str] = &[];
    let intersect = chart(proto::SelectionResolution::Intersect);
    assert_eq!(
        mask(&intersect, &selected(true, false), &clauses, nobody),
        [false, true, true, false]
    );
    let union = chart(proto::SelectionResolution::Union);
    assert_eq!(
        mask(&union, &selected(true, false), &clauses, nobody),
        [true; 4]
    );
}

/// A row matches a clause when it matches every condition, and `not`, `and`, and `or` combine
/// predicates. A predicate without a kind selects no row.
#[test]
fn conditions_and_predicates_combine() {
    let chart = chart(proto::SelectionResolution::Union);
    let ends = Clause::new("plot")
        .condition(Condition::values(
            "x",
            None,
            numbers(&[Some(1.0), Some(4.0)]),
        ))
        .condition(range([0.0, 10.0], false));
    let clauses = BTreeMap::from([("brush".to_string(), vec![ends])]);
    let nobody: &[&str] = &[];
    let brush = selected(true, false);
    assert_eq!(
        mask(&chart, &brush, &clauses, nobody),
        [true, false, false, true]
    );
    let not = proto::Predicate {
        predicate: Some(proto::predicate::Predicate::Not(Box::new(brush.clone()))),
    };
    assert_eq!(
        mask(&chart, &not, &clauses, nobody),
        [false, true, true, false]
    );
    let both = proto::Predicate {
        predicate: Some(proto::predicate::Predicate::And(proto::PredicateList {
            predicates: vec![brush.clone(), not.clone()],
        })),
    };
    assert_eq!(mask(&chart, &both, &clauses, nobody), [false; 4]);
    let either = proto::Predicate {
        predicate: Some(proto::predicate::Predicate::Or(proto::PredicateList {
            predicates: vec![brush, not],
        })),
    };
    assert_eq!(mask(&chart, &either, &clauses, nobody), [true; 4]);
    let kindless = proto::Predicate::default();
    assert_eq!(mask(&chart, &kindless, &clauses, nobody), [false; 4]);
}

/// A channel's label is its lowercase name, and the label gives the channel back.
#[test]
fn channels_round_trip_through_labels() {
    for channel in [
        proto::Channel::X,
        proto::Channel::Theta2,
        proto::Channel::Longitude,
    ] {
        let label = channel.label();
        assert_eq!(proto::Channel::from_label(label.as_deref()), channel);
    }
    assert_eq!(proto::Channel::X.label().as_deref(), Some("x"));
    assert_eq!(proto::Channel::Unspecified.label(), None);
    assert_eq!(
        proto::Channel::from_label(Some("nonsense")),
        proto::Channel::Unspecified
    );
}
