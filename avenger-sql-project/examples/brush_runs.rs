//! The time of the runs of a chart that a brush filters, as a drag reruns it: bars of the sums of
//! a host table's rows that `WHERE brush` keeps, run with a moving interval of the brush. It prints
//! the median time of a run through the project's store, and of the two parts of one run of the
//! bound query without the store: planning it (optimizing and physical planning) and executing it.
//!
//! ```text
//! cargo run --release -p avenger-sql-project --example brush_runs
//! ```

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use avenger_selection::{Clause, Condition};
use avenger_sql_project::{FileSource, LoadedProject, ProjectFiles};
use datafusion::{
    arrow::{
        array::{ArrayRef, Float64Array, RecordBatch, StringArray},
        datatypes::{DataType, Field, Schema},
    },
    datasource::MemTable,
    prelude::SessionContext,
};

const ROWS: usize = 5_000;
const RUNS: usize = 200;
const CHART: &str = "charts/bars.av.sql";
const TEXT: &str = "DECLARE brush SELECTION;\n\
                    FROM host |> WHERE brush |> AGGREGATE mark_bar(x => k, y => SUM(v)) AS chart";

#[tokio::main]
async fn main() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("avenger.toml"),
        "[project]\nname = \"brush\"\nformat = 1\n",
    )
    .unwrap();
    let ctx = SessionContext::new();
    ctx.register_table("host", Arc::new(host())).unwrap();
    let files = ProjectFiles::open(FileSource::directory(root.path()).unwrap())
        .await
        .unwrap();
    let project = LoadedProject::load(files, ctx.clone()).await.unwrap();
    let query = project.run_chart(CHART, TEXT).await.unwrap();

    let (mut runs, mut planning, mut executing) = (Vec::new(), Vec::new(), Vec::new());
    for run in 0..RUNS {
        let clauses = [("brush".to_string(), vec![brush(run)])];
        let bound = query
            .clone()
            .with_clauses(clauses.iter().map(|(name, clauses)| (name, clauses)))
            .await
            .unwrap();
        let started = Instant::now();
        bound.collect().await.unwrap();
        runs.push(started.elapsed());

        let started = Instant::now();
        let plan = bound.frame().clone().create_physical_plan().await.unwrap();
        planning.push(started.elapsed());
        let started = Instant::now();
        datafusion::physical_plan::collect(plan, ctx.task_ctx())
            .await
            .unwrap();
        executing.push(started.elapsed());
    }
    println!("{ROWS} rows, {RUNS} runs of a moving brush, medians:");
    println!("  run through the store: {:?}", median(runs));
    println!("  planning the bound query: {:?}", median(planning));
    println!("  executing its plan: {:?}", median(executing));
}

/// The host table: a key of ten categories and a value, the same on every run.
fn host() -> MemTable {
    let schema = Arc::new(Schema::new(vec![
        Field::new("k", DataType::Utf8, false),
        Field::new("v", DataType::Float64, false),
    ]));
    let keys: Vec<String> = (0..ROWS).map(|row| format!("k{}", row % 10)).collect();
    let values: Vec<f64> = (0..ROWS).map(|row| ((row * 7919) % 1000) as f64).collect();
    let columns: Vec<ArrayRef> = vec![
        Arc::new(StringArray::from(keys)),
        Arc::new(Float64Array::from(values)),
    ];
    let batch = RecordBatch::try_new(schema.clone(), columns).unwrap();
    MemTable::try_new(schema, vec![vec![batch]]).unwrap()
}

/// The clause of the brush at the `run`th move: an interval of `v` 200 wide that slides across
/// the values.
fn brush(run: usize) -> Clause {
    let low = (run * 4 % 800) as f64;
    Clause::new("scatter").condition(Condition::Range {
        column: "v".into(),
        channel: None,
        ends: Arc::new(Float64Array::from(vec![low, low + 200.0])),
        exclude_end: false,
        cells: None,
    })
}

fn median(mut times: Vec<Duration>) -> Duration {
    times.sort();
    times[times.len() / 2]
}
