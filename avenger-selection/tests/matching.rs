//! The rows that conditions match, and the matching of a predicate over the clauses of
//! selections.

use std::collections::BTreeMap;
use std::sync::Arc;

use arrow::array::{
    Array, ArrayRef, BooleanArray, DictionaryArray, Float64Array, Int64Array, StringArray,
};
use arrow::datatypes::Int32Type;
use avenger_scales::scales::linear::LinearScale;
use avenger_selection::{
    matching, Clause, Condition, Matching, PixelGrid, Predicate, Resolution, Selected,
};

/// The values of a mask, which has no nulls.
trait Bits {
    fn bits(self) -> Vec<bool>;
}

impl<E: std::fmt::Debug> Bits for Result<BooleanArray, E> {
    fn bits(self) -> Vec<bool> {
        let mask = self.unwrap();
        assert_eq!(mask.null_count(), 0, "a mask has no nulls");
        mask.values().iter().collect()
    }
}

fn numbers(values: &[Option<f64>]) -> ArrayRef {
    Arc::new(Float64Array::from(values.to_vec()))
}

fn range(ends: [f64; 2], exclude_end: bool) -> Condition {
    Condition::Range {
        column: "x".into(),
        channel: Some("x".into()),
        ends: numbers(&[Some(ends[0]), Some(ends[1])]),
        exclude_end,
        cells: None,
    }
}

/// A set matches equal values, through a dictionary and across numeric types, and a null matches
/// no set.
#[test]
fn sets_match_equal_values() {
    let set = Condition::values(
        "c",
        None,
        Arc::new(StringArray::from(vec![Some("a"), None, Some("c")])),
    );
    let keys: DictionaryArray<Int32Type> = vec![Some("a"), Some("b"), None, Some("c")]
        .into_iter()
        .collect();
    assert_eq!(
        set.matches(&[Arc::new(keys)]).bits(),
        [true, false, false, true]
    );
    let counts = Condition::values("n", None, Arc::new(Int64Array::from(vec![2, 3])));
    let column = numbers(&[Some(2.0), Some(2.5), Some(3.0)]);
    assert_eq!(counts.matches(&[column]).bits(), [true, false, true]);
}

/// A range includes both ends, or its start alone with `exclude_end`, and a null matches no range.
#[test]
fn ranges_match_their_ends() {
    let column = numbers(&[Some(1.0), Some(2.0), Some(3.0), None, Some(4.0)]);
    assert_eq!(
        range([2.0, 3.0], false)
            .matches(std::slice::from_ref(&column))
            .bits(),
        [false, true, true, false, false]
    );
    assert_eq!(
        range([2.0, 3.0], true).matches(&[column]).bits(),
        [false, true, false, false, false]
    );
}

/// A gridded range matches the values in its cells, not between its ends.
#[test]
fn a_gridded_range_matches_its_cells() {
    // Cells 2 to 9 of 10 pixels hold the values 10 to 50, without 50.
    let grid = PixelGrid::new(LinearScale::configured((0.0, 100.0), (0.0, 200.0)), 10.0).unwrap();
    let gridded = Condition::Range {
        column: "x".into(),
        channel: Some("x".into()),
        ends: numbers(&[Some(12.0), Some(48.0)]),
        exclude_end: false,
        cells: Some(grid.span(20.0, 100.0).unwrap()),
    };
    let column = numbers(&[Some(10.0), Some(11.0), Some(49.0), Some(50.0)]);
    assert_eq!(gridded.matches(&[column]).bits(), [true, true, true, false]);
}

/// A polygon matches the points inside it by the even-odd rule, and a point with a null matches
/// none.
#[test]
fn polygons_match_the_points_inside_them() {
    let square = Condition::Polygon {
        x_column: "x".into(),
        y_column: "y".into(),
        polygons: vec![(
            numbers(&[Some(0.0), Some(4.0), Some(4.0), Some(0.0)]),
            numbers(&[Some(0.0), Some(0.0), Some(4.0), Some(4.0)]),
        )],
        cells: None,
    };
    assert_eq!(square.columns(), ["x", "y"]);
    let x = numbers(&[Some(2.0), Some(5.0), None, Some(1.0)]);
    let y = numbers(&[Some(2.0), Some(2.0), Some(2.0), Some(3.5)]);
    assert_eq!(square.matches(&[x, y]).bits(), [true, false, false, true]);
}

/// A cyclic range runs forward from its start across the wrap, and a range of a whole period
/// matches every value.
#[test]
fn cyclic_ranges_wrap() {
    let across = |start: f64, end: f64| Condition::CyclicRange {
        column: "lon".into(),
        channel: Some("longitude".into()),
        start,
        end,
        period: 360.0,
    };
    let column = numbers(&[Some(175.0), Some(-175.0), Some(0.0), None, Some(535.0)]);
    assert_eq!(
        across(170.0, -170.0)
            .matches(std::slice::from_ref(&column))
            .bits(),
        [true, true, false, false, true]
    );
    assert_eq!(
        across(-180.0, 180.0).matches(&[column]).bits(),
        [true, true, true, false, true]
    );
}

/// The resolutions of the one selection `brush`.
fn resolutions(resolution: Resolution) -> BTreeMap<String, Resolution> {
    BTreeMap::from([("brush".to_string(), resolution)])
}

fn selected(empty_selects_none: bool, exclude_own: bool) -> Predicate {
    Predicate::Selected(Selected {
        selection: "brush".into(),
        empty_selects_none,
        exclude_own,
    })
}

/// The mask of `matching` over the column `x` of the values 1 to 4.
fn mask(matching: &Matching) -> Vec<bool> {
    let x = numbers(&[Some(1.0), Some(2.0), Some(3.0), Some(4.0)]);
    let mut matches = |_: &str, condition: &Condition| condition.matches(std::slice::from_ref(&x));
    matching.mask(4, &mut matches).bits()
}

/// A clause of `owner` that selects `x` from `start` through `end`.
fn brushed(owner: &str, start: f64, end: f64) -> Clause {
    Clause::new(owner).condition(range([start, end], false))
}

/// A selection without clauses selects every row, or none with `empty_selects_none`, and a
/// selection whose clauses `exclude_own` skips, all of them, selects every row.
#[test]
fn empty_and_skipped_selections_select_every_row_or_none() {
    let union = resolutions(Resolution::Union);
    let none = BTreeMap::new();
    let nobody = |_: &Clause| false;
    assert_eq!(
        mask(&matching(&selected(false, false), &union, &none, &nobody)),
        [true; 4]
    );
    assert_eq!(
        mask(&matching(&selected(true, false), &union, &none, &nobody)),
        [false; 4]
    );

    let own = BTreeMap::from([("brush".to_string(), vec![brushed("plot", 2.0, 3.0)])]);
    let mine = |clause: &Clause| clause.owner == "plot";
    assert_eq!(
        mask(&matching(&selected(true, true), &union, &own, &mine)),
        [true; 4]
    );
    assert_eq!(
        mask(&matching(&selected(true, false), &union, &own, &mine)),
        [false, true, true, false]
    );
}

/// An intersect selection matches the rows that every owner's clause matches, and a union
/// selection, or a selection without a resolution, the rows that some clause matches.
#[test]
fn resolutions_combine_clauses() {
    let clauses = BTreeMap::from([(
        "brush".to_string(),
        vec![brushed("left", 1.0, 3.0), brushed("right", 2.0, 4.0)],
    )]);
    let nobody = |_: &Clause| false;
    let intersect = resolutions(Resolution::Intersect);
    assert_eq!(
        mask(&matching(
            &selected(true, false),
            &intersect,
            &clauses,
            &nobody
        )),
        [false, true, true, false]
    );
    let union = resolutions(Resolution::Union);
    assert_eq!(
        mask(&matching(&selected(true, false), &union, &clauses, &nobody)),
        [true; 4]
    );
    assert_eq!(
        mask(&matching(
            &selected(true, false),
            &BTreeMap::new(),
            &clauses,
            &nobody
        )),
        [true; 4]
    );
}

/// A row matches a clause when it matches every condition, and `Not`, `And`, and `Or` combine
/// predicates. An `Or` of no predicates selects no row.
#[test]
fn conditions_and_predicates_combine() {
    let union = resolutions(Resolution::Union);
    let ends = Clause::new("plot")
        .condition(Condition::values(
            "x",
            None,
            numbers(&[Some(1.0), Some(4.0)]),
        ))
        .condition(range([0.0, 10.0], false));
    let clauses = BTreeMap::from([("brush".to_string(), vec![ends])]);
    let nobody = |_: &Clause| false;
    let brush = selected(true, false);
    let masked = |predicate: &Predicate| mask(&matching(predicate, &union, &clauses, &nobody));
    assert_eq!(masked(&brush), [true, false, false, true]);
    let not = Predicate::Not(Box::new(brush.clone()));
    assert_eq!(masked(&not), [false, true, true, false]);
    assert_eq!(
        masked(&Predicate::And(vec![brush.clone(), not.clone()])),
        [false; 4]
    );
    assert_eq!(masked(&Predicate::Or(vec![brush, not])), [true; 4]);
    assert_eq!(masked(&Predicate::Or(vec![])), [false; 4]);
}

/// A set of two columns matches its tuples and not their cross combinations. A null in the row or
/// in a tuple matches nothing, and the values cast to their columns' types.
#[test]
fn sets_of_two_columns_match_their_tuples() {
    let pairs = Condition::set(
        vec!["species".into(), "year".into()],
        vec![Some("x".into()), Some("color".into())],
        vec![
            Arc::new(StringArray::from(vec![
                Some("adelie"),
                Some("gentoo"),
                Some("chinstrap"),
            ])),
            Arc::new(Int64Array::from(vec![Some(2007), Some(2008), None])),
        ],
    )
    .unwrap();
    assert_eq!(pairs.columns(), ["species", "year"]);
    let species: ArrayRef = Arc::new(StringArray::from(vec![
        Some("adelie"),
        Some("gentoo"),
        Some("adelie"),
        Some("gentoo"),
        None,
        Some("chinstrap"),
    ]));
    let year = numbers(&[
        Some(2007.0),
        Some(2008.0),
        Some(2008.0),
        Some(2007.0),
        Some(2007.0),
        None,
    ]);
    assert_eq!(
        pairs.matches(&[species, year]).bits(),
        [true, true, false, false, false, false]
    );
    let uneven = Condition::set(
        vec!["a".into(), "b".into()],
        vec![None, None],
        vec![numbers(&[Some(1.0)]), numbers(&[Some(1.0), Some(2.0)])],
    );
    assert!(uneven.is_err());
    assert!(Condition::set(vec![], vec![], vec![]).is_err());
}

/// A point inside one of several polygons matches, also where two polygons overlap.
#[test]
fn several_polygons_match_their_union() {
    let square = |low: f64, high: f64| {
        (
            numbers(&[Some(low), Some(high), Some(high), Some(low)]),
            numbers(&[Some(low), Some(low), Some(high), Some(high)]),
        )
    };
    let lassos = Condition::Polygon {
        x_column: "x".into(),
        y_column: "y".into(),
        polygons: vec![square(0.0, 4.0), square(2.0, 6.0)],
        cells: None,
    };
    let x = numbers(&[Some(1.0), Some(3.0), Some(5.0), Some(7.0)]);
    let y = numbers(&[Some(1.0), Some(3.0), Some(5.0), Some(7.0)]);
    assert_eq!(lassos.matches(&[x, y]).bits(), [true, true, true, false]);
}

/// The even-odd rule over every edge of `vertices`, from each vertex to the vertex before it.
fn even_odd(vertices: &[(f64, f64)], x: f64, y: f64) -> bool {
    let mut inside = false;
    let mut previous = vertices.len().wrapping_sub(1);
    for (index, &(xi, yi)) in vertices.iter().enumerate() {
        let (xj, yj) = vertices[previous];
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        previous = index;
    }
    inside
}

/// A deterministic pseudo-random sequence in [0, 1).
fn noise(mut state: u64) -> impl FnMut() -> f64 {
    move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Whether `polygons` match exactly the points that the even-odd rule over every edge of some
/// polygon matches.
fn matches_even_odd(polygons: &[Vec<(f64, f64)>], points: &[(f64, f64)]) {
    let column = |values: Vec<f64>| -> ArrayRef { Arc::new(Float64Array::from(values)) };
    let condition = Condition::Polygon {
        x_column: "x".into(),
        y_column: "y".into(),
        polygons: (polygons.iter())
            .map(|vertices| {
                (
                    column(vertices.iter().map(|v| v.0).collect()),
                    column(vertices.iter().map(|v| v.1).collect()),
                )
            })
            .collect(),
        cells: None,
    };
    let x = column(points.iter().map(|p| p.0).collect());
    let y = column(points.iter().map(|p| p.1).collect());
    let matched = condition.matches(&[x, y]).bits();
    for (point, matched) in points.iter().zip(matched) {
        let expected = (polygons.iter()).any(|vertices| even_odd(vertices, point.0, point.1));
        assert_eq!(matched, expected, "the point {point:?}");
    }
}

/// A concave lasso of many vertices and a self-crossing star match exactly the points that the
/// even-odd rule over every edge matches: points inside and outside them, at the heights of their
/// vertices, on their vertices and the midpoints of their edges, and just beside their extremes.
#[test]
fn lassos_match_the_even_odd_rule_exactly() {
    let mut next = noise(0x9E37_79B9_7F4A_7C15);
    let lasso: Vec<(f64, f64)> = (0..500)
        .map(|index| {
            let angle = index as f64 / 500.0 * std::f64::consts::TAU;
            let radius = 10.0 + 30.0 * next();
            (50.0 + radius * angle.cos(), 50.0 + radius * angle.sin())
        })
        .collect();
    // A pentagram drawn in one stroke, whose center the even-odd rule leaves outside.
    let star: Vec<(f64, f64)> = [0, 2, 4, 1, 3]
        .map(|index| {
            let angle = index as f64 / 5.0 * std::f64::consts::TAU;
            (150.0 + 40.0 * angle.sin(), 50.0 + 40.0 * angle.cos())
        })
        .to_vec();
    let mut points: Vec<(f64, f64)> = (0..20_000)
        .map(|_| (200.0 * next() - 5.0, 100.0 * next()))
        .collect();
    for vertices in [&lasso, &star] {
        let previous = |index: usize| vertices[(index + vertices.len() - 1) % vertices.len()];
        for (index, &(x, y)) in vertices.iter().enumerate() {
            let (xj, yj) = previous(index);
            points.extend([
                (x, y),
                (x - 1e-9, y),
                (x + 1e-9, y),
                ((x + xj) / 2.0, (y + yj) / 2.0),
            ]);
        }
        let (left, right) = (vertices.iter()).fold((f64::INFINITY, f64::NEG_INFINITY), |b, v| {
            (b.0.min(v.0), b.1.max(v.0))
        });
        points.extend([
            (left, 50.0),
            (right, 50.0),
            (left - 1e-12, 50.0),
            (right + 1e-12, 50.0),
        ]);
    }
    matches_even_odd(std::slice::from_ref(&lasso), &points);
    matches_even_odd(std::slice::from_ref(&star), &points);
    matches_even_odd(&[lasso, star], &points);
}

/// A polygon with a vertex that is not finite matches as the even-odd rule over every edge does.
#[test]
fn polygons_with_infinite_vertices_follow_the_even_odd_rule() {
    let wedge = vec![(0.0, 0.0), (4.0, 0.0), (f64::INFINITY, 4.0), (0.0, 4.0)];
    let points: Vec<(f64, f64)> = (0..10)
        .flat_map(|x| (0..10).map(move |y| (x as f64 - 2.5, y as f64 * 0.5 - 0.25)))
        .collect();
    matches_even_odd(&[wedge], &points);
}
