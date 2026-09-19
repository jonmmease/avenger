//! Three cross-filtered charts with optional aggregate-state measures.
use std::{collections::HashMap, ops::Bound, sync::Arc};

use avenger_datafusion_dataflow::{
    Dataflow, DataflowBuilder, EvaluationReport, Runtime, TableSnapshot,
};
use avenger_selection::*;
use datafusion::{
    arrow::{
        array::{Float64Array, Int64Array, StringArray},
        compute::{concat_batches, sort_to_indices, take_record_batch},
        record_batch::RecordBatch,
        util::pretty::print_batches,
    },
    common::ScalarValue,
    functions_aggregate::expr_fn::{
        avg, count, max, min, stddev, stddev_pop, sum, var_pop, var_sample,
    },
    logical_expr::{col, lit, LogicalPlanBuilder, LogicalTableSource},
};

// Selection state at each request, all under the shared "filters" name:
//
// Request     Delay producer    Distance producer    Airline producer
// inactive    inactive          inactive             inactive
// brushed     [10, 40)          inactive             inactive
// dragged     [11, 40)          inactive             inactive
// unchanged   [11, 40)          inactive             inactive
//
// Inactive means no contribution, so it imposes no filter. Bounds include
// the lower endpoint and exclude the upper endpoint. The drag moves only
// the delay brush's lower edge. "Unchanged" reuses the dragged snapshot.
// Distance and airline charts apply the delay predicate. The delay chart
// excludes its own producer and continues to show all rows in every state.
// All requests share one focus and prepared extension.
fn brush_states(delay: &ProducerDefinition) -> Result<[(&'static str, SelectionSet); 4]> {
    let name = &delay.address().selection;
    let inactive = SelectionSet::new([SelectionSnapshot::new(SelectionDefinition::new(
        name.clone(),
        Resolution::Intersect,
    ))?])?;
    let projection = ProjectionId::new("delay")?;
    let brush = |lower: i64, upper: i64| {
        SelectionUpdate::set(
            delay,
            SelectionValue::Tuples(vec![SelectionTuple {
                terms: vec![SelectionTerm {
                    projection: projection.clone(),
                    test: ValueTest::Range {
                        lower: Bound::Included(lower.into()),
                        upper: Bound::Excluded(upper.into()),
                    },
                }],
            }]),
        )
    };
    let brushed = inactive.apply(name, brush(10, 40))?;
    let dragged = brushed.apply(name, brush(11, 40))?;
    Ok([
        ("Render with no selection", inactive),
        ("Brush delay: [10, 40)", brushed),
        ("Drag delay: [11, 40)", dragged.clone()),
        ("Repeat the same brush: [11, 40)", dragged),
    ])
}

fn print_report(report: &EvaluationReport) {
    println!("  Computed this request (runtime report):");
    if report.executed_nodes.is_empty() {
        println!("    (none)");
    }
    for node in &report.executed_nodes {
        println!("    {node}");
    }
    println!(
        "  Cache hits: {} | Shared in-progress computations: {}",
        report.cache_hits, report.in_flight_hits
    );
}

fn print_table(label: &str, table: &TableSnapshot) -> datafusion::arrow::error::Result<()> {
    // Aggregate row order is unspecified. Sort only the display for comparison.
    let batch = concat_batches(table.schema(), table.batches())?;
    let indices = sort_to_indices(batch.column(0), None, None)?;
    let sorted = take_record_batch(&batch, &indices)?;
    println!("\n  {label}");
    print_batches(&[sorted])
}

fn assert_same_table(
    actual: &TableSnapshot,
    expected: &TableSnapshot,
) -> datafusion::common::Result<()> {
    assert_eq!(actual.schema(), expected.schema());
    let sorted = |table: &TableSnapshot| -> datafusion::arrow::error::Result<RecordBatch> {
        let batch = concat_batches(table.schema(), table.batches())?;
        let indices = sort_to_indices(batch.column(0), None, None)?;
        take_record_batch(&batch, &indices)
    };
    let actual = sorted(actual)?;
    let expected = sorted(expected)?;
    assert_eq!(actual.num_rows(), expected.num_rows());
    for row in 0..actual.num_rows() {
        for column in 0..actual.num_columns() {
            let a = ScalarValue::try_from_array(actual.column(column), row)?;
            let e = ScalarValue::try_from_array(expected.column(column), row)?;
            if column > 0 {
                if let (ScalarValue::Float64(Some(a)), ScalarValue::Float64(Some(e))) = (&a, &e) {
                    assert!(
                        a.is_finite() && e.is_finite() && (a - e).abs() <= 1e-10 * (1.0 + e.abs()),
                        "{a} != {e}"
                    );
                    continue;
                }
            }
            assert_eq!(a, e);
        }
    }
    Ok(())
}

fn print_sql_appendix(
    dataflow: &Dataflow,
    installed: &[(&str, QueryFamily, InstalledSelectionQuery)],
    states: &[(&str, SelectionSet)],
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    println!("\nSQL inspection appendix");
    println!("These are bound logical plans. The execution reports above show what ran.");
    println!("Final aggregates read named materializations. Only changed SQL text is printed.");
    let sql = dataflow.sql();
    let mut previous = HashMap::new();
    for (step, state) in states {
        println!("\n--- {step} ---");
        let mut changed = false;
        for (label, family, _) in installed {
            let plans = match family.bind(state)? {
                BoundQuery::Direct { plan, .. } => vec![(format!("{label}__direct"), plan)],
                BoundQuery::Preaggregated {
                    materialization,
                    aggregate,
                } => {
                    let name = format!("{label}__materialization");
                    // This schema-only relation lets SQL inspection show the
                    // same boundary that the installed dataflow materializes.
                    let relation = LogicalPlanBuilder::scan(
                        name.clone(),
                        Arc::new(LogicalTableSource::new(Arc::new(
                            materialization.schema().as_arrow().clone(),
                        ))),
                        None,
                    )?
                    .build()?;
                    vec![
                        (name, materialization),
                        (format!("{label}__aggregate"), aggregate.over(relation)?),
                    ]
                }
                other => return Err(format!("SQL display does not support {other:?}").into()),
            };
            for (name, plan) in plans {
                let text = sql.plan(&plan)?;
                if previous.get(&name) != Some(&text) {
                    println!("\n{name}:\n{text}");
                    previous.insert(name, text);
                    changed = true;
                }
            }
        }
        if !changed {
            println!("SQL text is unchanged from the previous step.");
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let force_direct = std::env::args().any(|a| a == "--force-direct");
    let show_aggregates = std::env::args().any(|a| a == "--aggregates");
    let show_sql = std::env::args().any(|a| a == "--sql");
    let policy = if force_direct {
        QueryPolicy::ForceDirect
    } else {
        QueryPolicy::Auto
    };
    println!("Cross-filtered charts | policy: {policy:?}");
    println!("Focus: delay. Only the delay brush changes during this run.");
    println!("The delay chart excludes its own brush; distance and airlines use it.");
    println!("Bounds [lower, upper) include lower and exclude upper.");
    println!("Display groups: delay / 20, distance / 500, and airline carrier.");
    println!("Requests run sequentially. Cache statistics below come from the runtime.");
    if show_aggregates {
        println!("Receiver measures: count, sum/min/max/average fare, and variance/stddev.");
        println!("Fare includes nulls, and delay cells have unequal row counts.");
    }
    println!("\nNode names:");
    println!("  __materialization = reusable aggregate states by display group and delay value");
    println!("  __aggregate       = filter materialized states by brush bounds, then merge");
    println!("  __direct          = apply the full predicate to the original query");
    println!("\nPrepared chart strategies:");
    let name = SelectionId::new("filters")?;
    let mut panels = Vec::new();
    for (view, field, kind) in [
        ("delay", "delay", SelectionKind::Interval),
        ("distance", "distance", SelectionKind::Interval),
        ("airlines", "carrier", SelectionKind::Point),
    ] {
        let origin = ViewAddress::root(ViewId::new(view)?);
        let producer = ProducerDefinition::new(
            ProducerAddress {
                selection: name.clone(),
                producer: ProducerId::new(view)?,
                origin: origin.clone(),
            },
            kind,
            vec![Projection::new(ProjectionId::new(field)?, col(field))?],
        )?;
        let filter = SelectionCompiler::new().filter(
            &SelectionConsumer::new(origin),
            SelectionFilter::cross_filter([&name]),
        )?;
        panels.push((producer, filter));
    }
    let focus = &panels[0].0;
    let states = brush_states(focus)?;
    let inactive = &states[0].1;

    let batch = if show_aggregates {
        RecordBatch::try_from_iter(vec![
            (
                "delay",
                Arc::new(Int64Array::from(vec![0, 10, 10, 10, 20, 30, 40, 50])) as _,
            ),
            (
                "distance",
                Arc::new(Int64Array::from(vec![
                    500, 500, 500, 500, 1000, 1000, 1500, 1500,
                ])) as _,
            ),
            (
                "carrier",
                Arc::new(StringArray::from(vec![
                    "AA", "AA", "AA", "DL", "AA", "UA", "DL", "UA",
                ])) as _,
            ),
            (
                "fare",
                Arc::new(Float64Array::from(vec![
                    Some(5.),
                    Some(2.),
                    Some(4.),
                    None,
                    Some(12.),
                    Some(18.),
                    None,
                    Some(32.),
                ])) as _,
            ),
        ])?
    } else {
        RecordBatch::try_from_iter(vec![
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
        ])?
    };
    let mut graph = DataflowBuilder::new();
    let source = graph.table_snapshot(
        "flights",
        TableSnapshot::from_batches(batch.schema(), vec![batch])?,
    )?;
    let source_output = graph.table_output("flights", &source)?;
    let base = Runtime::new(Default::default())?
        .prepare(&graph.finish()?)
        .await?;
    let base_inputs = base.inputs().finish()?;
    let mut additional = DataflowBuilder::with_base(&base.interface());
    let imported = additional.import_table("flights", &source_output)?;
    let mut installed = Vec::new();
    for ((producer, filter), group) in panels.iter().zip([
        (col("delay") / lit(20_i64)).alias("delay_bin"),
        (col("distance") / lit(500_i64)).alias("distance_bin"),
        col("carrier"),
    ]) {
        let mut aggregates = vec![count(lit(1_i64)).alias("count")];
        if show_aggregates && producer.address().origin.view.as_str() != "delay" {
            aggregates.extend([
                sum(col("fare")).alias("sum"),
                min(col("fare")).alias("min"),
                max(col("fare")).alias("max"),
                avg(col("fare")).alias("mean"),
                var_sample(col("fare")).alias("var_samp"),
                var_pop(col("fare")).alias("var_pop"),
                stddev(col("fare")).alias("stddev_samp"),
                stddev_pop(col("fare")).alias("stddev_pop"),
            ]);
        }
        let query = filter.query(imported.plan_ref(), |rows| {
            LogicalPlanBuilder::from(rows)
                .aggregate(vec![group], aggregates)?
                .build()
        })?;
        let family = query.plan(inactive).focus(focus).policy(policy).build()?;
        let label = producer.address().origin.view.as_str();
        println!("  {label:8} {}", family.explain());
        let panel = family.install(&mut additional, label)?;
        installed.push((label, family, panel));
    }
    let additional = additional.finish()?;
    let extension = base.prepare_extension(&additional).await?;

    // Plot entry can warm every compatible receiver before the first brush.
    // The runtime retains these ordinary outputs under its configured cache policy.
    let mut warm_inputs = extension.inputs();
    let mut warm_outputs = Vec::new();
    for (_, _, panel) in &installed {
        let binding = panel.bind(inactive)?;
        warm_outputs.extend(binding.preaggregate_output());
        warm_inputs = binding.apply(warm_inputs)?;
    }
    let mut step_number = 0;
    if !warm_outputs.is_empty() {
        step_number += 1;
        println!("\n{step_number}. Hover over delay: warm-up before any selection");
        println!("  Request: reusable states for the other charts. No chart results requested.");
        let warm = extension
            .query(&warm_outputs, &[], &base_inputs, &warm_inputs.finish()?)
            .await?;
        print_report(warm.report());
    } else {
        println!("\nWarm-up skipped: these bindings use direct execution.");
    }
    let mut observed = Vec::new();
    for (step, state) in &states {
        step_number += 1;
        println!("\n{step_number}. {step}");
        println!("  Request: final results for all three charts.");
        let mut inputs = extension.inputs();
        let mut outputs = Vec::new();
        for (_, _, panel) in &installed {
            let binding = panel.bind(state)?;
            outputs.push(binding.output());
            inputs = binding.apply(inputs)?;
        }
        let result = extension
            .query(&outputs, &[], &base_inputs, &inputs.finish()?)
            .await?;
        print_report(result.report());
        println!("  Chart results:");
        let mut tables = Vec::new();
        for ((label, _, _), output) in installed.iter().zip(outputs) {
            let table = result.table(&output)?;
            print_table(label, table)?;
            tables.push(table.clone());
        }
        observed.push(tables);
    }

    // Run comparisons after the displayed sequence so verification cannot warm its cache.
    let reference_policy = if force_direct {
        QueryPolicy::Auto
    } else {
        QueryPolicy::ForceDirect
    };
    for ((_, state), tables) in states.iter().zip(observed) {
        let mut inputs = extension.inputs();
        let mut outputs = Vec::new();
        for (_, _, panel) in &installed {
            let binding = panel.bind_with_policy(state, reference_policy)?;
            outputs.push(binding.output());
            inputs = binding.apply(inputs)?;
        }
        let reference = extension
            .query(&outputs, &[], &base_inputs, &inputs.finish()?)
            .await?;
        for (table, output) in tables.iter().zip(outputs) {
            assert_same_table(table, reference.table(&output)?)?;
        }
    }
    println!("\nVerified all displayed results against {reference_policy:?} (after the reported sequence).");

    if show_sql {
        print_sql_appendix(&additional, &installed, &states)?;
    } else {
        println!("\nAdd --sql to inspect the bound SQL for these steps.");
    }
    Ok(())
}
