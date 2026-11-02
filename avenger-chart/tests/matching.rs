//! The rows that the tests of contributions match, and the matching of a predicate over the
//! contributions of a chart's selections.

use std::collections::BTreeMap;
use std::sync::Arc;

use arrow::array::{ArrayRef, DictionaryArray, Float64Array, Int64Array, RecordBatch, StringArray};
use arrow::datatypes::Int32Type;
use avenger_chart::{proto, Chart, Contribution, Matching, PixelGrid, Selection, TupleTest};
use avenger_scales::scales::linear::LinearScale;

fn numbers(values: &[Option<f64>]) -> ArrayRef {
    Arc::new(Float64Array::from(values.to_vec()))
}

fn range(ends: [f64; 2], exclude_end: bool) -> TupleTest {
    TupleTest::Range {
        column: "x".into(),
        channel: proto::Channel::X,
        ends: numbers(&[Some(ends[0]), Some(ends[1])]),
        exclude_end,
        grid: None,
    }
}

/// A set matches equal values, through a dictionary and across numeric types, and a null matches
/// no set.
#[test]
fn sets_match_equal_values() {
    let set = TupleTest::Set {
        column: "c".into(),
        channel: proto::Channel::Unspecified,
        values: Arc::new(StringArray::from(vec![Some("a"), None, Some("c")])),
    };
    let keys: DictionaryArray<Int32Type> = vec![Some("a"), Some("b"), None, Some("c")]
        .into_iter()
        .collect();
    assert_eq!(
        set.matches(&[Arc::new(keys)]).unwrap(),
        [true, false, false, true]
    );
    let counts = TupleTest::Set {
        column: "n".into(),
        channel: proto::Channel::Unspecified,
        values: Arc::new(Int64Array::from(vec![2, 3])),
    };
    let column = numbers(&[Some(2.0), Some(2.5), Some(3.0)]);
    assert_eq!(counts.matches(&[column]).unwrap(), [true, false, true]);
}

/// A range includes both ends, or its start alone with `exclude_end`, and a null matches no range.
#[test]
fn ranges_match_their_ends() {
    let column = numbers(&[Some(1.0), Some(2.0), Some(3.0), None, Some(4.0)]);
    assert_eq!(
        range([2.0, 3.0], false)
            .matches(std::slice::from_ref(&column))
            .unwrap(),
        [false, true, true, false, false]
    );
    assert_eq!(
        range([2.0, 3.0], true).matches(&[column]).unwrap(),
        [false, true, false, false, false]
    );
}

/// A gridded range matches the values in its cells, not between its ends.
#[test]
fn a_gridded_range_matches_its_cells() {
    // Cells 2 to 9 of 10 pixels hold the values 10 to 50, without 50.
    let grid = PixelGrid::new(LinearScale::configured((0.0, 100.0), (0.0, 200.0)), 10.0).unwrap();
    let gridded = TupleTest::Range {
        column: "x".into(),
        channel: proto::Channel::X,
        ends: numbers(&[Some(12.0), Some(48.0)]),
        exclude_end: false,
        grid: Some(grid.span(20.0, 100.0).unwrap()),
    };
    let column = numbers(&[Some(10.0), Some(11.0), Some(49.0), Some(50.0)]);
    assert_eq!(
        gridded.matches(&[column]).unwrap(),
        [true, true, true, false]
    );
}

/// A polygon matches the points inside it by the even-odd rule, and a point with a null matches
/// none.
#[test]
fn polygons_match_the_points_inside_them() {
    let square = TupleTest::Polygon {
        x_column: "x".into(),
        y_column: "y".into(),
        x: numbers(&[Some(0.0), Some(4.0), Some(4.0), Some(0.0)]),
        y: numbers(&[Some(0.0), Some(0.0), Some(4.0), Some(4.0)]),
    };
    assert_eq!(square.columns(), ["x", "y"]);
    let x = numbers(&[Some(2.0), Some(5.0), None, Some(1.0)]);
    let y = numbers(&[Some(2.0), Some(2.0), Some(2.0), Some(3.5)]);
    assert_eq!(square.matches(&[x, y]).unwrap(), [true, false, false, true]);
}

/// A cyclic range runs forward from its start across the wrap, and a range of a whole period
/// matches every value.
#[test]
fn cyclic_ranges_wrap() {
    let across = |start: f64, end: f64| TupleTest::CyclicRange {
        column: "lon".into(),
        channel: proto::Channel::Longitude,
        start,
        end,
        period: 360.0,
    };
    let column = numbers(&[Some(175.0), Some(-175.0), Some(0.0), None, Some(535.0)]);
    assert_eq!(
        across(170.0, -170.0)
            .matches(std::slice::from_ref(&column))
            .unwrap(),
        [true, true, false, false, true]
    );
    assert_eq!(
        across(-180.0, 180.0).matches(&[column]).unwrap(),
        [true, true, true, false, true]
    );
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
        predicate: Some(proto::predicate::Predicate::Selection(
            proto::SelectionTest {
                selection: "brush".into(),
                empty_selects_none,
                exclude_own,
            },
        )),
    }
}

/// The mask of `matching` over the column `x` of the values 1 to 4.
fn mask(matching: &Matching) -> Vec<bool> {
    let x = numbers(&[Some(1.0), Some(2.0), Some(3.0), Some(4.0)]);
    let mut test = |_: &str, test: &TupleTest| test.matches(std::slice::from_ref(&x));
    matching.mask(4, &mut test).unwrap()
}

/// A contribution of `owner` that selects `x` from `start` through `end`.
fn brushed(owner: &str, start: f64, end: f64) -> Contribution {
    Contribution::new(owner).tuple([range([start, end], false)])
}

/// A selection without contributions selects every row, or none with `empty_selects_none`, and a
/// selection whose contributions `exclude_own` skips, all of them, selects every row.
#[test]
fn empty_and_skipped_selections_select_every_row_or_none() {
    let chart = chart(proto::SelectionResolution::Union);
    let none = BTreeMap::new();
    let nobody = |_: &Contribution| false;
    assert_eq!(
        mask(&chart.matching(&selected(false, false), &none, &nobody)),
        [true; 4]
    );
    assert_eq!(
        mask(&chart.matching(&selected(true, false), &none, &nobody)),
        [false; 4]
    );

    let own = BTreeMap::from([("brush".to_string(), vec![brushed("plot", 2.0, 3.0)])]);
    let mine = |contribution: &Contribution| contribution.owner == "plot";
    assert_eq!(
        mask(&chart.matching(&selected(true, true), &own, &mine)),
        [true; 4]
    );
    assert_eq!(
        mask(&chart.matching(&selected(true, false), &own, &mine)),
        [false, true, true, false]
    );
}

/// An intersect selection matches the rows that every owner's contribution matches, and a union
/// selection the rows that some contribution matches.
#[test]
fn resolutions_combine_contributions() {
    let contributions = BTreeMap::from([(
        "brush".to_string(),
        vec![brushed("left", 1.0, 3.0), brushed("right", 2.0, 4.0)],
    )]);
    let nobody = |_: &Contribution| false;
    let intersect = chart(proto::SelectionResolution::Intersect);
    assert_eq!(
        mask(&intersect.matching(&selected(true, false), &contributions, &nobody)),
        [false, true, true, false]
    );
    let union = chart(proto::SelectionResolution::Union);
    assert_eq!(
        mask(&union.matching(&selected(true, false), &contributions, &nobody)),
        [true; 4]
    );
}

/// A row matches a contribution when it matches one of its tuples, and `not`, `and`, and `or`
/// combine predicates. A predicate without a kind selects no row.
#[test]
fn tuples_and_predicates_combine() {
    let chart = chart(proto::SelectionResolution::Union);
    let two_ranges = Contribution::new("plot")
        .tuple([range([1.0, 1.0], false)])
        .tuple([range([4.0, 4.0], false)]);
    let contributions = BTreeMap::from([("brush".to_string(), vec![two_ranges])]);
    let nobody = |_: &Contribution| false;
    let brush = selected(true, false);
    assert_eq!(
        mask(&chart.matching(&brush, &contributions, &nobody)),
        [true, false, false, true]
    );
    let not = proto::Predicate {
        predicate: Some(proto::predicate::Predicate::Not(Box::new(brush.clone()))),
    };
    assert_eq!(
        mask(&chart.matching(&not, &contributions, &nobody)),
        [false, true, true, false]
    );
    let both = proto::Predicate {
        predicate: Some(proto::predicate::Predicate::And(proto::PredicateList {
            predicates: vec![brush.clone(), not.clone()],
        })),
    };
    assert_eq!(
        mask(&chart.matching(&both, &contributions, &nobody)),
        [false; 4]
    );
    let either = proto::Predicate {
        predicate: Some(proto::predicate::Predicate::Or(proto::PredicateList {
            predicates: vec![brush, not],
        })),
    };
    assert_eq!(
        mask(&chart.matching(&either, &contributions, &nobody)),
        [true; 4]
    );
    let kindless = proto::Predicate::default();
    assert_eq!(
        mask(&chart.matching(&kindless, &contributions, &nobody)),
        [false; 4]
    );
}
