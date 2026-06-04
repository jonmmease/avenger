use async_trait::async_trait;
use avenger_chart::prelude::*;
use avenger_chart_core::{
    AvengerChartError, CompiledDataTransform, DataTransformExecutionContext, DataTransformResult,
    DerivedScalarMap,
};
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::{
    arrow::{
        array::{ArrayRef, Float64Array, StructArray},
        datatypes::{DataType, Field, Fields, Schema},
        record_batch::RecordBatch,
    },
    common::ScalarValue,
    dataframe::DataFrame,
    prelude::SessionContext,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[test]
fn test_axis_config_from_channel() {
    // Create a simple plot with axis configuration via channel
    let _symbol = Symbol::<Cartesian>::new()
        .x_with(col("x"), |c| {
            c.scale(|s| s.domain((0.0, 100.0)))
                .axis(|a| a.title("X Axis from Channel"))
        })
        .y_with(col("y"), |c| {
            c.scale(|s| s.domain((0.0, 50.0)))
                .axis(|a| a.title("Y Axis from Channel").grid(true))
        });

    // Test that it compiles - that's the main verification for now
    // In a full test, we would create a plot and render it
}

#[test]
fn test_axis_config_with_no_scale() {
    // Test that axis config works with no_scale channels
    let _symbol = Symbol::<Cartesian>::new()
        .x_with(10.0, |c| c.no_scale().axis(|a| a.title("Fixed X")))
        .y_with(col("y"), |c| c.axis(|a| a.title("Y Column")));
}

#[test]
fn test_axis_config_precedence() {
    // Test that channel-level axis config takes precedence over plot-level
    // Since plot-level axis config is separate from channel-level,
    // we just verify that both syntaxes compile correctly
    let _symbol = Symbol::<Cartesian>::new().x_with(col("x"), |c| {
        c.scale(|s| s.domain((0.0, 100.0)))
            .axis(|a| a.title("Channel Level X"))
    });

    // In practice, "Channel Level X" should be used since marks are processed after plot axes
}

#[test]
fn test_axis_config_on_channel_value_roundtrip() {
    let value: ChannelValue = col("x").into();
    let value = value.with_axis_config(
        CartesianAxis::new()
            .title("Binned x")
            .ticks_start_step(0.0, 2.5),
    );

    let json = serde_json::to_string(&value).expect("serialize channel value");
    let decoded: ChannelValue = serde_json::from_str(&json).expect("deserialize channel value");
    let axis = decoded
        .get_axis_config()
        .expect("axis config")
        .as_any()
        .downcast_ref::<CartesianAxis>()
        .expect("cartesian axis");

    assert!(axis.title.is_set());
    assert!(axis.tick_spacing.is_set());
}

#[derive(Clone)]
struct AxisTickSpacingTransform;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CompiledAxisTickSpacingTransform;

impl DataTransform for AxisTickSpacingTransform {
    type Output = ();

    fn into_compiled_and_output(
        self,
    ) -> Result<(Box<dyn CompiledDataTransform>, Self::Output), AvengerChartError> {
        Ok((Box::new(CompiledAxisTickSpacingTransform), ()))
    }
}

#[typetag::serde(name = "test_axis_derived_tick_spacing")]
#[async_trait]
impl CompiledDataTransform for CompiledAxisTickSpacingTransform {
    fn clone_box(&self) -> Box<dyn CompiledDataTransform> {
        Box::new(self.clone())
    }

    async fn apply(
        &self,
        dataframe: DataFrame,
        _ctx: &DataTransformExecutionContext<'_>,
    ) -> Result<DataTransformResult, AvengerChartError> {
        let mut derived_scalars = DerivedScalarMap::new();
        let fields = Fields::from(vec![
            Field::new("start", DataType::Float64, false),
            Field::new("step", DataType::Float64, false),
        ]);
        let arrays: Vec<ArrayRef> = vec![
            Arc::new(Float64Array::from(vec![0.0])),
            Arc::new(Float64Array::from(vec![2.5])),
        ];
        derived_scalars.insert(
            "x_tick_spacing".to_string(),
            lit(ScalarValue::Struct(Arc::new(StructArray::new(
                fields, arrays, None,
            )))),
        );
        Ok(DataTransformResult {
            dataframe,
            derived_scalars,
        })
    }
}

fn collect_text_labels(mark: &SceneMark, labels: &mut Vec<String>) {
    match mark {
        SceneMark::Text(text) => {
            labels.extend(text.text_iter().cloned());
        }
        SceneMark::Group(group) => {
            for child in &group.marks {
                collect_text_labels(child, labels);
            }
        }
        _ => {}
    }
}

#[tokio::test]
async fn axis_config_resolves_transform_derived_scalar_tick_spacing()
-> Result<(), Box<dyn std::error::Error>> {
    let ctx = SessionContext::new();
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("source_x", DataType::Float64, false),
            Field::new("source_y", DataType::Float64, false),
        ])),
        vec![
            Arc::new(Float64Array::from(vec![0.0, 2.5, 5.0])),
            Arc::new(Float64Array::from(vec![0.2, 0.5, 0.8])),
        ],
    )?;
    let df = ctx.read_batch(batch)?;

    let plot = Plot::<Cartesian>::new()
        .canvas_size(420.0, 320.0)
        .data(df)
        .mark(
            Symbol::new().transform(AxisTickSpacingTransform, |mark, _| {
                mark.x_with(col("source_x"), |c| {
                    c.scale_with::<Linear>(|s| s.domain((0.0, 5.0)).nice(false).zero(false))
                        .axis(|a| {
                            a.title("derived x ticks")
                                .tick_spacing(derived_scalar("x_tick_spacing", None))
                        })
                })
                .y_with(col("source_y"), |c| {
                    c.scale_with::<Linear>(|s| s.domain((0.0, 1.0)).nice(false).zero(false))
                })
                .size(48.0)
                .fill("#2563eb")
            }),
        );

    let compiled = plot.compile(&ctx).await?;
    let mut session = Arc::new(compiled).instantiate(Arc::new(ctx));
    let evaluated = session.evaluate(EvaluationRequest::new().exact()).await?;

    let mut labels = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_text_labels(mark, &mut labels);
    }

    assert!(labels.iter().any(|label| label == "2.5"), "{labels:?}");
    assert!(labels.iter().any(|label| label == "5"), "{labels:?}");
    Ok(())
}

#[tokio::test]
async fn axis_config_resolves_bin_derived_tick_spacing() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = SessionContext::new();
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("source_x", DataType::Float64, false),
            Field::new("source_y", DataType::Float64, false),
        ])),
        vec![
            Arc::new(Float64Array::from(vec![0.3, 1.0, 2.8, 5.3])),
            Arc::new(Float64Array::from(vec![0.2, 0.5, 0.8, 0.4])),
        ],
    )?;
    let df = ctx.read_batch(batch)?;

    let plot = Plot::<Cartesian>::new()
        .canvas_size(420.0, 320.0)
        .data(df)
        .mark(
            Symbol::new().transform(Bin::new(col("source_x")).maxbins(2), |mark, bin| {
                mark.x(bin.start())
                    .y_with(col("source_y"), |c| {
                        c.scale_with::<Linear>(|s| s.domain((0.0, 1.0)).nice(false).zero(false))
                    })
                    .size(48.0)
                    .fill("#2563eb")
            }),
        );

    let compiled = plot.compile(&ctx).await?;
    let mut session = Arc::new(compiled).instantiate(Arc::new(ctx));
    let evaluated = session.evaluate(EvaluationRequest::new().exact()).await?;

    let mut labels = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_text_labels(mark, &mut labels);
    }

    assert!(labels.iter().any(|label| label == "2.8"), "{labels:?}");
    assert!(labels.iter().any(|label| label == "5.3"), "{labels:?}");
    Ok(())
}
