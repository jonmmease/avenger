//! The test of a selection's state selects the rows that its rules select one row at a time, for
//! every option, and a faceted owner's rows skip only the clause of their own panel.

use std::{collections::BTreeMap, sync::Arc};

use arrow::{
    array::{Array, ArrayRef, DictionaryArray, Float64Array, RecordBatch, StringArray},
    datatypes::{DataType, Int32Type},
};
use avenger_scales::scalar::Scalar;
use avenger_selection::{Clause, Condition, Resolution, SelectedOptions, SelectionState};

const ROWS: usize = 24;

/// The rows: `x` with a null and a NaN, `y`, `species`, a dictionary of three values, and
/// `island`, the panel, with a null.
fn table() -> RecordBatch {
    let x: Vec<Option<f64>> = (0..ROWS)
        .map(|i| match i {
            5 => None,
            7 => Some(f64::NAN),
            _ => Some(i as f64 * 0.5),
        })
        .collect();
    let y: Vec<f64> = (0..ROWS).map(|i| ((i * 7) % 10) as f64).collect();
    let species: DictionaryArray<Int32Type> = (0..ROWS)
        .map(|i| Some(["adelie", "gentoo", "chinstrap"][i % 3]))
        .collect();
    let island: Vec<Option<&str>> = (0..ROWS)
        .map(|i| (i != 11).then_some(["biscoe", "dream", "torgersen"][(i / 2) % 3]))
        .collect();
    RecordBatch::try_from_iter([
        ("x", Arc::new(Float64Array::from(x)) as ArrayRef),
        ("y", Arc::new(Float64Array::from(y))),
        ("species", Arc::new(species)),
        ("island", Arc::new(StringArray::from(island))),
    ])
    .unwrap()
}

fn numbers(values: &[f64]) -> ArrayRef {
    Arc::new(Float64Array::from(values.to_vec()))
}

fn range(column: &str, low: f64, high: f64) -> Condition {
    Condition::Range {
        column: column.into(),
        channel: None,
        ends: numbers(&[low, high]),
        exclude_end: false,
        cells: None,
    }
}

/// The rows that the rules select, one row at a time, with the row's `island` as its panel: a
/// selection without clauses selects every row, or none with `empty_selects_none`, a selection
/// whose clauses the options skip, all of them, every row, and otherwise an intersect selection
/// the rows that every kept clause matches, and any other the rows that one matches.
fn expected(state: &SelectionState, options: &SelectedOptions) -> Vec<bool> {
    let table = table();
    let islands = table.column_by_name("island").unwrap().clone();
    (0..ROWS)
        .map(|row| {
            if state.clauses.is_empty() {
                return !options.empty_selects_none;
            }
            let panel =
                BTreeMap::from([("island".to_string(), Scalar::new(islands.slice(row, 1)))]);
            let skipped = |clause: &Clause| {
                options.without.contains(&clause.owner)
                    || (options.exclude_own.contains(&clause.owner)
                        && (!options.faceted || clause.partition == panel))
            };
            let slice = table.slice(row, 1);
            let matches = |clause: &&Clause| {
                clause.conditions.iter().all(|condition| {
                    let columns: Vec<ArrayRef> = (condition.columns().iter())
                        .map(|name| slice.column_by_name(name).unwrap().clone())
                        .collect();
                    condition.matches(&columns).unwrap().value(0)
                })
            };
            let kept: Vec<&Clause> = state.clauses.iter().filter(|c| !skipped(c)).collect();
            match state.resolution {
                _ if kept.is_empty() => true,
                Resolution::Intersect => kept.iter().all(matches),
                _ => kept.iter().any(matches),
            }
        })
        .collect()
}

fn conforms(state: &SelectionState, options: &SelectedOptions) {
    let mask = state.mask(options, &table()).unwrap();
    assert_eq!(mask.null_count(), 0);
    let mask: Vec<bool> = mask.values().iter().collect();
    assert_eq!(mask, expected(state, options), "{options:?}");
}

fn options() -> [SelectedOptions; 6] {
    let exclude = |owner: &str| vec![owner.to_string()];
    [
        SelectedOptions::default(),
        SelectedOptions {
            empty_selects_none: true,
            ..Default::default()
        },
        SelectedOptions {
            exclude_own: exclude("a"),
            ..Default::default()
        },
        SelectedOptions {
            without: vec!["b".into()],
            ..Default::default()
        },
        SelectedOptions {
            without: vec!["a".into(), "b".into(), "panels".into()],
            empty_selects_none: true,
            ..Default::default()
        },
        SelectedOptions {
            exclude_own: exclude("panels"),
            faceted: true,
            without: vec!["b".into()],
            ..Default::default()
        },
    ]
}

/// Every resolution and option selects the rows that the rules select, for selections of
/// several owners and kinds of conditions, and for an empty selection.
#[test]
fn options_and_resolutions_conform() {
    let set = Condition::values("species", None, Arc::new(StringArray::from(vec!["gentoo"])));
    let clauses = vec![
        Clause::new("a").conditions([range("x", 0.0, 6.0), range("y", 0.0, 5.0)]),
        Clause::new("b").condition(set),
        Clause::new("c").condition(range("y", 3.0, 8.0)),
    ];
    for resolution in [Resolution::Global, Resolution::Union, Resolution::Intersect] {
        let state = SelectionState::new(resolution, clauses.clone());
        for options in options() {
            conforms(&state, &options);
            conforms(&SelectionState::default(), &options);
        }
    }
}

/// A faceted owner's row skips only the clause of its own panel, a row of a panel without a
/// clause and a row with a null panel keep every clause of the owner, and a panel value casts to
/// its column's type.
#[test]
fn a_faceted_owner_skips_only_its_own_panel() {
    let panel = |island: &str, low: f64, high: f64| {
        Clause::new("panels")
            .partition("island", island)
            .condition(range("x", low, high))
    };
    let other = Clause::new("other").condition(range("y", 2.0, 9.0));
    for resolution in [Resolution::Union, Resolution::Intersect] {
        let state = SelectionState::new(
            resolution,
            vec![
                panel("biscoe", 0.0, 3.0),
                panel("dream", 6.0, 9.0),
                other.clone(),
            ],
        );
        for options in options() {
            conforms(&state, &options);
            // A dictionary of the panels selects the same rows.
            let table = table();
            let island = table.column_by_name("island").unwrap();
            let dictionary =
                DataType::Dictionary(Box::new(DataType::Int8), Box::new(DataType::Utf8));
            let island = arrow::compute::cast(island, &dictionary).unwrap();
            let index = table.schema().index_of("island").unwrap();
            let mut columns = table.columns().to_vec();
            columns[index] = island;
            let fields: Vec<_> = (table.schema().fields().iter())
                .map(|field| match field.name().as_str() {
                    "island" => arrow::datatypes::Field::new("island", dictionary.clone(), true),
                    _ => field.as_ref().clone(),
                })
                .collect();
            let schema = Arc::new(arrow::datatypes::Schema::new(fields));
            let dictionaries = RecordBatch::try_new(schema, columns).unwrap();
            assert_eq!(
                state.mask(&options, &dictionaries).unwrap(),
                state.mask(&options, &table).unwrap(),
                "{options:?}"
            );
        }
    }
}

/// A test reads each column once, gives an empty selection's answer without reading a row, and
/// fails for a column that the rows do not have.
#[test]
fn a_prepared_test_reads_its_columns() {
    let state = SelectionState::new(
        Resolution::Union,
        vec![
            Clause::new("a").conditions([range("x", 0.0, 6.0), range("y", 0.0, 5.0)]),
            Clause::new("b").condition(range("x", 8.0, 9.0)),
        ],
    );
    let types = |name: &str| (name != "z").then_some(DataType::Float64);
    let prepared = state
        .prepare_selected(&SelectedOptions::default(), types)
        .unwrap();
    assert_eq!(prepared.columns(), ["x", "y"]);
    assert_eq!(prepared.constant(), None);
    let empty = SelectionState::default();
    let none = SelectedOptions {
        empty_selects_none: true,
        ..Default::default()
    };
    let all = empty
        .prepare_selected(&SelectedOptions::default(), types)
        .unwrap();
    assert_eq!(all.constant(), Some(true));
    assert_eq!(
        empty.prepare_selected(&none, types).unwrap().constant(),
        Some(false)
    );
    let missing = SelectionState::new(
        Resolution::Global,
        vec![Clause::new("a").condition(range("z", 0.0, 1.0))],
    );
    assert!(missing
        .prepare_selected(&SelectedOptions::default(), types)
        .is_err());
    assert!(prepared.mask(2, |_| None, |_| None).is_err());
}

/// Tests of equal content have equal fingerprints, and tests of other conditions or options do
/// not.
#[test]
fn tests_fingerprint_their_content() {
    let hash = |state: &SelectionState, options: &SelectedOptions| {
        let types = |_: &str| Some(DataType::Float64);
        let prepared = state.prepare_selected(options, types).unwrap();
        prepared.fingerprint()
    };
    let state = |high: f64| {
        SelectionState::new(
            Resolution::Union,
            vec![
                Clause::new("a").condition(range("x", 0.0, high)),
                Clause::new("b").condition(range("y", 0.0, 5.0)),
            ],
        )
    };
    let plain = SelectedOptions::default();
    assert_eq!(hash(&state(6.0), &plain), hash(&state(6.0), &plain));
    assert_ne!(hash(&state(6.0), &plain), hash(&state(7.0), &plain));
    let without = SelectedOptions {
        without: vec!["b".into()],
        ..Default::default()
    };
    assert_ne!(hash(&state(6.0), &plain), hash(&state(6.0), &without));
    // A test that skips the clauses of `a` does not read them, whatever they hold.
    let skip_a = SelectedOptions {
        without: vec!["a".into()],
        ..Default::default()
    };
    assert_eq!(hash(&state(6.0), &skip_a), hash(&state(7.0), &skip_a));

    // Clauses differ by their owners, partitions, and conditions, and grids by what `same`
    // compares.
    let clause = Clause::new("a").condition(range("x", 0.0, 6.0));
    assert_eq!(clause.fingerprint(), clause.clone().fingerprint());
    assert_ne!(
        clause.fingerprint(),
        Clause::new("b")
            .condition(range("x", 0.0, 6.0))
            .fingerprint()
    );
    assert_ne!(
        clause.fingerprint(),
        clause.clone().partition("island", "dream").fingerprint()
    );
    let grid = |cell: f64| {
        let scale =
            avenger_scales::scales::linear::LinearScale::configured((0.0, 10.0), (0.0, 100.0));
        avenger_selection::PixelGrid::new(scale, cell).unwrap()
    };
    assert_eq!(grid(5.0).fingerprint(), grid(5.0).fingerprint());
    assert_ne!(grid(5.0).fingerprint(), grid(10.0).fingerprint());
}
