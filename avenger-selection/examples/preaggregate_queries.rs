//! Selection and preaggregation with native DataFusion queries and explicit MemTables.
use std::sync::Arc;

use avenger_datafusion_preaggregate::{
    BoundQuery, FilterQuery, PreaggregatePlanner, PreparedQuery,
};
use avenger_selection::*;
use datafusion::{
    arrow::{
        array::{Int64Array, StringArray},
        record_batch::RecordBatch,
        util::pretty::pretty_format_batches,
    },
    datasource::MemTable,
    functions_aggregate::expr_fn::count,
    logical_expr::{col, lit, LogicalPlan, LogicalPlanBuilder},
    prelude::SessionContext,
    sql::unparser::Unparser,
};

type ExampleResult<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[tokio::main]
async fn main() -> ExampleResult<()> {
    let show_sql = std::env::args().any(|arg| arg == "--sql");
    let sql = Unparser::default().with_pretty(true);

    // 1. Register the input rows and define the shared selection.
    let batch = RecordBatch::try_from_iter(vec![
        (
            "delay",
            Arc::new(Int64Array::from(vec![0, 10, 20, 30, 40, 50])) as _,
        ),
        (
            "distance",
            Arc::new(Int64Array::from(vec![500, 500, 1000, 1000, 1500, 1500])) as _,
        ),
        (
            "carrier",
            Arc::new(StringArray::from(vec!["AA", "DL", "AA", "UA", "DL", "UA"])) as _,
        ),
    ])?;
    let ctx = SessionContext::new();
    ctx.register_table(
        "flights",
        Arc::new(MemTable::try_new(batch.schema(), vec![vec![batch]])?),
    )?;
    let source = ctx.table("flights").await?.into_unoptimized_plan();
    let selection = SelectionId::new("filters")?;
    let delay = ProducerDefinition::new(
        ProducerAddress {
            selection: selection.clone(),
            producer: ProducerId::new("delay_brush")?,
            origin: ViewAddress::root(ViewId::new("delay")?),
        },
        SelectionKind::Interval,
        vec![Projection::new(ProjectionId::new("delay")?, col("delay"))?],
    )?;
    let inactive = SelectionSet::new([(selection.clone(), Resolution::Intersect)])?;

    // 2. Prepare chart queries and materialize receiver states before any brush.
    let planner = PreaggregatePlanner::default();
    let mut charts = Vec::new();

    println!("Three charts | native DataFusion queries | exact delay selection");
    println!("Warm-up executes once per receiving chart and stores its batches in a MemTable.");
    for (name, group, order) in [
        (
            "delay",
            (col("delay") / lit(20_i64)).alias("delay_bin"),
            "delay_bin",
        ),
        (
            "distance",
            (col("distance") / lit(500_i64)).alias("distance_bin"),
            "distance_bin",
        ),
        ("airlines", col("carrier"), "carrier"),
    ] {
        let filter = ConsumerFilter::new(
            ViewAddress::root(ViewId::new(name)?),
            SelectionFilter::cross_filter([&selection]),
        );
        let target = |rows| {
            LogicalPlanBuilder::from(rows)
                .aggregate(
                    vec![group.clone()],
                    vec![count(lit(1_i64)).alias("flights")],
                )?
                .sort(vec![col(order).sort(true, true)])?
                .build()
        };
        let direct = FilterQuery::new(source.clone(), target)?;
        let predicates = filter.predicates(&inactive, &delay)?;

        let stored = if let Ok(split) = predicates.split() {
            let query = FilterQuery::new(source.clone(), |rows| {
                target(
                    LogicalPlanBuilder::from(rows)
                        .filter(split.fixed().clone())?
                        .build()?,
                )
            })?;
            let prepared = planner.prepare(query, split.dimensions().to_vec())?;
            if let Some(materialization) = prepared.materialization_plan() {
                if show_sql {
                    println!("\nWarm-up {name}:\n{:#}", sql.plan_to_sql(materialization)?);
                }
                let schema = Arc::new(materialization.schema().as_arrow().clone());
                let batches = ctx
                    .execute_logical_plan(materialization.clone())
                    .await?
                    .collect()
                    .await?;
                let row_count: usize = batches.iter().map(RecordBatch::num_rows).sum();
                let table_name = format!("{name}_states");
                // Explicit ownership preserves the declared schema even with no batches.
                ctx.register_table(
                    &table_name,
                    Arc::new(MemTable::try_new(schema, vec![batches])?),
                )?;
                println!("  {name}: stored {row_count} rows in {table_name}");
                let stored = ctx.table(&table_name).await?.into_unoptimized_plan();
                Some((prepared, stored))
            } else {
                println!("  {name}: direct ({:?})", prepared.explain().direct_reason);
                None
            }
        } else {
            println!("  {name}: direct ({})", predicates.split().unwrap_err());
            None
        };
        charts.push(ChartQuery {
            name,
            filter,
            direct,
            stored,
        });
    }

    // 3. Apply each brush update and query the same prepared charts.
    // Request     Delay producer    Distance producer    Airline producer
    // inactive    inactive          inactive             inactive
    // brushed     [10, 40)          inactive             inactive
    // dragged     [11, 40)          inactive             inactive
    // cleared     inactive          inactive             inactive
    //
    // Bounds include lower and exclude upper. Only delay changes, so the fixed
    // predicates and interaction dimensions remain compatible with warm-up.
    let mut selections = inactive;
    let mut comparisons = Vec::new();
    for (label, lower) in [
        ("No selection", None),
        ("Brush [10, 40)", Some(10_i64)),
        ("Drag [11, 40)", Some(11_i64)),
        ("Clear", None),
    ] {
        selections = match lower {
            Some(lower) => selections.set(
                &delay,
                SelectionValue::tuple([(
                    ProjectionId::new("delay")?,
                    ValueTest::range(lower..40_i64),
                )]),
            )?,
            None => selections.clear(&delay)?,
        };
        println!("\n{label}");
        for chart in &charts {
            let predicates = chart.filter.predicates(&selections, &delay)?;
            let direct = chart.direct.direct(predicates.full().clone())?;
            // These states change only focus bounds. Changed fixed predicates or
            // dimensions require another preparation and materialization.
            let (plan, strategy) = match (&chart.stored, predicates.split()) {
                (Some((prepared, stored)), Ok(split)) => {
                    match prepared.bind(split.changing().clone())? {
                        BoundQuery::Preaggregated { rollup, .. } => (
                            rollup.with_materialization(stored.clone())?,
                            "rollup over stored states",
                        ),
                        BoundQuery::Direct { .. } => (direct.clone(), "direct source query"),
                    }
                }
                _ => (direct.clone(), "direct source query"),
            };
            assert_eq!(plan.schema(), direct.schema());
            if show_sql {
                println!("\n{}:\n{:#}", chart.name, sql.plan_to_sql(&plan)?);
            }
            let batches = ctx.execute_logical_plan(plan).await?.collect().await?;
            let table = pretty_format_batches(&batches)?.to_string();
            println!("  {}: {strategy}\n{table}", chart.name);
            comparisons.push((direct, table));
        }
    }

    // 4. Verify results with extra direct queries after the demonstration.
    for (direct, actual) in comparisons {
        let expected = ctx.execute_logical_plan(direct).await?.collect().await?;
        assert_eq!(actual, pretty_format_batches(&expected)?.to_string());
    }
    println!("\nAll tables match complete direct queries (checked after the demonstration).");
    println!(
        "The caller owns these MemTables. Each rollup runs again, with no automatic result cache."
    );
    Ok(())
}

// This example-owned record holds native plans, not a graph or query runtime.
struct ChartQuery {
    name: &'static str,
    filter: ConsumerFilter,
    direct: FilterQuery,
    stored: Option<(PreparedQuery, LogicalPlan)>,
}
