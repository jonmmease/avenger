//! `selected` lowers to filters that keep the rows that avenger-selection's test of the state
//! selects.

use std::sync::Arc;

use avenger_scales::scales::linear::LinearScale;
use avenger_selection::{
    Clause, Condition, PixelGrid, PolygonCells, Resolution, SelectedOptions, SelectionState,
};
use datafusion::{
    arrow::{
        array::{
            Array, ArrayRef, AsArray, DictionaryArray, Float64Array, Int64Array, RecordBatch,
            StringArray,
        },
        datatypes::Int32Type,
    },
    common::{ParamValues, Result, ScalarValue},
    dataframe::DataFrame,
    prelude::SessionContext,
};

const ROWS: usize = 24;

/// The rows: `id`, `x` from 0 to 11.5 with a null and a NaN, `y`, `species`, a dictionary of three
/// values, `island`, the panel, and `lon`, longitudes across the antimeridian.
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
        .map(|i| (i != 11).then_some(["biscoe", "dream"][i % 2]))
        .collect();
    let lon: Vec<f64> = (0..ROWS).map(|i| -180.0 + i as f64 * 15.7).collect();
    RecordBatch::try_from_iter([
        (
            "id",
            Arc::new(Int64Array::from_iter_values(0..ROWS as i64)) as ArrayRef,
        ),
        ("x", Arc::new(Float64Array::from(x))),
        ("y", Arc::new(Float64Array::from(y))),
        ("species", Arc::new(species)),
        ("island", Arc::new(StringArray::from(island))),
        ("lon", Arc::new(Float64Array::from(lon))),
    ])
    .unwrap()
}

fn context() -> SessionContext {
    let mut ctx = SessionContext::new();
    avenger_selection_datafusion::register(&mut ctx).unwrap();
    ctx.register_batch("t", table()).unwrap();
    ctx
}

fn numbers(values: &[f64]) -> ArrayRef {
    Arc::new(Float64Array::from(values.to_vec()))
}

fn range(column: &str, low: f64, high: f64, exclude_end: bool) -> Condition {
    Condition::Range {
        column: column.into(),
        channel: None,
        ends: numbers(&[low, high]),
        exclude_end,
        cells: None,
    }
}

/// A range whose absent end does not bound it.
fn one_end(column: &str, low: Option<f64>, high: Option<f64>, exclude_end: bool) -> Condition {
    Condition::Range {
        column: column.into(),
        channel: None,
        ends: Arc::new(Float64Array::from(vec![low, high])),
        exclude_end,
        cells: None,
    }
}

/// A grid of `x` over the domain 0 to 12 on 240 pixels, with cells of 10 pixels, and the cells of
/// a box from `low` to `high` pixels.
fn gridded(low: f64, high: f64) -> Condition {
    let grid = PixelGrid::new(LinearScale::configured((0.0, 12.0), (0.0, 240.0)), 10.0).unwrap();
    Condition::Range {
        column: "x".into(),
        channel: Some("x".into()),
        ends: numbers(&[low / 20.0, high / 20.0]),
        exclude_end: false,
        cells: Some(grid.span(low, high).unwrap()),
    }
}

/// A lasso of the pixel path `path` over grids of `x`, the domain 0 to 12 on 240 pixels, and of
/// `y`, the domain 0 to 10 on 200 pixels from the top, with cells of 20 pixels, which selects the
/// cells that the path covers.
fn gridded_lasso(path: &[[f64; 2]]) -> Condition {
    let x = PixelGrid::new(LinearScale::configured((0.0, 12.0), (0.0, 240.0)), 20.0).unwrap();
    let y = PixelGrid::new(LinearScale::configured((0.0, 10.0), (200.0, 0.0)), 20.0).unwrap();
    Condition::Polygon {
        x_column: "x".into(),
        y_column: "y".into(),
        polygons: vec![(
            numbers(&path.iter().map(|[x, _]| x / 20.0).collect::<Vec<_>>()),
            numbers(
                &path
                    .iter()
                    .map(|[_, y]| (200.0 - y) / 20.0)
                    .collect::<Vec<_>>(),
            ),
        )],
        cells: Some(PolygonCells::cover([x, y], &[path.to_vec()])),
    }
}

/// The arguments of a call after the state, as SQL, and the same options for the expectation.
#[derive(Clone, Copy)]
struct Options {
    sql: &'static str,
    exclude_own: bool,
    owner: Option<&'static str>,
    faceted: bool,
    empty_selects_none: bool,
    without: &'static [&'static str],
}

const PLAIN: Options = Options {
    sql: "",
    exclude_own: false,
    owner: None,
    faceted: false,
    empty_selects_none: false,
    without: &[],
};

/// The frame of `sql` with the states of `states` bound by their placeholders' names.
async fn bound(
    ctx: &SessionContext,
    sql: &str,
    states: &[(&str, &SelectionState)],
) -> Result<DataFrame> {
    let plan = ctx.sql(sql).await?.into_unoptimized_plan();
    let plan = avenger_selection_datafusion::bind_selections(plan, states)?;
    ctx.execute_logical_plan(plan).await
}

/// The rows that `selected($s, <options>)` keeps, through DataFusion.
async fn lowered(state: &SelectionState, options: &Options) -> Vec<bool> {
    let sql = format!("SELECT selected($s{}) AS m FROM t ORDER BY id", options.sql);
    mask(bound(&context(), &sql, &[("s", state)]).await.unwrap()).await
}

/// The values of the first column of `frame`, which has no nulls.
async fn mask(frame: DataFrame) -> Vec<bool> {
    let batches = frame.collect().await.unwrap();
    let mut mask = Vec::new();
    for batch in batches {
        let column = batch.column(0).as_boolean();
        assert_eq!(column.null_count(), 0, "a lowered filter is never null");
        mask.extend(column.iter().map(Option::unwrap));
    }
    mask
}

/// The lowered call keeps exactly the rows that avenger-selection's test of the state selects for
/// the same options, which a renderer runs.
async fn conforms(state: SelectionState, options: Options) {
    let selected = SelectedOptions {
        exclude_own: (options.owner.filter(|_| options.exclude_own).into_iter())
            .map(str::to_string)
            .collect(),
        faceted: options.faceted,
        empty_selects_none: options.empty_selects_none,
        without: options
            .without
            .iter()
            .map(|owner| owner.to_string())
            .collect(),
    };
    let expected = state.mask(&selected, &table()).unwrap();
    assert_eq!(
        lowered(&state, &options).await,
        expected.values().iter().collect::<Vec<_>>(),
        "{}",
        options.sql
    );
}

#[tokio::test]
async fn each_kind_of_condition_conforms() {
    let species = Condition::values(
        "species",
        None,
        Arc::new(StringArray::from(vec![Some("gentoo"), None])),
    );
    let polygon = Condition::Polygon {
        x_column: "x".into(),
        y_column: "y".into(),
        polygons: vec![(
            numbers(&[1.0, 9.0, 9.0, 1.0]),
            numbers(&[1.0, 1.0, 6.0, 6.0]),
        )],
        cells: None,
    };
    let cyclic = Condition::CyclicRange {
        column: "lon".into(),
        channel: Some("longitude".into()),
        start: 150.0,
        end: -150.0,
        period: 360.0,
    };
    // Pairs of species and islands: the cross combination (gentoo, biscoe) is in no tuple.
    let pairs = Condition::set(
        vec!["species".into(), "island".into()],
        vec![Some("x".into()), Some("color".into())],
        vec![
            Arc::new(StringArray::from(vec!["adelie", "gentoo", "chinstrap"])),
            Arc::new(StringArray::from(vec![Some("biscoe"), Some("dream"), None])),
        ],
    )
    .unwrap();
    let lassos = Condition::Polygon {
        x_column: "x".into(),
        y_column: "y".into(),
        polygons: vec![
            (
                numbers(&[0.0, 4.0, 4.0, 0.0]),
                numbers(&[0.0, 0.0, 5.0, 5.0]),
            ),
            (
                numbers(&[3.0, 9.0, 9.0, 3.0]),
                numbers(&[4.0, 4.0, 8.0, 8.0]),
            ),
        ],
        cells: None,
    };
    // The cell of the row with `x` 1 and `y` 4, the pixels 20 to 40 and 120 to 140, whose center
    // the small square holds, while the row is outside the square itself.
    let cell_lasso = gridded_lasso(&[[25.0, 125.0], [35.0, 125.0], [35.0, 135.0], [25.0, 135.0]]);
    let triangle = gridded_lasso(&[[10.0, 10.0], [230.0, 30.0], [60.0, 190.0]]);
    let conditions = [
        species,
        pairs,
        lassos,
        range("x", 2.0, 6.0, false),
        range("x", 2.0, 6.0, true),
        // Ranges with one end, as a slider at the end of its domain writes them.
        one_end("x", Some(2.0), None, false),
        one_end("x", None, Some(6.0), true),
        gridded(40.0, 120.0),
        gridded(45.5, 119.0),
        polygon,
        cell_lasso,
        triangle,
        cyclic,
    ];
    for condition in conditions {
        let state = SelectionState::new(
            Resolution::Global,
            vec![Clause::new("a").condition(condition)],
        );
        conforms(state, PLAIN).await;
    }
}

#[tokio::test]
async fn values_cast_to_their_columns() {
    // Integer set values and range ends test a column of floats.
    let set = Condition::values("x", None, Arc::new(Int64Array::from(vec![2, 3])));
    let ends = Condition::Range {
        column: "y".into(),
        channel: None,
        ends: Arc::new(Int64Array::from(vec![2, 5])),
        exclude_end: false,
        cells: None,
    };
    for condition in [set, ends] {
        let state = SelectionState::new(
            Resolution::Global,
            vec![Clause::new("a").condition(condition)],
        );
        conforms(state, PLAIN).await;
    }
}

#[tokio::test]
async fn conditions_and_owners_combine() {
    let boxes =
        Clause::new("a").conditions([range("x", 0.0, 6.0, false), range("y", 0.0, 5.0, false)]);
    let other = Clause::new("b").conditions([range("y", 3.0, 8.0, false)]);
    for resolution in [Resolution::Global, Resolution::Union, Resolution::Intersect] {
        let state = SelectionState::new(resolution, vec![boxes.clone(), other.clone()]);
        conforms(state, PLAIN).await;
    }
}

#[tokio::test]
async fn options_conform() {
    let a = Clause::new("a").conditions([range("x", 0.0, 4.0, false)]);
    let b = Clause::new("b").conditions([range("y", 3.0, 8.0, false)]);
    let state = SelectionState::new(Resolution::Intersect, vec![a, b]);
    let cases = [
        Options {
            sql: ", exclude_own => true, owner => 'a'",
            exclude_own: true,
            owner: Some("a"),
            ..PLAIN
        },
        Options {
            sql: ", NULL, NULL, NULL, NULL, ['b']",
            without: &["b"],
            ..PLAIN
        },
        Options {
            sql: ", NULL, NULL, NULL, NULL, ['a', 'b']",
            without: &["a", "b"],
            ..PLAIN
        },
    ];
    for options in cases {
        conforms(state.clone(), options).await;
    }
    for (sql, empty_selects_none) in [
        (", NULL, NULL, NULL, 'all'", false),
        (", NULL, NULL, NULL, 'none'", true),
    ] {
        let options = Options {
            sql,
            empty_selects_none,
            ..PLAIN
        };
        conforms(state.clone(), options).await;
        conforms(SelectionState::default(), options).await;
    }
}

#[tokio::test]
async fn a_faceted_owner_skips_only_its_own_panel() {
    let panel = |island: &str, low: f64, high: f64| {
        Clause::new("panels")
            .partition("island", island)
            .conditions([range("x", low, high, false)])
    };
    let other = Clause::new("other").conditions([range("y", 2.0, 9.0, false)]);
    for resolution in [Resolution::Union, Resolution::Intersect] {
        let state = SelectionState::new(
            resolution,
            vec![
                panel("biscoe", 0.0, 3.0),
                panel("dream", 6.0, 9.0),
                other.clone(),
            ],
        );
        let options = Options {
            sql: ", exclude_own => true, owner => 'panels', partition => named_struct('island', island)",
            exclude_own: true,
            owner: Some("panels"),
            faceted: true,
            ..PLAIN
        };
        conforms(state, options).await;
    }
}

#[tokio::test]
async fn named_arguments_form_prefixes() {
    let ctx = context();
    let state = SelectionState::new(
        Resolution::Global,
        vec![Clause::new("a").conditions([range("x", 0.0, 4.0, false)])],
    );
    let run = |sql: &'static str| {
        let (ctx, state) = (ctx.clone(), state.clone());
        async move { bound(&ctx, sql, &[("s", &state)]).await?.collect().await }
    };
    let valid = [
        "SELECT count(*) FROM t WHERE selected($s)",
        "SELECT count(*) FROM t WHERE selected(state => $s, exclude_own => false)",
        "SELECT count(*) FROM t WHERE selected($s, exclude_own => true, owner => 'b')",
        "SELECT count(*) FROM t WHERE selected(state => $s, exclude_own => NULL, owner => NULL, \
         partition => NULL, empty => NULL, without => NULL)",
    ];
    let mut counts = Vec::new();
    for sql in valid {
        let batches = run(sql)
            .await
            .unwrap_or_else(|error| panic!("{sql}: {error}"));
        counts.push(
            batches[0]
                .column(0)
                .as_primitive::<datafusion::arrow::datatypes::Int64Type>()
                .value(0),
        );
    }
    assert!(counts.iter().all(|&count| count == counts[0]), "{counts:?}");
    let skipped = run("SELECT count(*) FROM t WHERE selected($s, empty => 'none')").await;
    let error = skipped.unwrap_err().to_string();
    assert!(error.contains("exclude_own"), "{error}");
}

/// A plan that runs with a `selected` call that `bind_selections` did not lower fails with an
/// error that names the selection: unbound, bound under another name, or not a placeholder.
#[tokio::test]
async fn unbound_selections_name_their_selection() {
    let ctx = context();
    let sql = "SELECT count(*) FROM t WHERE selected($s)";
    let message = |error: datafusion::error::DataFusionError| error.find_root().to_string();
    let unbound = ctx.sql(sql).await.unwrap().collect().await.unwrap_err();
    assert!(message(unbound).ends_with("selection `s` was not registered"));
    let state = SelectionState::default();
    let frame = bound(&ctx, sql, &[("brush", &state)]).await.unwrap();
    let elsewhere = frame.collect().await.unwrap_err();
    assert!(message(elsewhere).ends_with("selection `s` was not registered"));
    let literal = "SELECT count(*) FROM t WHERE selected(NULL)";
    let frame = bound(&ctx, literal, &[("s", &state)]).await.unwrap();
    let literal = message(frame.collect().await.unwrap_err());
    assert!(
        literal.contains("takes the placeholder of a selection"),
        "{literal}"
    );
}

/// A lowered filter matches every batch of a table through its prepared condition, also batches
/// whose dictionaries differ.
#[tokio::test]
async fn prepared_conditions_match_every_batch() {
    use datafusion::{
        arrow::datatypes::{DataType, Field, Schema},
        datasource::MemTable,
    };
    let dictionary = DataType::Dictionary(Box::new(DataType::Int32), Box::new(DataType::Utf8));
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("species", dictionary.clone(), true),
    ]));
    let batch = |ids: std::ops::Range<i64>, species: &[Option<&str>]| {
        let species: DictionaryArray<Int32Type> = species.iter().copied().collect();
        RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(Int64Array::from_iter_values(ids)),
                Arc::new(species),
            ],
        )
        .unwrap()
    };
    let batches = [
        batch(0..4, &[Some("b"), Some("a"), Some("c"), None]),
        batch(4..8, &[Some("d"), Some("d"), Some("c"), Some("e")]),
        batch(8..12, &[Some("a"), None, Some("b"), Some("d")]),
    ];
    let mut ctx = SessionContext::new();
    avenger_selection_datafusion::register(&mut ctx).unwrap();
    let table = MemTable::try_new(
        schema.clone(),
        vec![batches[..2].to_vec(), batches[2..].to_vec()],
    );
    ctx.register_table("t", Arc::new(table.unwrap())).unwrap();
    let set = Condition::values("species", None, Arc::new(StringArray::from(vec!["a", "d"])));
    let state = SelectionState::new(Resolution::Global, vec![Clause::new("a").condition(set)]);
    let sql = "SELECT id FROM t WHERE selected($s)";
    let frame = bound(&ctx, sql, &[("s", &state)]).await.unwrap();
    let mut ids: Vec<i64> = Vec::new();
    for batch in frame.collect().await.unwrap() {
        ids.extend(
            batch
                .column(0)
                .as_primitive::<datafusion::arrow::datatypes::Int64Type>()
                .values(),
        );
    }
    ids.sort();
    assert_eq!(ids, [1, 4, 5, 8, 11]);
}

/// Two selections of different conditions over one column keep their own rows and their own
/// names in one projection, though each lowers to a call of the same columns, which the optimizer
/// would merge if the calls compared equal: cyclic ranges of other ends, and sets of other values
/// of one length.
#[tokio::test]
async fn different_conditions_on_one_column_stay_apart() {
    let ctx = context();
    let cyclic = |start: f64, end: f64| Condition::CyclicRange {
        column: "lon".into(),
        channel: None,
        start,
        end,
        period: 360.0,
    };
    // The sets hold the longitudes of the rows of `rows`.
    let set = |rows: [usize; 2]| {
        let values = rows.map(|row| -180.0 + row as f64 * 15.7);
        Condition::values("lon", None, numbers(&values))
    };
    let pairs = [
        (cyclic(150.0, -150.0), cyclic(-60.0, 60.0)),
        (set([1, 2]), set([1, 3])),
    ];
    let lon = table().column_by_name("lon").unwrap().clone();
    for (a, b) in pairs {
        let state = |condition: &Condition| {
            SelectionState::new(
                Resolution::Global,
                vec![Clause::new("a").condition(condition.clone())],
            )
        };
        let (state_a, state_b) = (state(&a), state(&b));
        let sql = "SELECT selected($a), selected($b) FROM t ORDER BY id";
        let frame = bound(&ctx, sql, &[("a", &state_a), ("b", &state_b)])
            .await
            .unwrap();
        let batch = datafusion::arrow::compute::concat_batches(
            &frame.schema().as_arrow().clone().into(),
            &frame.collect().await.unwrap(),
        )
        .unwrap();
        let names: Vec<&String> = batch
            .schema_ref()
            .fields()
            .iter()
            .map(|f| f.name())
            .collect();
        assert_ne!(names[0], names[1]);
        assert!(names.iter().all(|name| name.len() < 100), "{names:?}");
        let column = |index: usize| -> Vec<bool> {
            let mask = batch.column(index).as_boolean();
            mask.iter()
                .map(|matched| matched.unwrap_or(false))
                .collect()
        };
        let expected = |condition: &Condition| -> Vec<bool> {
            let mask = condition.matches(std::slice::from_ref(&lon)).unwrap();
            mask.values().iter().collect()
        };
        assert_eq!(column(0), expected(&a), "{a:?}");
        assert_eq!(column(1), expected(&b), "{b:?}");
        assert_ne!(column(0), column(1), "{a:?} and {b:?}");
    }
}

/// `bind_selections` binds the calls on the placeholders that it names, keeps the names of a
/// projection's columns, and leaves other placeholders to `with_param_values`.
#[tokio::test]
async fn bound_states_keep_names_and_other_placeholders() {
    let ctx = context();
    let set = Condition::values("x", None, numbers(&[1.0, 2.5, 3.0]));
    let state = SelectionState::new(
        Resolution::Global,
        vec![Clause::new("a").condition(set.clone())],
    );
    let plan = ctx
        .sql("SELECT selected($s) AS m, id = $n AS n FROM t ORDER BY id")
        .await
        .unwrap()
        .into_unoptimized_plan();
    let plan = avenger_selection_datafusion::bind_selections(plan, &[("s", &state)]).unwrap();
    let names: Vec<String> = (plan.schema().fields().iter())
        .map(|field| field.name().clone())
        .collect();
    assert_eq!(names, ["m", "n"]);
    let plan = plan
        .with_param_values(ParamValues::Map(
            [("n".to_string(), ScalarValue::Int64(Some(4)).into())]
                .into_iter()
                .collect(),
        ))
        .unwrap();
    let batch = datafusion::arrow::compute::concat_batches(
        &Arc::new(plan.schema().as_arrow().clone()),
        &ctx.execute_logical_plan(plan)
            .await
            .unwrap()
            .collect()
            .await
            .unwrap(),
    )
    .unwrap();
    let x = table().column_by_name("x").unwrap().clone();
    let expected: Vec<bool> = (set
        .matches(std::slice::from_ref(&x))
        .unwrap()
        .values()
        .iter())
    .collect();
    let matched: Vec<bool> = (batch.column(0).as_boolean().iter())
        .map(|matched| matched.unwrap_or(false))
        .collect();
    assert_eq!(matched, expected);
    let found: Vec<bool> = (batch.column(1).as_boolean().iter())
        .map(|found| found.unwrap())
        .collect();
    assert_eq!(found.iter().filter(|found| **found).count(), 1);
}

#[tokio::test]
async fn a_set_with_a_null_lowers_as_its_sql() {
    // A null equals a null, as a click on the bar of rows without an island writes it.
    let set = |values: Vec<Option<&str>>| {
        Condition::values("island", None, Arc::new(StringArray::from(values)))
    };
    let cases = [
        (
            set(vec![Some("biscoe"), None]),
            "island IN ('biscoe') OR island IS NULL",
        ),
        (set(vec![None]), "island IS NULL"),
        (set(vec![Some("biscoe")]), "island IN ('biscoe')"),
    ];
    for (condition, sql) in cases {
        let state = SelectionState::new(
            Resolution::Global,
            vec![Clause::new("a").condition(condition)],
        );
        conforms(state.clone(), PLAIN).await;
        let expected = format!("SELECT coalesce({sql}, false) AS m FROM t ORDER BY id");
        let expected = mask(context().sql(&expected).await.unwrap()).await;
        assert_eq!(lowered(&state, &PLAIN).await, expected, "{sql}");
    }
}

#[tokio::test]
async fn a_complement_lowers_as_its_sql() {
    // Everything except the kept islands, as a click on the Other group of a lump writes it: the
    // row of a null island matches unless the kept islands hold null.
    let complement = |values: Vec<Option<&str>>| {
        Condition::values("island", None, Arc::new(StringArray::from(values)))
            .complement()
            .unwrap()
    };
    let cases = [
        (
            complement(vec![Some("biscoe")]),
            "island NOT IN ('biscoe') OR island IS NULL",
        ),
        (
            complement(vec![Some("biscoe"), None]),
            "island NOT IN ('biscoe') AND island IS NOT NULL",
        ),
        (complement(vec![]), "true"),
    ];
    for (condition, sql) in cases {
        let state = SelectionState::new(
            Resolution::Global,
            vec![Clause::new("a").condition(condition)],
        );
        conforms(state.clone(), PLAIN).await;
        let expected = format!("SELECT coalesce({sql}, false) AS m FROM t ORDER BY id");
        let expected = mask(context().sql(&expected).await.unwrap()).await;
        assert_eq!(lowered(&state, &PLAIN).await, expected, "{sql}");
    }
    // A dictionary column takes a complement of its values.
    let species = Condition::values("species", None, Arc::new(StringArray::from(vec!["adelie"])))
        .complement()
        .unwrap();
    let state = SelectionState::new(
        Resolution::Global,
        vec![Clause::new("a").condition(species)],
    );
    conforms(state, PLAIN).await;
}
