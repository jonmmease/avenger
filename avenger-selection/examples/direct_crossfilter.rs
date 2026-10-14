use avenger_datafusion_dataflow::{DataflowBuilder, Runtime, TableSnapshot};
use avenger_scales_datafusion::BuiltinScale;
use avenger_selection::{
    ConsumerFilter, PixelGrid, ProducerDefinition, ProducerId, Projection, ProjectionId,
    Resolution, SelectionFilter, SelectionId, SelectionSet, SelectionUpdate, SelectionValue,
    ValueTest, ViewId,
};
use datafusion::{
    arrow::{
        array::{ArrayRef, Float32Array, Int64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
        util::pretty::print_batches,
    },
    functions_aggregate::expr_fn::count,
    logical_expr::{col, lit, LogicalPlanBuilder},
};
use std::sync::Arc;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

struct Plot {
    producer: ProducerDefinition,
    projection: ProjectionId,
    filter: ConsumerFilter,
}
fn plot(selection: &SelectionId, view: &str, field: &str) -> Result<Plot> {
    let origin = ViewId::new(view)?;
    let projection = ProjectionId::new(field)?;
    let producer = ProducerDefinition::new(
        selection.clone(),
        ProducerId::new(view)?,
        origin.clone(),
        vec![Projection::new(projection.clone(), col(field))?],
    )?;
    let filter = ConsumerFilter::new(origin, SelectionFilter::cross_filter([selection]));
    Ok(Plot {
        producer,
        projection,
        filter,
    })
}
fn range(plot: &Plot, lower: i64, upper: i64) -> SelectionValue {
    SelectionValue::tuple([(plot.projection.clone(), ValueTest::range(lower..upper))])
}
fn flights() -> Result<TableSnapshot> {
    let columns: Vec<(&str, ArrayRef)> = vec![
        (
            "delay",
            Arc::new(Int64Array::from(vec![0, 10, 20, 30, 20, 20, 15])),
        ),
        (
            "distance",
            Arc::new(Int64Array::from(vec![600, 600, 1000, 600, 2000, 600, 800])),
        ),
        (
            "carrier",
            Arc::new(StringArray::from(vec![
                "AA", "AA", "DL", "AA", "AA", "UA", "UA",
            ])),
        ),
        (
            "delay_bin",
            Arc::new(Int64Array::from(vec![0, 1, 2, 3, 2, 2, 1])),
        ),
        (
            "distance_bin",
            Arc::new(Int64Array::from(vec![2, 2, 4, 2, 8, 2, 3])),
        ),
    ];
    let schema = Arc::new(Schema::new(
        columns
            .iter()
            .map(|(name, array)| Field::new(*name, array.data_type().clone(), false))
            .collect::<Vec<_>>(),
    ));
    let batch = RecordBatch::try_new(
        schema.clone(),
        columns.into_iter().map(|(_, array)| array).collect(),
    )?;
    Ok(TableSnapshot::from_batches(schema, vec![batch])?)
}

#[tokio::main]
async fn main() -> Result<()> {
    let selection = SelectionId::new("filters")?;
    let mut delay = plot(&selection, "delay_histogram", "delay")?;
    let mut distance = plot(&selection, "distance_histogram", "distance")?;
    let airlines = plot(&selection, "airline_bars", "carrier")?;
    if std::env::args().any(|arg| arg == "--pixels") {
        for (plot, domain) in [(&mut delay, [-20.0, 60.0]), (&mut distance, [0.0, 3000.0])] {
            let grid = PixelGrid::new(
                BuiltinScale::Linear,
                Arc::new(Float32Array::from(domain.to_vec())),
                Arc::new(Float32Array::from(vec![0.0, 600.0])),
                Default::default(),
                0.0,
                2.0,
            )?;
            plot.producer = plot
                .producer
                .with_pixel_grids([(plot.projection.clone(), grid)])?;
        }
        println!("Using two-logical-pixel interval membership");
    }
    let mut state = SelectionSet::new([(selection.clone(), Resolution::Intersect)])?;
    state = state.apply_all([
        SelectionUpdate::set(&delay.producer, range(&delay, 10, 30)),
        SelectionUpdate::set(&distance.producer, range(&distance, 500, 1500)),
        SelectionUpdate::set(
            &airlines.producer,
            SelectionValue::tuple([(airlines.projection.clone(), ValueTest::one_of(["AA", "DL"]))]),
        ),
    ])?;

    let mut builder = DataflowBuilder::new();
    let source = builder.table_snapshot("flights", flights()?)?;
    let mut panels = Vec::new();
    for (plot, group) in [
        (&delay, "delay_bin"),
        (&distance, "distance_bin"),
        (&airlines, "carrier"),
    ] {
        let name = plot.producer.view().as_str();
        let input = builder.expr_input(name, DataType::Boolean)?;
        let counts = builder.add_plan(
            format!("{name}_counts"),
            LogicalPlanBuilder::from(source.plan_ref())
                .filter(input.expr_ref())?
                .aggregate(vec![col(group)], vec![count(lit(1_i64)).alias("count")])?
                .sort(vec![col(group).sort(true, true)])?
                .build()?,
        )?;
        let output = builder.table_output(name, &counts)?;
        panels.push((plot, input, output));
    }
    let prepared = Runtime::new(Default::default())?
        .prepare(&builder.finish()?)
        .await?;
    let outputs: Vec<_> = panels.iter().map(|(_, _, out)| *out).collect();
    for step in 0..3 {
        let mut inputs = prepared.inputs();
        for (plot, input, _) in &panels {
            inputs = inputs.expr(input, plot.filter.predicate(&state)?)?;
        }
        let result = prepared.query(&outputs, &[], &inputs.finish()?).await?;
        println!(
            "Update {step}: {} physical plans",
            result.report().physical_plans
        );
        for (plot, _, output) in &panels {
            println!("{}", plot.producer.view());
            print_batches(result.table(output)?.batches())?;
        }
        if step == 0 {
            state = state.set(&delay.producer, range(&delay, 20, 40))?;
        } else if step == 1 {
            assert_eq!(
                result.report().physical_plans,
                2,
                "the delay view excludes its own brush and reuses its result"
            );
        } else {
            assert_eq!(
                result.report().physical_plans,
                0,
                "unchanged bindings reuse all outputs"
            );
        }
    }
    Ok(())
}
