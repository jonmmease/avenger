use avenger_datafusion_dataflow::{DataflowBuilder, Runtime, TableSnapshot};
use avenger_selection::{
    ConsumerFilter, ProducerAddress, ProducerDefinition, ProducerId, Projection, ProjectionId,
    Resolution, SelectionCompiler, SelectionConsumer, SelectionDefinition, SelectionFilter,
    SelectionId, SelectionKind, SelectionSet, SelectionSnapshot, SelectionTerm, SelectionTuple,
    SelectionUpdate, SelectionValue, ValueTest, ViewAddress, ViewId,
};
use datafusion::{
    arrow::{
        array::{ArrayRef, Int64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
        util::pretty::print_batches,
    },
    common::ScalarValue,
    functions_aggregate::expr_fn::count,
    logical_expr::{col, lit, LogicalPlanBuilder},
};
use std::{ops::Bound, sync::Arc};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

struct Plot {
    producer: ProducerDefinition,
    projection: ProjectionId,
    filter: ConsumerFilter,
}
fn plot(selection: &SelectionId, view: &str, field: &str, kind: SelectionKind) -> Result<Plot> {
    let origin = ViewAddress::root(ViewId::new(view)?);
    let projection = ProjectionId::new(field)?;
    let producer = ProducerDefinition::new(
        ProducerAddress {
            selection: selection.clone(),
            producer: ProducerId::new(view)?,
            origin: origin.clone(),
        },
        kind,
        vec![Projection::new(projection.clone(), col(field))?],
    )?;
    let filter = SelectionCompiler::new().filter(
        &SelectionConsumer::new(origin),
        SelectionFilter::cross_filter([selection]),
    )?;
    Ok(Plot {
        producer,
        projection,
        filter,
    })
}
fn range(plot: &Plot, lower: i64, upper: i64) -> SelectionValue {
    SelectionValue::Tuples(vec![SelectionTuple {
        terms: vec![SelectionTerm {
            projection: plot.projection.clone(),
            test: ValueTest::Range {
                lower: Bound::Included(lower.into()),
                upper: Bound::Excluded(upper.into()),
            },
        }],
    }])
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
    let delay = plot(
        &selection,
        "delay_histogram",
        "delay",
        SelectionKind::Interval,
    )?;
    let distance = plot(
        &selection,
        "distance_histogram",
        "distance",
        SelectionKind::Interval,
    )?;
    let airlines = plot(&selection, "airline_bars", "carrier", SelectionKind::Point)?;
    let mut state = SelectionSet::new([SelectionSnapshot::new(SelectionDefinition::new(
        selection.clone(),
        Resolution::Intersect,
    ))?])?;
    state = state.apply_all([
        (
            selection.clone(),
            SelectionUpdate::set(&delay.producer, range(&delay, 10, 30)),
        ),
        (
            selection.clone(),
            SelectionUpdate::set(&distance.producer, range(&distance, 500, 1500)),
        ),
        (
            selection.clone(),
            SelectionUpdate::set(
                &airlines.producer,
                SelectionValue::Tuples(vec![SelectionTuple {
                    terms: vec![SelectionTerm {
                        projection: airlines.projection.clone(),
                        test: ValueTest::OneOf(vec![
                            ScalarValue::from("AA"),
                            ScalarValue::from("DL"),
                        ]),
                    }],
                }]),
            ),
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
        let name = plot.producer.address().origin.view.as_str();
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
            println!("{}", plot.producer.address().origin.view);
            print_batches(result.table(output)?.batches())?;
        }
        if step == 0 {
            state = state.apply(
                &selection,
                SelectionUpdate::set(&delay.producer, range(&delay, 20, 40)),
            )?;
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
