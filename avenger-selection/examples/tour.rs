//! A tour of avenger-selection through a small dashboard of penguins: a scatter plot of bill
//! length against flipper length, faceted by island, and a bar chart of the species. Each step
//! prints the ids of the rows that a selection selects.
//!
//! Run with `cargo run --release -p avenger-selection --example tour`.

use std::{collections::BTreeMap, sync::Arc};

use arrow::array::{
    ArrayRef, AsArray, BooleanArray, Float64Array, Int64Array, RecordBatch, StringArray,
};
use arrow::datatypes::Int64Type;
use avenger_scales::scales::linear::LinearScale;
use avenger_selection::{
    Clause, Condition, PixelGrid, PolygonCells, Predicate, Resolution, SelectedOptions,
    SelectionState,
};

fn main() -> avenger_selection::Result<()> {
    let penguins = penguins();
    let ids = |mask: &BooleanArray| -> Vec<i64> {
        let id = penguins
            .column_by_name("id")
            .unwrap()
            .as_primitive::<Int64Type>();
        (0..mask.len())
            .filter(|&row| mask.value(row))
            .map(|row| id.value(row))
            .collect()
    };
    let plain = SelectedOptions::default();

    // 1. A plot's tool writes a clause: the conditions that a row must match, all of them. A
    // brush on the scatter writes two ranges, and a click on the bars writes a set.
    let brush = Clause::new("scatter")
        .conditions([range("bill", 40.2, 47.8), range("flipper", 190.0, 210.0)]);
    let click = Clause::new("bars").condition(Condition::values(
        "species",
        Some("x".into()),
        Arc::new(StringArray::from(vec!["Gentoo"])),
    ));
    println!("1. Clauses");
    for clause in [&brush, &click] {
        let state = SelectionState::new(Resolution::Global, vec![clause.clone()]);
        println!(
            "   {:<7} selects {:?}",
            clause.owner,
            ids(&state.mask(&plain, &penguins)?)
        );
    }

    // 2. A selection's state holds the clauses of its owners and how they combine: a row matches
    // an intersect selection when it matches every owner's clause, and a union selection when it
    // matches one.
    println!("2. Resolutions");
    for resolution in [Resolution::Intersect, Resolution::Union] {
        let state = SelectionState::new(resolution, vec![brush.clone(), click.clone()]);
        println!(
            "   {:<9} selects {:?}",
            format!("{resolution:?}"),
            ids(&state.mask(&plain, &penguins)?)
        );
    }

    // 3. `selected` options decide what a consumer of the selection sees. A cross filter skips
    // the clauses of the plot that draws the rows, so each plot shows the other plot's filter.
    // `without` skips named owners, and an empty selection selects every row, or none.
    println!("3. Options");
    let state = SelectionState::new(Resolution::Intersect, vec![brush.clone(), click.clone()]);
    for owner in ["scatter", "bars"] {
        let options = SelectedOptions {
            exclude_own: vec![owner.into()],
            ..Default::default()
        };
        println!(
            "   the {owner} plot shows {:?}",
            ids(&state.mask(&options, &penguins)?)
        );
    }
    let without = SelectedOptions {
        without: vec!["bars".into()],
        ..Default::default()
    };
    println!(
        "   without bars, {:?}",
        ids(&state.mask(&without, &penguins)?)
    );
    let empty = SelectionState::default();
    let none = SelectedOptions {
        empty_selects_none: true,
        ..Default::default()
    };
    println!(
        "   an empty selection selects {} rows, or {} with empty_selects_none",
        ids(&empty.mask(&plain, &penguins)?).len(),
        ids(&empty.mask(&none, &penguins)?).len()
    );

    // 4. A faceted plot writes a clause for each panel, with the panel's partition values. With
    // `faceted`, a row skips only the clause of its own panel, so a brush in the Biscoe panel
    // filters the Dream panel and not the other way around.
    println!("4. Facets");
    let panel = |island: &str, bill: (f64, f64), flipper: (f64, f64)| {
        Clause::new("scatter")
            .partition("island", island)
            .conditions([
                range("bill", bill.0, bill.1),
                range("flipper", flipper.0, flipper.1),
            ])
    };
    let panels = SelectionState::new(
        Resolution::Union,
        vec![
            panel("Biscoe", (44.0, 52.0), (200.0, 215.0)),
            panel("Dream", (45.0, 52.0), (190.0, 200.0)),
        ],
    );
    println!(
        "   other plots show {:?}",
        ids(&panels.mask(&plain, &penguins)?)
    );
    let faceted = SelectedOptions {
        exclude_own: vec!["scatter".into()],
        faceted: true,
        ..Default::default()
    };
    // Row 11, in the Dream panel, is inside the Biscoe brush. No Biscoe row is inside the Dream
    // brush, and the Torgersen row is inside neither.
    println!(
        "   the scatter shows {:?}",
        ids(&panels.mask(&faceted, &penguins)?)
    );

    // 5. A brush can select whole pixel cells. A grid divides the range of a dimension's scale
    // into cells, and a range with cells matches every value in the cells that the box covers, so
    // a query that groups rows by their cells keeps exactly the rows that the renderer styles.
    // The bill axis is 300 pixels for 30 to 60 mm, with cells of 10 pixels, 1 mm each.
    println!("5. Pixel grids");
    let bill = PixelGrid::new(LinearScale::configured((30.0, 60.0), (0.0, 300.0)), 10.0)?;
    let box_ = (102.0, 178.0);
    let snapped = Condition::Range {
        column: "bill".into(),
        channel: Some("x".into()),
        ends: Arc::new(Float64Array::from(vec![40.2, 47.8])),
        exclude_end: false,
        cells: Some(bill.clone().span(box_.0, box_.1)?),
    };
    for (label, condition) in [
        ("ends 40.2 to 47.8", range("bill", 40.2, 47.8)),
        ("cells 10 to 17", snapped),
    ] {
        let state = SelectionState::new(
            Resolution::Global,
            vec![Clause::new("scatter").condition(condition)],
        );
        println!(
            "   {label:<18} selects {:?}",
            ids(&state.mask(&plain, &penguins)?)
        );
    }
    // A lasso covers the cells whose centers are inside its path, in pixels, and matches the
    // points in those cells. The flipper axis is 300 pixels for 230 down to 170 mm, from the top.
    let flipper = PixelGrid::new(LinearScale::configured((170.0, 230.0), (300.0, 0.0)), 10.0)?;
    let path = vec![[130.0, 140.0], [220.0, 140.0], [175.0, 30.0]];
    let cells = PolygonCells::cover([bill, flipper], std::slice::from_ref(&path));
    let lasso = Condition::Polygon {
        x_column: "bill".into(),
        y_column: "flipper".into(),
        polygons: vec![pixels_to_values(&path)],
        cells: Some(cells.clone()),
    };
    let state = SelectionState::new(
        Resolution::Global,
        vec![Clause::new("scatter").condition(lasso)],
    );
    println!(
        "   a lasso over {} rows of cells selects {:?}",
        cells.rows.len(),
        ids(&state.mask(&plain, &penguins)?)
    );

    // 6. A query prepares the test once, for the types of its columns, and matches each batch:
    // the sets' indexes and the lassos' cells are built once. This is the test that
    // avenger-selection-datafusion's `selected` runs on each batch of a query.
    println!("6. Prepared tests");
    let state = SelectionState::new(Resolution::Union, vec![brush.clone(), click.clone()]);
    let schema = penguins.schema();
    let types = |name: &str| (schema.field_with_name(name).ok()).map(|f| f.data_type().clone());
    let prepared = state.prepare_selected(&plain, types)?;
    println!("   the test reads {:?}", prepared.columns());
    for (index, batch) in [penguins.slice(0, 6), penguins.slice(6, 6)]
        .iter()
        .enumerate()
    {
        let column = |name: &str| batch.column_by_name(name).cloned();
        let mask = prepared.mask(batch.num_rows(), column, column)?;
        let id = batch
            .column_by_name("id")
            .unwrap()
            .as_primitive::<Int64Type>();
        let selected: Vec<i64> = (0..mask.len())
            .filter(|&row| mask.value(row))
            .map(|row| id.value(row))
            .collect();
        println!("   batch {index} selects {selected:?}");
    }

    // 7. A predicate combines several selections, each with the options of its own test, and
    // prepares once like the test of one selection. A plot that styles the rows of the brush
    // without the clicked species, and skips its own brush, as a cross filter:
    println!("7. Predicates over selections");
    let states = BTreeMap::from([
        (
            "brush".to_string(),
            SelectionState::new(Resolution::Global, vec![brush]),
        ),
        (
            "click".to_string(),
            SelectionState::new(Resolution::Global, vec![click]),
        ),
    ]);
    let not_clicked = Predicate::Not(Box::new(Predicate::selected("click", plain.clone())));
    let predicate = Predicate::And(vec![
        Predicate::selected("brush", plain.clone()),
        not_clicked,
    ]);
    let mask = predicate.mask(&states, &penguins)?;
    println!("   brush and not click selects {:?}", ids(&mask));
    Ok(())
}

/// Twelve penguins: `id`, `species`, `island`, and bill and flipper lengths in millimeters.
fn penguins() -> RecordBatch {
    let rows = [
        ("Adelie", "Torgersen", 39.1, 181.0),
        ("Adelie", "Biscoe", 37.8, 188.0),
        ("Adelie", "Dream", 40.6, 195.0),
        ("Adelie", "Biscoe", 41.1, 199.0),
        ("Gentoo", "Biscoe", 46.1, 211.0),
        ("Gentoo", "Biscoe", 50.0, 220.0),
        ("Gentoo", "Biscoe", 44.5, 205.0),
        ("Gentoo", "Biscoe", 48.7, 210.0),
        ("Chinstrap", "Dream", 46.5, 192.0),
        ("Chinstrap", "Dream", 50.9, 196.0),
        ("Chinstrap", "Dream", 47.9, 198.0),
        ("Chinstrap", "Dream", 52.0, 201.0),
    ];
    RecordBatch::try_from_iter([
        (
            "id",
            Arc::new(Int64Array::from_iter_values(0..rows.len() as i64)) as ArrayRef,
        ),
        (
            "species",
            Arc::new(StringArray::from_iter_values(rows.iter().map(|r| r.0))),
        ),
        (
            "island",
            Arc::new(StringArray::from_iter_values(rows.iter().map(|r| r.1))),
        ),
        (
            "bill",
            Arc::new(Float64Array::from_iter_values(rows.iter().map(|r| r.2))),
        ),
        (
            "flipper",
            Arc::new(Float64Array::from_iter_values(rows.iter().map(|r| r.3))),
        ),
    ])
    .unwrap()
}

/// A range of `column` from `low` through `high`.
fn range(column: &str, low: f64, high: f64) -> Condition {
    Condition::Range {
        column: column.into(),
        channel: None,
        ends: Arc::new(Float64Array::from(vec![low, high])),
        exclude_end: false,
        cells: None,
    }
}

/// A lasso's path in pixels as the bill and flipper values of its vertices, which a polygon
/// condition keeps beside its cells.
fn pixels_to_values(path: &[[f64; 2]]) -> (ArrayRef, ArrayRef) {
    let bill = path.iter().map(|[x, _]| 30.0 + x / 10.0);
    let flipper = path.iter().map(|[_, y]| 230.0 - y / 5.0);
    (
        Arc::new(Float64Array::from_iter_values(bill)),
        Arc::new(Float64Array::from_iter_values(flipper)),
    )
}
