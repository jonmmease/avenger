//! Tables, sessions, and checks that the tests share.

#![allow(dead_code)]

use std::sync::Arc;

use avenger_query_graph::{Inputs, ResultStore, StoreConfig};
use avenger_selection::SelectionState;
use datafusion::{
    arrow::{
        array::{Array, Float64Array, Int64Array, RecordBatch, StringArray},
        datatypes::{DataType, Field, Schema},
        util::display::array_value_to_string,
    },
    common::ParamValues,
    datasource::MemTable,
    logical_expr::LogicalPlan,
    prelude::SessionContext,
    scalar::ScalarValue,
};

pub const ROWS: usize = 600;

/// Deterministic numbers in `[0, 1)`.
pub struct Numbers(pub u64);

impl Numbers {
    pub fn next(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }

    pub fn between(&mut self, low: f64, high: f64) -> f64 {
        low + (high - low) * self.next()
    }
}

/// Flights over three years: a carrier, a year, a delay from -30 to 110 minutes, a distance, and
/// an air time.
pub fn flights() -> RecordBatch {
    let carriers = ["AA", "DL", "UA", "WN", "B6"];
    let mut numbers = Numbers(7);
    let mut carrier = Vec::new();
    let mut year = Vec::new();
    let mut delay = Vec::new();
    let mut distance = Vec::new();
    let mut air = Vec::new();
    for _ in 0..ROWS {
        carrier.push(carriers[(numbers.next() * carriers.len() as f64) as usize]);
        year.push(2022 + (numbers.next() * 3.0) as i64);
        delay.push(numbers.between(-30.0, 110.0).round());
        distance.push(numbers.between(100.0, 2500.0).round());
        air.push(numbers.between(20.0, 320.0).round());
    }
    let schema = Arc::new(Schema::new(vec![
        Field::new("carrier", DataType::Utf8, false),
        Field::new("year", DataType::Int64, false),
        Field::new("delay", DataType::Float64, false),
        Field::new("distance", DataType::Float64, false),
        Field::new("air_time", DataType::Float64, false),
    ]));
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(carrier)),
            Arc::new(Int64Array::from(year)),
            Arc::new(Float64Array::from(delay)),
            Arc::new(Float64Array::from(distance)),
            Arc::new(Float64Array::from(air)),
        ],
    )
    .unwrap()
}

/// The names of the carriers.
pub fn carriers() -> RecordBatch {
    let schema = Arc::new(Schema::new(vec![
        Field::new("code", DataType::Utf8, false),
        Field::new("name", DataType::Utf8, false),
    ]));
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(vec!["AA", "DL", "UA", "WN", "B6"])),
            Arc::new(StringArray::from(vec![
                "American",
                "Delta",
                "United",
                "Southwest",
                "JetBlue",
            ])),
        ],
    )
    .unwrap()
}

/// A session with `flights` and `carriers` as in-memory tables and the selection functions, with
/// two partitions so tests are fast.
pub fn context() -> SessionContext {
    let config = datafusion::prelude::SessionConfig::new().with_target_partitions(2);
    let mut ctx = SessionContext::new_with_config(config);
    avenger_selection_datafusion::register(&mut ctx).unwrap();
    let flights = flights();
    let table = MemTable::try_new(flights.schema(), vec![vec![flights]]).unwrap();
    ctx.register_table("flights", Arc::new(table)).unwrap();
    let carriers = carriers();
    let table = MemTable::try_new(carriers.schema(), vec![vec![carriers]]).unwrap();
    ctx.register_table("carriers", Arc::new(table)).unwrap();
    ctx
}

pub fn store() -> ResultStore {
    ResultStore::new(StoreConfig {
        budget: 64 << 20,
        max_result: 16 << 20,
    })
}

pub async fn plan(ctx: &SessionContext, sql: &str) -> LogicalPlan {
    ctx.sql(sql).await.unwrap().into_unoptimized_plan()
}

/// Inputs with the parameters `params` and the selections `selections`.
pub fn inputs(params: &[(&str, ScalarValue)], selections: &[(&str, SelectionState)]) -> Inputs {
    Inputs {
        params: ParamValues::from(
            (params.iter())
                .map(|(name, value)| (name.to_string(), value.clone()))
                .collect::<Vec<_>>(),
        ),
        selections: (selections.iter())
            .map(|(name, state)| (name.to_string(), state.clone()))
            .collect(),
        focus: None,
    }
}

/// The rows of `batches` as text, sorted.
pub fn rows(batches: &[RecordBatch]) -> Vec<String> {
    let mut rows = Vec::new();
    for batch in batches {
        for row in 0..batch.num_rows() {
            let cells: Vec<String> = (batch.columns().iter())
                .map(|column| match column.is_null(row) {
                    true => "null".to_string(),
                    false => array_value_to_string(column, row).unwrap(),
                })
                .collect();
            rows.push(cells.join("|"));
        }
    }
    rows.sort();
    rows
}

/// The rows of `plan` run uncut for `inputs`.
pub async fn uncut(ctx: &SessionContext, plan: &LogicalPlan, inputs: &Inputs) -> Vec<String> {
    let states: Vec<(&str, &SelectionState)> = (inputs.selections.iter())
        .map(|(name, state)| (name.as_str(), state))
        .collect();
    let bound = avenger_selection_datafusion::bind_selections(plan.clone(), &states)
        .unwrap()
        .with_param_values(inputs.params.clone())
        .unwrap();
    let batches = ctx
        .execute_logical_plan(bound)
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    rows(&batches)
}

/// A range of `column` from `low` to `high`, without cells.
pub fn exact(column: &str, low: f64, high: f64) -> avenger_selection::Condition {
    avenger_selection::Condition::Range {
        column: column.into(),
        channel: None,
        ends: Arc::new(Float64Array::from(vec![low, high])),
        exclude_end: false,
        cells: None,
    }
}

/// A range of `column` over the pixels `low` to `high` of `grid`, with the cells that it covers.
pub fn gridded(
    column: &str,
    grid: &avenger_selection::PixelGrid,
    low: f64,
    high: f64,
) -> avenger_selection::Condition {
    let cells = grid.clone().span(low, high).unwrap();
    avenger_selection::Condition::Range {
        column: column.into(),
        channel: None,
        ends: Arc::new(Float64Array::from(vec![low, high])),
        exclude_end: false,
        cells: Some(cells),
    }
}

/// A state of intersected clauses, each of an owner and its conditions.
pub fn state(clauses: Vec<(&str, avenger_selection::Condition)>) -> SelectionState {
    SelectionState::new(
        avenger_selection::Resolution::Intersect,
        (clauses.into_iter())
            .map(|(owner, condition)| avenger_selection::Clause::new(owner).condition(condition))
            .collect(),
    )
}

/// The name of each outcome.
pub fn outcomes(run: &avenger_query_graph::Run) -> Vec<&'static str> {
    use avenger_query_graph::Outcome;
    (run.outcomes.iter())
        .map(|outcome| match outcome {
            Outcome::Hit(_) => "hit",
            Outcome::Computed { widened, .. } if !widened.is_empty() => "widened",
            Outcome::Computed { .. } => "computed",
            Outcome::Uncached { .. } => "uncached",
            Outcome::NotRead => "not read",
        })
        .collect()
}
