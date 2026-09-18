//! Three cross-filtered counts through the same API for automatic and forced-direct queries.
use std::{ops::Bound, sync::Arc};

use avenger_datafusion_dataflow::{DataflowBuilder, Runtime, TableSnapshot};
use avenger_selection::*;
use datafusion::{
    arrow::{
        array::{Int64Array, StringArray},
        record_batch::RecordBatch,
        util::pretty::print_batches,
    },
    functions_aggregate::expr_fn::count,
    logical_expr::{col, lit, LogicalPlanBuilder},
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
// All requests share one focus and prepared extension. Direct execution
// creates 3, 2, 2, then 0 physical plans. New drag bounds exercise the case
// that pre-aggregation can accelerate; the repeat exercises result caching.
fn brush_states(delay: &ProducerDefinition) -> Result<[(&'static str, SelectionSet, usize); 4]> {
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
        ("inactive", inactive, 3),
        ("brushed [10, 40)", brushed, 2),
        ("dragged [11, 40)", dragged.clone(), 2),
        ("unchanged [11, 40)", dragged, 0),
    ])
}

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let force_direct = std::env::args().any(|a| a == "--force-direct");
    let policy = if force_direct {
        QueryPolicy::ForceDirect
    } else {
        QueryPolicy::Auto
    };
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
        let query = filter.query(imported.plan_ref(), |rows| {
            LogicalPlanBuilder::from(rows)
                .aggregate(vec![group], vec![count(lit(1_i64)).alias("count")])?
                .build()
        })?;
        let family = query.plan(inactive).focus(focus).policy(policy).build()?;
        let label = producer.address().origin.view.as_str();
        println!("{label}: {}", family.explain());
        let panel = family.install(&mut additional, label)?;
        installed.push((label, family, panel));
    }
    let additional = additional.finish()?;
    let sql = additional.sql();
    let extension = base.prepare_extension(&additional).await?;
    for (step, state, expected_plans) in states {
        println!("\n=== {step} ===");
        let mut inputs = extension.inputs();
        let mut outputs = Vec::new();
        for (label, family, panel) in &installed {
            // Native binding exposes the generated plan for inspection. Installed
            // binding supplies the same selection state to the prepared dataflow.
            match family.bind(&state)? {
                BoundQuery::Direct { plan, .. } => {
                    println!("{label} SQL:\n{}\n", sql.plan(&plan)?);
                }
                other => return Err(format!("SQL display does not support {other:?}").into()),
            }
            let binding = panel.bind(&state)?;
            assert!(binding.preaggregate_output().is_none());
            outputs.push(binding.output());
            inputs = binding.apply(inputs)?;
        }
        let result = extension
            .query(&outputs, &[], &base_inputs, &inputs.finish()?)
            .await?;
        println!("{step}: {} physical plans", result.report().physical_plans);
        for ((label, _, _), output) in installed.iter().zip(outputs) {
            println!("{label}");
            print_batches(result.table(&output)?.batches())?;
        }
        assert_eq!(
            result.report().physical_plans,
            expected_plans,
            "direct execution reuses the focused chart during a drag and all outputs on an unchanged repeat"
        );
    }
    Ok(())
}
