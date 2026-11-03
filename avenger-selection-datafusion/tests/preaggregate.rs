//! The pre-aggregation's contract: for every state that the guard accepts, the prepared plan
//! returns the rows of the direct plan, within the rounding of regrouped floating-point arithmetic,
//! and its cells subplans do not read the moving clauses.

use std::sync::Arc;

use avenger_scales::scales::linear::LinearScale;
use avenger_selection::{Clause, Condition, PixelGrid, PolygonCells, Resolution, SelectionState};
use avenger_selection_datafusion::{
    bind_selections,
    preaggregate::{prepare, DirectReason, Focus, FocusDimension, Preaggregation, Strategy},
    selection_reads, Owners, SelectionRead,
};
use datafusion::{
    arrow::{
        array::{Array, ArrayRef, AsArray, Float64Array, Int64Array, RecordBatch, StringArray},
        compute::{cast, concat_batches},
        datatypes::DataType,
        util::display::array_value_to_string,
    },
    common::tree_node::TreeNodeRecursion,
    logical_expr::{Expr, LogicalPlan},
    prelude::SessionContext,
};

const ROWS: usize = 400;

/// A small generator of the same numbers on every run.
struct Numbers(u64);

impl Numbers {
    fn next(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }

    fn between(&mut self, low: f64, high: f64) -> f64 {
        low + self.next() * (high - low)
    }
}

/// Flights: a delay from -30 to 110 minutes with nulls, an air time, a distance, and a carrier.
fn table() -> RecordBatch {
    let mut numbers = Numbers(7);
    let delay: Vec<Option<f64>> = (0..ROWS)
        .map(|i| (i % 37 != 0).then(|| numbers.between(-30.0, 110.0).round()))
        .collect();
    let air: Vec<f64> = (0..ROWS)
        .map(|_| numbers.between(20.0, 300.0).round())
        .collect();
    let distance: Vec<f64> = (0..ROWS).map(|_| numbers.between(100.0, 2500.0)).collect();
    let carrier: Vec<&str> = (0..ROWS)
        .map(|_| ["AA", "DL", "UA", "WN", "B6"][(numbers.next() * 5.0) as usize])
        .collect();
    RecordBatch::try_from_iter([
        (
            "id",
            Arc::new(Int64Array::from_iter_values(0..ROWS as i64)) as ArrayRef,
        ),
        ("delay", Arc::new(Float64Array::from(delay))),
        ("air_time", Arc::new(Float64Array::from(air))),
        ("distance", Arc::new(Float64Array::from(distance))),
        ("carrier", Arc::new(StringArray::from(carrier))),
    ])
    .unwrap()
}

fn context() -> SessionContext {
    let mut ctx = SessionContext::new();
    avenger_selection_datafusion::register(&mut ctx).unwrap();
    ctx.register_batch("flights", table()).unwrap();
    ctx
}

/// The grid of a delay plot 240 pixels wide over -20 to 100 minutes, in cells of 4 pixels.
fn delay_grid() -> PixelGrid {
    PixelGrid::new(LinearScale::configured((-20.0, 100.0), (0.0, 240.0)), 4.0).unwrap()
}

/// The grid of an air time plot 200 pixels tall over 0 to 320 minutes, in cells of 5 pixels.
fn air_grid() -> PixelGrid {
    PixelGrid::new(LinearScale::configured((0.0, 320.0), (200.0, 0.0)), 5.0).unwrap()
}

fn gridded(column: &str, grid: PixelGrid, low: f64, high: f64) -> Condition {
    let cells = grid.clone().span(low, high).unwrap();
    Condition::Range {
        column: column.into(),
        channel: None,
        ends: Arc::new(Float64Array::from(vec![low, high])),
        exclude_end: false,
        cells: Some(cells),
    }
}

fn exact(column: &str, low: f64, high: f64) -> Condition {
    Condition::Range {
        column: column.into(),
        channel: None,
        ends: Arc::new(Float64Array::from(vec![low, high])),
        exclude_end: false,
        cells: None,
    }
}

fn carriers(values: &[&str]) -> Condition {
    Condition::values(
        "carrier",
        None,
        Arc::new(StringArray::from(values.to_vec())),
    )
}

/// `plan` with `state` bound to `$s`.
fn bind(plan: LogicalPlan, state: &SelectionState) -> LogicalPlan {
    bind_selections(plan, &[("s", state)]).unwrap()
}

/// The cells subplans of a bound prepared plan: the input of each rollup's `in_keys` filter.
fn cells_subplans(plan: &LogicalPlan) -> Vec<LogicalPlan> {
    let mut cells = Vec::new();
    plan.apply_with_subqueries(|node| {
        if let LogicalPlan::Filter(filter) = node {
            if matches!(&filter.predicate, Expr::ScalarFunction(call) if call.name() == "in_keys") {
                cells.push(filter.input.as_ref().clone());
            }
        }
        Ok(TreeNodeRecursion::Continue)
    })
    .unwrap();
    cells
}

/// A row: its values as text, and its numbers apart, so rows compare within a tolerance.
#[derive(Debug)]
struct Row {
    text: Vec<String>,
    numbers: Vec<Option<f64>>,
}

impl PartialEq for Row {
    fn eq(&self, other: &Row) -> bool {
        self.text == other.text
            && self.numbers.len() == other.numbers.len()
            && (self.numbers.iter().zip(&other.numbers)).all(|pair| match pair {
                (Some(a), Some(b)) => (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0),
                (a, b) => a == b,
            })
    }
}

/// The rows of `plan` with `state`, sorted by their text and their numbers.
async fn rows(ctx: &SessionContext, plan: LogicalPlan, state: &SelectionState) -> Vec<Row> {
    rows_of(ctx, bind(plan, state)).await
}

/// The rows of the bound plan `plan`, sorted by their text and their numbers.
async fn rows_of(ctx: &SessionContext, plan: LogicalPlan) -> Vec<Row> {
    let frame = ctx.execute_logical_plan(plan).await.unwrap();
    let schema = Arc::new(frame.schema().as_arrow().clone());
    let batches = frame.collect().await.unwrap();
    // A batch's own schema, which a cast to a string type can make differ from the plan's.
    let schema = batches.first().map_or(schema, |batch| batch.schema());
    let batch = concat_batches(&schema, &batches).unwrap();
    let mut rows: Vec<Row> = (0..batch.num_rows())
        .map(|row| {
            let mut text = Vec::new();
            let mut numbers = Vec::new();
            for column in batch.columns() {
                if column.data_type() == &DataType::Float64 {
                    let values = column.as_primitive::<datafusion::arrow::datatypes::Float64Type>();
                    numbers.push(values.is_valid(row).then(|| values.value(row)));
                } else {
                    let column = cast(column, &DataType::Utf8).unwrap();
                    text.push(array_value_to_string(&column, row).unwrap());
                }
            }
            Row { text, numbers }
        })
        .collect();
    rows.sort_by(|a, b| {
        (a.text.cmp(&b.text)).then_with(|| {
            let rounded = |row: &Row| -> Vec<String> {
                (row.numbers.iter())
                    .map(|n| format!("{:.6e}", n.unwrap_or(f64::NAN)))
                    .collect()
            };
            rounded(a).cmp(&rounded(b))
        })
    });
    rows
}

/// The direct plan of `sql`, its prepared plan for `focus`, and its one pre-aggregation.
async fn prepared(
    ctx: &SessionContext,
    sql: &str,
    focus: &Focus,
) -> (LogicalPlan, LogicalPlan, Preaggregation) {
    let plan = ctx.sql(sql).await.unwrap().into_unoptimized_plan();
    let prepared = prepare(&plan, focus).unwrap();
    let [Strategy::Preaggregated(preaggregation)] = prepared.strategies.as_slice() else {
        panic!("{sql}: {:?}", prepared.strategies);
    };
    (plan, prepared.plan, preaggregation.clone())
}

/// For `states`, which share the fixed clauses, the prepared plan returns the rows of the direct
/// plan, and its cells subplan is the same for every state once bound.
async fn contract(sql: &str, focus: Focus, states: Vec<SelectionState>) {
    let ctx = context();
    let (plan, rewritten, preaggregation) = prepared(&ctx, sql, &focus).await;
    let mut cells = Vec::new();
    for state in &states {
        assert!(preaggregation.serves(state));
        let direct = rows(&ctx, plan.clone(), state).await;
        assert!(!direct.is_empty());
        assert_eq!(rows(&ctx, rewritten.clone(), state).await, direct, "{sql}");
        cells.push(cells_subplans(&bind(rewritten.clone(), state)));
    }
    assert!(cells.windows(2).all(|pair| pair[0] == pair[1]), "{sql}");
}

/// The moving owner `delay`'s box over the pixels `low` to `high`, with the fixed box of `air`.
fn delay_state(low: f64, high: f64) -> SelectionState {
    SelectionState::new(
        Resolution::Intersect,
        vec![
            Clause::new("delay").condition(gridded("delay", delay_grid(), low, high)),
            Clause::new("air").condition(exact("air_time", 60.0, 240.0)),
        ],
    )
}

fn delay_focus() -> Focus {
    Focus {
        parameter: "$s".into(),
        owner: "delay".into(),
        dimensions: vec![FocusDimension {
            column: "delay".into(),
            grid: Some(delay_grid()),
        }],
    }
}

/// States whose `delay` boxes cover random spans, a span at the plot's edge, and no box.
fn delay_states() -> Vec<SelectionState> {
    let mut numbers = Numbers(11);
    let mut states: Vec<SelectionState> = (0..4)
        .map(|_| {
            let a = numbers.between(0.0, 240.0);
            let b = numbers.between(0.0, 240.0);
            delay_state(a.min(b), a.max(b))
        })
        .collect();
    states.push(delay_state(0.0, 240.0));
    states.push(SelectionState::new(
        Resolution::Intersect,
        vec![Clause::new("air").condition(exact("air_time", 60.0, 240.0))],
    ));
    states
}

#[tokio::test]
async fn every_family_matches_the_direct_plan() {
    let sql = "SELECT carrier, count(*) AS n, sum(distance) AS total, min(distance) AS low, \
               max(distance) AS high, avg(distance) AS mean, var_samp(distance) AS vs, \
               var_pop(distance) AS vp, stddev(distance) AS ss, stddev_pop(distance) AS sp, \
               count(delay) AS delays \
               FROM flights WHERE selected($s, exclude_own => true, owner => 'carriers') \
               GROUP BY carrier";
    contract(sql, delay_focus(), delay_states()).await;
}

#[tokio::test]
async fn bins_filters_and_finishing_operators_match() {
    let queries = [
        // A bin of one row between the filter and the aggregate, and an aggregate filter.
        "SELECT bin, count(*) AS n, avg(distance) FILTER (WHERE carrier <> 'WN') AS mean \
         FROM (SELECT *, floor(distance / 250) * 250 AS bin FROM flights \
               WHERE selected($s, exclude_own => true, owner => 'distance')) \
         GROUP BY bin",
        // A window, an order, and a limit above the aggregate.
        "SELECT carrier, n, rank() OVER (ORDER BY n DESC, carrier) AS place FROM \
         (SELECT carrier, count(*) AS n FROM flights WHERE selected($s) GROUP BY carrier) \
         ORDER BY place LIMIT 3",
        // An outer aggregate.
        "SELECT max(n) AS most, sum(n) AS all_rows FROM \
         (SELECT carrier, count(*) AS n FROM flights WHERE selected($s) GROUP BY carrier)",
    ];
    for sql in queries {
        contract(sql, delay_focus(), delay_states()).await;
    }
}

#[tokio::test]
async fn two_gridded_dimensions_match() {
    let focus = Focus {
        parameter: "$s".into(),
        owner: "scatter".into(),
        dimensions: vec![
            FocusDimension {
                column: "delay".into(),
                grid: Some(delay_grid()),
            },
            FocusDimension {
                column: "air_time".into(),
                grid: Some(air_grid()),
            },
        ],
    };
    let state = |x: (f64, f64), y: (f64, f64)| {
        SelectionState::new(
            Resolution::Intersect,
            vec![Clause::new("scatter").conditions([
                gridded("delay", delay_grid(), x.0, x.1),
                gridded("air_time", air_grid(), y.0, y.1),
            ])],
        )
    };
    let states = vec![
        state((40.0, 180.0), (20.0, 150.0)),
        state((0.0, 240.0), (0.0, 200.0)),
        state((101.0, 133.0), (77.0, 78.0)),
    ];
    let sql = "SELECT carrier, count(*) AS n, avg(distance) AS mean FROM flights \
               WHERE selected($s) GROUP BY carrier";
    let ctx = context();
    let (_, _, preaggregation) = prepared(&ctx, sql, &focus).await;
    assert_eq!(preaggregation.size().cells_per_group, 60 * 40);
    contract(sql, focus, states).await;
}

/// The focus of a scatter plot of `delay` and `air_time`, with both of its grids.
fn scatter_focus() -> Focus {
    Focus {
        parameter: "$s".into(),
        owner: "scatter".into(),
        dimensions: vec![
            FocusDimension {
                column: "delay".into(),
                grid: Some(delay_grid()),
            },
            FocusDimension {
                column: "air_time".into(),
                grid: Some(air_grid()),
            },
        ],
    }
}

/// A lasso of the scatter plot along the pixel path `path`, which selects the cells of
/// `delay_grid` and `air_grid` that it covers, or of the grids `grids`.
fn lasso(path: &[[f64; 2]], grids: [PixelGrid; 2]) -> Condition {
    // The data of a pixel: delay over 240 pixels from -20, and air time over 200 pixels from the
    // top down to 0.
    let column = |values: Vec<f64>| -> ArrayRef { Arc::new(Float64Array::from(values)) };
    Condition::Polygon {
        x_column: "delay".into(),
        y_column: "air_time".into(),
        polygons: vec![(
            column(path.iter().map(|[x, _]| x * 0.5 - 20.0).collect()),
            column(path.iter().map(|[_, y]| (200.0 - y) * 1.6).collect()),
        )],
        cells: Some(PolygonCells::cover(grids, &[path.to_vec()])),
    }
}

/// The lassos of the scatter plot and a box of its two grids roll up from one cells table to the
/// rows of the direct plan.
#[tokio::test]
async fn lassos_match_from_the_cells_of_their_grids() {
    let lassoed = |path: &[[f64; 2]]| {
        SelectionState::new(
            Resolution::Intersect,
            vec![
                Clause::new("scatter").condition(lasso(path, [delay_grid(), air_grid()])),
                Clause::new("carriers").condition(carriers(&["AA", "DL", "UA"])),
            ],
        )
    };
    let mut numbers = Numbers(13);
    let jagged: Vec<[f64; 2]> = (0..60)
        .map(|index| {
            let angle = index as f64 / 60.0 * std::f64::consts::TAU;
            let radius = 40.0 + 50.0 * numbers.next();
            [120.0 + radius * angle.cos(), 100.0 + radius * angle.sin()]
        })
        .collect();
    let states = vec![
        lassoed(&[[10.0, 10.0], [230.0, 30.0], [60.0, 190.0]]),
        lassoed(&jagged),
        lassoed(&[[0.0, 0.0], [240.0, 0.0], [240.0, 200.0], [0.0, 200.0]]),
        SelectionState::new(
            Resolution::Intersect,
            vec![
                Clause::new("scatter").conditions([
                    gridded("delay", delay_grid(), 40.0, 180.0),
                    gridded("air_time", air_grid(), 20.0, 150.0),
                ]),
                Clause::new("carriers").condition(carriers(&["AA", "DL", "UA"])),
            ],
        ),
    ];
    let sql = "SELECT carrier, count(*) AS n, avg(distance) AS mean FROM flights \
               WHERE selected($s) GROUP BY carrier";
    contract(sql, scatter_focus(), states).await;
}

/// The guard rejects a lasso of other grids and a lasso without cells.
#[tokio::test]
async fn the_guard_rejects_lassos_that_the_cells_cannot_serve() {
    let ctx = context();
    let sql = "SELECT carrier, count(*) AS n FROM flights WHERE selected($s) GROUP BY carrier";
    let (_, _, preaggregation) = prepared(&ctx, sql, &scatter_focus()).await;
    let serves = |condition: Condition| {
        let state = SelectionState::new(
            Resolution::Intersect,
            vec![Clause::new("scatter").condition(condition)],
        );
        preaggregation.serves(&state)
    };
    let path = [[10.0, 10.0], [230.0, 30.0], [60.0, 190.0]];
    assert!(serves(lasso(&path, [delay_grid(), air_grid()])));
    let coarse = PixelGrid::new(LinearScale::configured((0.0, 320.0), (200.0, 0.0)), 10.0).unwrap();
    assert!(!serves(lasso(&path, [delay_grid(), coarse])));
    let Condition::Polygon {
        x_column,
        y_column,
        polygons,
        ..
    } = lasso(&path, [delay_grid(), air_grid()])
    else {
        unreachable!()
    };
    assert!(!serves(Condition::Polygon {
        x_column,
        y_column,
        polygons,
        cells: None,
    }));
}

#[tokio::test]
async fn sets_and_mixed_keys_match() {
    let set_focus = Focus {
        parameter: "$s".into(),
        owner: "bars".into(),
        dimensions: vec![FocusDimension {
            column: "carrier".into(),
            grid: None,
        }],
    };
    let picks = |values: &[&[&str]]| {
        let values: Vec<&str> = values
            .iter()
            .flat_map(|values| values.iter().copied())
            .collect();
        let clause = Clause::new("bars").condition(carriers(&values));
        SelectionState::new(Resolution::Intersect, vec![clause])
    };
    let sql = "SELECT floor(distance / 500) AS bin, count(*) AS n, sum(air_time) AS air \
               FROM flights WHERE selected($s) GROUP BY floor(distance / 500)";
    contract(
        sql,
        set_focus.clone(),
        vec![
            picks(&[&["AA"]]),
            picks(&[&["DL", "UA"]]),
            picks(&[&["WN"], &["B6"]]),
        ],
    )
    .await;
    let ctx = context();
    let (_, _, preaggregation) = prepared(&ctx, sql, &set_focus).await;
    assert_eq!(preaggregation.size().value_keys, ["carrier"]);

    let mixed_focus = Focus {
        parameter: "$s".into(),
        owner: "strip".into(),
        dimensions: vec![
            FocusDimension {
                column: "delay".into(),
                grid: Some(delay_grid()),
            },
            FocusDimension {
                column: "carrier".into(),
                grid: None,
            },
        ],
    };
    let strip = |low: f64, high: f64, values: &[&str]| {
        SelectionState::new(
            Resolution::Intersect,
            vec![Clause::new("strip")
                .conditions([gridded("delay", delay_grid(), low, high), carriers(values)])],
        )
    };
    contract(
        sql,
        mixed_focus,
        vec![
            strip(20.0, 200.0, &["AA", "UA"]),
            strip(0.0, 240.0, &["WN"]),
        ],
    )
    .await;
}

#[tokio::test]
async fn a_faceted_consumer_keeps_its_own_panels() {
    // The rows' own panel skips its clause, which the cells plan keeps as a fixed filter.
    let state = |low: f64, high: f64| {
        SelectionState::new(
            Resolution::Intersect,
            vec![
                Clause::new("delay").condition(gridded("delay", delay_grid(), low, high)),
                Clause::new("panels")
                    .partition("carrier", "AA")
                    .condition(exact("distance", 500.0, 1500.0)),
                Clause::new("panels")
                    .partition("carrier", "UA")
                    .condition(exact("air_time", 100.0, 200.0)),
            ],
        )
    };
    let sql = "SELECT carrier, count(*) AS n, avg(distance) AS mean FROM flights \
               WHERE selected($s, exclude_own => true, owner => 'panels', \
                              partition => named_struct('carrier', carrier)) \
               GROUP BY carrier";
    contract(
        sql,
        delay_focus(),
        vec![state(30.0, 90.0), state(100.0, 230.0)],
    )
    .await;
}

/// The cells subplan reads every owner's clauses but the moving owner's, and the rollup's filter
/// reads the moving owner's alone, so a dataflow cuts the plan between them. Bound, the cells
/// subplan is the same for two positions of the moving box, and the rollup is not.
#[tokio::test]
async fn the_cells_subplan_does_not_read_the_moving_clause() {
    let ctx = context();
    let sql = "SELECT carrier, count(*) AS n FROM flights WHERE selected($s) GROUP BY carrier";
    let (_, rewritten, _) = prepared(&ctx, sql, &delay_focus()).await;
    let mut reads = Vec::new();
    rewritten
        .apply_with_subqueries(|node| {
            if let LogicalPlan::Filter(filter) = node {
                reads.extend(selection_reads(&filter.predicate, filter.input.schema()).unwrap());
            }
            Ok(TreeNodeRecursion::Continue)
        })
        .unwrap();
    let read = |owners| SelectionRead {
        parameter: "$s".into(),
        owners,
    };
    let delay = std::collections::BTreeSet::from(["delay".to_string()]);
    assert_eq!(
        reads,
        [
            read(Owners::Only(delay.clone())),
            read(Owners::AllBut(delay))
        ]
    );
    let bound = |state: SelectionState| bind(rewritten.clone(), &state);
    let (near, far) = (
        bound(delay_state(10.0, 50.0)),
        bound(delay_state(80.0, 200.0)),
    );
    assert_eq!(cells_subplans(&near), cells_subplans(&far));
    assert_ne!(near, far);
}

/// The reads of `selected` skip the owners of `without` and of an unfaceted `exclude_own`, and a
/// call that selects no row for an empty selection reads every owner.
#[tokio::test]
async fn calls_read_the_clauses_that_they_keep() {
    let ctx = context();
    let reads = |options: &'static str| {
        let ctx = ctx.clone();
        async move {
            let sql = format!("SELECT * FROM flights WHERE selected($s{options})");
            let plan = ctx.sql(&sql).await.unwrap().into_unoptimized_plan();
            let LogicalPlan::Projection(projection) = plan else {
                panic!("{plan}")
            };
            let LogicalPlan::Filter(filter) = projection.input.as_ref() else {
                panic!("{}", projection.input)
            };
            let [read] = selection_reads(&filter.predicate, filter.input.schema())
                .unwrap()
                .try_into()
                .unwrap();
            read.owners
        }
    };
    let owners = |names: &[&str]| names.iter().map(|name| name.to_string()).collect();
    assert_eq!(reads("").await, Owners::AllBut(owners(&[])));
    assert_eq!(
        reads(", exclude_own => true, owner => 'a'").await,
        Owners::AllBut(owners(&["a"]))
    );
    assert_eq!(
        reads(", true, 'a', named_struct('carrier', carrier), NULL, ['b']").await,
        Owners::AllBut(owners(&["b"]))
    );
    assert_eq!(
        reads(", NULL, NULL, NULL, 'none', ['b']").await,
        Owners::AllBut(owners(&[]))
    );
}

/// A query of several concatenated plots that share the selection: the focused plot's own cross
/// filter is unaffected, the other plots are pre-aggregated, a second selection stays a filter
/// in a cells subplan, and the prepared plan returns the direct plan's rows for each move.
#[tokio::test]
async fn concatenated_plots_prepare_each_of_their_aggregates() {
    let ctx = context();
    let sql = "SELECT 'carriers' AS plot, carrier AS key, count(*) AS n FROM flights \
               WHERE selected($s) GROUP BY carrier \
               UNION ALL \
               SELECT 'delay', CAST(floor(delay / 20) AS VARCHAR), count(*) FROM flights \
               WHERE selected($s, exclude_own => true, owner => 'delay') \
               GROUP BY floor(delay / 20) \
               UNION ALL \
               SELECT 'air', CAST(floor(air_time / 50) AS VARCHAR), count(*) FROM flights \
               WHERE selected($s) AND selected($t) GROUP BY floor(air_time / 50)";
    let plan = ctx.sql(sql).await.unwrap().into_unoptimized_plan();
    let prepared = prepare(&plan, &delay_focus()).unwrap();
    let kinds: Vec<&str> = (prepared.strategies.iter())
        .map(|strategy| match strategy {
            Strategy::Preaggregated(_) => "preaggregated",
            Strategy::Unaffected => "unaffected",
            Strategy::Direct(_) => "direct",
        })
        .collect();
    assert_eq!(kinds, ["preaggregated", "unaffected", "preaggregated"]);
    let click = SelectionState::new(
        Resolution::Global,
        vec![Clause::new("bars").condition(carriers(&["AA", "DL"]))],
    );
    let mut cells = Vec::new();
    for state in delay_states() {
        let bind = |plan: &LogicalPlan| {
            bind_selections(plan.clone(), &[("s", &state), ("t", &click)]).unwrap()
        };
        let direct = rows_of(&ctx, bind(&plan)).await;
        assert!(!direct.is_empty());
        assert_eq!(rows_of(&ctx, bind(&prepared.plan)).await, direct);
        cells.push(cells_subplans(&bind(&prepared.plan)));
    }
    assert_eq!(cells[0].len(), 2);
    assert!(cells.windows(2).all(|pair| pair[0] == pair[1]));
}

#[tokio::test]
async fn the_guard_rejects_states_that_the_rollup_cannot_serve() {
    let ctx = context();
    let sql = "SELECT carrier, count(*) AS n FROM flights WHERE selected($s) GROUP BY carrier";
    let (_, _, preaggregation) = prepared(&ctx, sql, &delay_focus()).await;
    let serves = |state: SelectionState| preaggregation.serves(&state);
    assert!(serves(delay_state(10.0, 50.0)));
    let resized =
        PixelGrid::new(LinearScale::configured((-20.0, 100.0), (0.0, 300.0)), 4.0).unwrap();
    let cases = [
        // Another grid, after a resize.
        vec![Clause::new("delay").condition(gridded("delay", resized, 10.0, 50.0))],
        // An exact range, which has no cells.
        vec![Clause::new("delay").condition(exact("delay", 0.0, 10.0))],
        // A column that the focus does not hold.
        vec![Clause::new("delay").condition(exact("distance", 0.0, 10.0))],
    ];
    for clauses in cases {
        assert!(!serves(SelectionState::new(Resolution::Intersect, clauses)));
    }
    let union = SelectionState::new(
        Resolution::Union,
        vec![Clause::new("delay").condition(gridded("delay", delay_grid(), 10.0, 50.0))],
    );
    assert!(!serves(union));
}

#[tokio::test]
async fn ineligible_sites_say_why() {
    let ctx = context();
    let cases = [
        (
            "SELECT * FROM flights WHERE selected($s)",
            Strategy::Direct(DirectReason::NoAggregate),
        ),
        (
            "SELECT carrier, median(distance) FROM flights WHERE selected($s) GROUP BY carrier",
            Strategy::Direct(DirectReason::UnsupportedAggregate),
        ),
        (
            "SELECT carrier, count(DISTINCT distance) FROM flights WHERE selected($s) \
             GROUP BY carrier",
            Strategy::Direct(DirectReason::UnsupportedAggregate),
        ),
        (
            "SELECT carrier, count(*) FROM flights WHERE selected($s, NULL, NULL, NULL, 'none') \
             GROUP BY carrier",
            Strategy::Direct(DirectReason::EmptySelectsNone),
        ),
        (
            "SELECT carrier, count(*) FROM (SELECT carrier FROM flights WHERE selected($s)) \
             GROUP BY carrier",
            Strategy::Direct(DirectReason::MissingKey),
        ),
        (
            "SELECT carrier, count(*) FROM flights WHERE selected($s) AND random() < 2 \
             GROUP BY carrier",
            Strategy::Direct(DirectReason::UnsupportedOperator),
        ),
        (
            "SELECT carrier, count(*) FROM flights \
             WHERE selected($s, exclude_own => true, owner => 'delay') GROUP BY carrier",
            Strategy::Unaffected,
        ),
    ];
    for (sql, expected) in cases {
        let plan = ctx.sql(sql).await.unwrap().into_unoptimized_plan();
        let strategies = prepare(&plan, &delay_focus()).unwrap().strategies;
        assert_eq!(strategies.len(), 1, "{sql}");
        assert_eq!(
            format!("{:?}", strategies[0]),
            format!("{expected:?}"),
            "{sql}"
        );
    }
}

#[tokio::test]
async fn pairs_of_a_grouped_chart_match() {
    // A click selection on the bars of a chart grouped by carrier and distance bin: its keys are
    // both columns, and its tuples select pairs, not their cross combinations.
    let focus = Focus {
        parameter: "$s".into(),
        owner: "grouped".into(),
        dimensions: vec![
            FocusDimension {
                column: "carrier".into(),
                grid: None,
            },
            FocusDimension {
                column: "bin".into(),
                grid: None,
            },
        ],
    };
    let pairs = |pairs: &[(&str, f64)]| {
        let set = Condition::set(
            vec!["carrier".into(), "bin".into()],
            vec![Some("x".into()), Some("color".into())],
            vec![
                Arc::new(StringArray::from_iter_values(pairs.iter().map(|(c, _)| *c))),
                Arc::new(Float64Array::from_iter_values(
                    pairs.iter().map(|(_, b)| *b),
                )),
            ],
        )
        .unwrap();
        SelectionState::new(
            Resolution::Intersect,
            vec![Clause::new("grouped").condition(set)],
        )
    };
    let sql = "SELECT floor(air_time / 50) AS band, count(*) AS n, avg(distance) AS mean \
               FROM (SELECT *, floor(distance / 500) AS bin FROM flights) \
               WHERE selected($s) GROUP BY floor(air_time / 50)";
    contract(
        sql,
        focus,
        vec![
            pairs(&[("AA", 0.0), ("UA", 2.0)]),
            pairs(&[("AA", 0.0), ("UA", 2.0), ("WN", 1.0)]),
            pairs(&[("UA", 2.0), ("WN", 1.0)]),
        ],
    )
    .await;
}
