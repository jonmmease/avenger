//! Three charts compose selection predicates, generic preaggregation, and dataflow.
#[path = "support/composition.rs"]
mod composition;
use avenger_datafusion_dataflow::{
    Dataflow, DataflowBuilder, EvaluationReport, ExprInput, Inputs, PreparedDataflow,
    PreparedExtension, Runtime, TableOutput, TableSnapshot,
};
use avenger_scales_datafusion::BuiltinScale;
use avenger_selection::*;
use composition::{ExampleResult, OptimizedQuery};
use datafusion::{
    arrow::{
        array::{Float32Array, Float64Array, Int64Array, StringArray},
        compute::{concat_batches, sort_to_indices, take_record_batch},
        datatypes::DataType,
        record_batch::RecordBatch,
        util::pretty::print_batches,
    },
    common::ScalarValue,
    functions_aggregate::expr_fn::{
        avg, count, max, min, stddev, stddev_pop, sum, var_pop, var_sample,
    },
    logical_expr::{col, lit, Expr, LogicalPlan, LogicalPlanBuilder},
};
use std::{collections::HashMap, sync::Arc};

// Selection states share the "filters" name and intersection resolution:
// Request        Delay producer    Distance producer    Airline producer
// inactive       inactive          inactive             inactive
// brushed        [10, 40)          inactive             inactive
// dragged        [11, 40)          inactive             inactive
// unchanged      [11, 40)          inactive             inactive
// fixed changed  [11, 40)          inactive             AA
// regridded      [11, 40)          inactive             AA
//
// Bounds include lower and exclude upper. The first four requests share one
// preparation. The last two replace it. Inactive means no contribution.
// Each chart excludes its own producers. Pixel mode uses cell membership in
// both direct and optimized queries. Regridding occurs only in pixel mode.
fn brush_states(delay: &ProducerDefinition) -> Result<[(&'static str, SelectionSet); 4]> {
    let name = delay.selection();
    let inactive = SelectionSet::new([(name.clone(), Resolution::Intersect)])?;
    let projection = ProjectionId::new("delay")?;
    let brush = |lower: i64, upper: i64| {
        SelectionUpdate::set(
            delay,
            SelectionValue::tuple([(projection.clone(), ValueTest::range(lower..upper))]),
        )
    };
    let brushed = inactive.apply(brush(10, 40))?;
    let dragged = brushed.apply(brush(11, 40))?;
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
    println!("  Source executions: {}", report.source_executions);
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

fn flights(show_aggregates: bool) -> ExampleResult<TableSnapshot> {
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
    Ok(TableSnapshot::from_batches(batch.schema(), vec![batch])?)
}

// These are private chart records, not selection-library types.
struct Plot {
    name: &'static str,
    producer: ProducerDefinition,
    filter: ConsumerFilter,
    group: Expr,
    measures: Vec<Expr>,
}
impl Plot {
    fn query(&self, rows: LogicalPlan) -> datafusion::common::Result<LogicalPlan> {
        LogicalPlanBuilder::from(rows)
            .aggregate(vec![self.group.clone()], self.measures.clone())?
            .build()
    }
}
fn delay_grid(size: f64) -> Result<PixelGrid> {
    PixelGrid::new(
        BuiltinScale::Linear,
        Arc::new(Float32Array::from(vec![0., 60.])),
        Arc::new(Float32Array::from(vec![0., 600.])),
        Default::default(),
        0.,
        size,
    )
}
struct ActiveFocus {
    definition: Dataflow,
    flow: PreparedExtension,
    targets: Vec<Option<OptimizedQuery>>,
    fallbacks: Vec<TableOutput>,
    producer: ProducerDefinition,
}
async fn prepare_focus(
    base: &PreparedDataflow,
    source: &TableOutput,
    direct: &[TableOutput],
    plots: &[Plot],
    focus: &ProducerDefinition,
    state: &SelectionSet,
) -> ExampleResult<ActiveFocus> {
    let mut additional = DataflowBuilder::with_base(&base.interface());
    let source = additional.import_table("flights", source)?;
    let mut targets = Vec::new();
    let mut fallbacks = Vec::new();
    for (plot, direct) in plots.iter().zip(direct) {
        let fallback = additional.import_table(format!("{}_direct", plot.name), direct)?;
        fallbacks.push(additional.table_output(format!("{}_direct", plot.name), &fallback)?);
        let predicates = plot.filter.predicates(state, focus)?;
        let target = match predicates.split() {
            Ok(split) => {
                let prepared =
                    composition::prepare(source.plan_ref(), split, |rows| plot.query(rows))?;
                println!(
                    "  {} candidate: {:?}",
                    plot.name,
                    prepared.bind(split.changing().clone())?.diagnostics()
                );
                OptimizedQuery::install(&mut additional, plot.name, prepared, split)?
            }
            Err(reason) => {
                println!("  {}: direct ({reason})", plot.name);
                None
            }
        };
        targets.push(target);
    }
    let definition = additional.finish()?;
    let flow = base.prepare_extension(&definition).await?;
    Ok(ActiveFocus {
        definition,
        flow,
        targets,
        fallbacks,
        producer: focus.clone(),
    })
}
fn bindings(
    base: &PreparedDataflow,
    full_inputs: &[ExprInput],
    active: &ActiveFocus,
    plots: &[Plot],
    state: &SelectionSet,
    force_direct: bool,
) -> ExampleResult<(Inputs, Inputs, Vec<TableOutput>, Vec<TableOutput>)> {
    let mut base_inputs = base.inputs();
    let mut inputs = active.flow.inputs();
    let mut outputs = Vec::new();
    let mut warm = Vec::new();
    for (index, (plot, full)) in plots.iter().zip(full_inputs).enumerate() {
        let p = plot.filter.predicates(state, &active.producer)?;
        base_inputs = base_inputs.expr(full, p.full().clone())?;
        let mut output = active.fallbacks[index];
        if let Some(target) = &active.targets[index] {
            let retained = if force_direct {
                None
            } else {
                match p.split() {
                    Ok(split) => target.bind(split)?,
                    Err(_) => None,
                }
            };
            if retained.is_some() {
                output = target.output;
                warm.push(target.materialization);
            }
            inputs = inputs.expr(&target.predicate, retained.unwrap_or_else(|| lit(true)))?;
        }
        outputs.push(output);
    }
    Ok((base_inputs.finish()?, inputs.finish()?, outputs, warm))
}

#[tokio::main]
async fn main() -> ExampleResult<()> {
    let force_direct = std::env::args().any(|a| a == "--force-direct");
    let show_aggregates = std::env::args().any(|a| a == "--aggregates");
    let show_sql = std::env::args().any(|a| a == "--sql");
    let pixels = std::env::args().any(|a| a == "--pixels");
    println!("Cross-filtered charts | force direct: {force_direct} | pixel membership: {pixels}");
    println!("Focus: delay. Distance and airlines use its brush. Delay excludes its own brush.");
    println!("Display groups: delay / 20, distance / 500, and carrier.");
    println!("_states nodes materialize display groups plus delay interaction dimensions.");
    println!("_rollup nodes filter those states and merge aggregates into display groups.");
    let name = SelectionId::new("filters")?;
    let mut plots = Vec::new();
    for (view, field, group) in [
        (
            "delay",
            "delay",
            (col("delay") / lit(20_i64)).alias("delay_bin"),
        ),
        (
            "distance",
            "distance",
            (col("distance") / lit(500_i64)).alias("distance_bin"),
        ),
        ("airlines", "carrier", col("carrier")),
    ] {
        let origin = ViewId::new(view)?;
        let mut producer = ProducerDefinition::new(
            name.clone(),
            ProducerId::new(view)?,
            origin.clone(),
            vec![Projection::new(ProjectionId::new(field)?, col(field))?],
        )?;
        if pixels && view == "delay" {
            producer = producer.with_pixel_grids([(ProjectionId::new(field)?, delay_grid(2.)?)])?;
        }
        let filter = ConsumerFilter::new(origin, SelectionFilter::cross_filter([&name]));
        let mut measures = vec![count(lit(1_i64)).alias("count")];
        if show_aggregates && view != "delay" {
            measures.extend([
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
        plots.push(Plot {
            name: view,
            producer,
            filter,
            group,
            measures,
        });
    }
    let states = brush_states(&plots[0].producer)?;
    let mut graph = DataflowBuilder::new();
    let source = graph.table_snapshot("flights", flights(show_aggregates)?)?;
    let source_output = graph.table_output("flights", &source)?;
    let mut full_inputs = Vec::new();
    let mut direct = Vec::new();
    for plot in &plots {
        let predicate = graph.expr_input(format!("{}_full", plot.name), DataType::Boolean)?;
        let rows = LogicalPlanBuilder::from(source.plan_ref())
            .filter(predicate.expr_ref())?
            .build()?;
        let plan = graph.add_plan(format!("{}_direct", plot.name), plot.query(rows)?)?;
        direct.push(graph.table_output(plot.name, &plan)?);
        full_inputs.push(predicate);
    }
    let definition = graph.finish()?;
    let base = Runtime::new(Default::default())?
        .prepare(&definition)
        .await?;
    println!("\nPrepare from inactive delay definition:");
    let mut active = prepare_focus(
        &base,
        &source_output,
        &direct,
        &plots,
        &plots[0].producer,
        &states[0].1,
    )
    .await?;
    let (base_inputs, inputs, _, warm) = bindings(
        &base,
        &full_inputs,
        &active,
        &plots,
        &states[0].1,
        force_direct,
    )?;
    println!("\nHover: warm receiver states before any brush (no final chart requested)");
    if !warm.is_empty() {
        let result = active.flow.query(&warm, &[], &base_inputs, &inputs).await?;
        print_report(result.report());
    } else if force_direct {
        println!("  Warm-up skipped: force-direct is enabled.");
    } else {
        println!("  No eligible materialization requested.");
    }

    let airline_state = states[3].1.set(
        &plots[2].producer,
        SelectionValue::tuple([(ProjectionId::new("carrier")?, ValueTest::equal("AA"))]),
    )?;
    let mut requests = states
        .into_iter()
        .map(|(label, state)| (label, state, None))
        .collect::<Vec<_>>();
    requests.push((
        "Other selection changes: airline AA (replace preparation)",
        airline_state.clone(),
        None,
    ));
    if pixels {
        let regridded = plots[0]
            .producer
            .with_pixel_grids([(ProjectionId::new("delay")?, delay_grid(20.)?)])?;
        let values = airline_state
            .contributions(&name)?
            .find(|c| {
                c.producer().selection() == regridded.selection()
                    && c.producer().id() == regridded.id()
                    && c.producer().view() == regridded.view()
            })
            .unwrap()
            .value()
            .clone();
        requests.push((
            "Delay grid changes: direct fallback, then replacement",
            airline_state.set(&regridded, values)?,
            Some(regridded),
        ));
    }
    let mut observed = Vec::new();
    let mut previous_sql = HashMap::new();
    for (index, (label, state, new_focus)) in requests.iter().enumerate() {
        println!("\n{}. {label}", index + 1);
        if index == 4 {
            active = prepare_focus(
                &base,
                &source_output,
                &direct,
                &plots,
                &active.producer,
                state,
            )
            .await?;
        }
        // A new grid cannot use old states. The full base query remains available.
        let (base_inputs, inputs, outputs, _) =
            bindings(&base, &full_inputs, &active, &plots, state, force_direct)?;
        let result = active
            .flow
            .query(&outputs, &[], &base_inputs, &inputs)
            .await?;
        print_report(result.report());
        let mut tables = Vec::new();
        for (plot, output) in plots.iter().zip(&outputs) {
            let table = result.table(output)?;
            print_table(plot.name, table)?;
            tables.push(table.clone());
            if show_sql {
                let predicate = plot.filter.predicate(state)?;
                let sql = definition.sql().expr(&predicate)?;
                if previous_sql.get(plot.name) != Some(&sql) {
                    println!("  Full {} membership: {sql}", plot.name);
                    previous_sql.insert(plot.name, sql);
                }
            }
        }
        observed.push((state.clone(), tables));
        if show_sql && (index == 0 || index == 4) {
            for (plot, target) in plots.iter().zip(&active.targets) {
                let Some(target) = target else { continue };
                println!(
                    "\nWarm-up SQL:\n{}",
                    active
                        .definition
                        .sql()
                        .table_output(&target.materialization)?
                );
                println!(
                    "Rollup template SQL:\n{}",
                    active.definition.sql().table_output(&target.output)?
                );
                let p = plot.filter.predicates(state, &active.producer)?;
                if let Ok(split) = p.split() {
                    if let Some(retained) = target.bind(split)? {
                        println!(
                            "Retained-space binding: {}",
                            active.definition.sql().expr(&retained)?
                        );
                    }
                }
            }
        }
        if let Some(focus) = new_focus {
            active = prepare_focus(&base, &source_output, &direct, &plots, focus, state).await?;
            let (base_inputs, inputs, outputs, _) =
                bindings(&base, &full_inputs, &active, &plots, state, force_direct)?;
            let result = active
                .flow
                .query(&outputs, &[], &base_inputs, &inputs)
                .await?;
            println!("  Replacement preparation result:");
            print_report(result.report());
            for (output, table) in outputs.iter().zip(&observed.last().unwrap().1) {
                assert_same_table(result.table(output)?, table)?;
            }
        }
    }
    // Reference requests follow the reported sequence so they cannot warm its cache.
    for (state, tables) in observed {
        let mut inputs = base.inputs();
        for (plot, input) in plots.iter().zip(&full_inputs) {
            inputs = inputs.expr(input, plot.filter.predicate(&state)?)?;
        }
        let expected = base.query(&direct, &[], &inputs.finish()?).await?;
        for (output, table) in direct.iter().zip(tables) {
            assert_same_table(&table, expected.table(output)?)?;
        }
    }
    println!("\nVerified all displayed tables against complete direct queries.");
    if !show_sql {
        println!("Add --sql to inspect plans and predicate expressions.");
    }
    Ok(())
}
