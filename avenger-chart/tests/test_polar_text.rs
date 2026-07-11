mod mark_effects_support;

use std::{
    f64::consts::{FRAC_PI_2, PI},
    sync::Arc,
};

use avenger_chart::prelude::*;
use avenger_chart::{plot::CompiledPlot, render::EvaluatedEventDatumRows};
use avenger_chart_core::AvengerChartError;
use avenger_scenegraph::marks::{
    mark::{MarkInstance, SceneMark},
    text::SceneTextMark,
};
use avenger_text::types::TextSyntaxMode;
use datafusion::{
    arrow::{
        array::{ArrayRef, Float32Array, Float64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    common::ScalarValue,
    prelude::*,
};
use mark_effects_support::KeepUprightText;

fn read_batch(fields: Vec<Field>, columns: Vec<ArrayRef>) -> DataFrame {
    let batch = RecordBatch::try_new(Arc::new(Schema::new(fields)), columns)
        .expect("create polar text data");
    SessionContext::new()
        .read_batch(batch)
        .expect("read polar text data")
}

fn theta_data(values: Vec<f64>) -> DataFrame {
    read_batch(
        vec![Field::new("theta", DataType::Float64, false)],
        vec![Arc::new(Float64Array::from(values)) as ArrayRef],
    )
}

fn r_angle_data(r: Vec<f64>, angle: Vec<f64>) -> DataFrame {
    read_batch(
        vec![
            Field::new("r", DataType::Float64, false),
            Field::new("angle", DataType::Float64, false),
        ],
        vec![
            Arc::new(Float64Array::from(r)) as ArrayRef,
            Arc::new(Float64Array::from(angle)) as ArrayRef,
        ],
    )
}

fn categorical_theta_data(values: Vec<&str>) -> DataFrame {
    read_batch(
        vec![Field::new("theta", DataType::Utf8, false)],
        vec![Arc::new(StringArray::from(values)) as ArrayRef],
    )
}

fn collect_texts<'a>(mark: &'a SceneMark, texts: &mut Vec<&'a SceneTextMark>) {
    match mark {
        SceneMark::Text(text) if text.name == "text" => texts.push(text),
        SceneMark::Group(group) => {
            for child in &group.marks {
                collect_texts(child, texts);
            }
        }
        _ => {}
    }
}

async fn rendered_texts(compiled: &CompiledPlot, ctx: &SessionContext) -> Vec<SceneTextMark> {
    let evaluated = compiled
        .evaluate(ctx, None)
        .await
        .expect("evaluate polar text plot");
    let mut texts = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_texts(mark, &mut texts);
    }
    assert_eq!(texts.len(), 1, "expected one rendered polar text mark");
    texts.into_iter().cloned().collect()
}

fn roundtrip_compiled_plot(compiled: &CompiledPlot) -> CompiledPlot {
    let serialized = bincode::serialize(compiled).expect("serialize compiled polar text plot");
    bincode::deserialize(&serialized).expect("deserialize compiled polar text plot")
}

fn assert_close(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() < 1e-4,
        "expected {expected}, got {actual}"
    );
}

fn assert_angles_close(actual: Vec<f32>, expected: Vec<f32>) {
    assert_eq!(actual.len(), expected.len());
    for (actual, expected) in actual.into_iter().zip(expected) {
        let delta = ((actual - expected + 180.0).rem_euclid(360.0) - 180.0).abs();
        assert!(
            delta < 1e-4,
            "expected angle {expected}, got {actual} with delta {delta}"
        );
    }
}

fn event_datum_rows_with_id(rows: &[EvaluatedEventDatumRows]) -> Vec<&EvaluatedEventDatumRows> {
    rows.iter()
        .filter(|rows| rows.rows.column_by_name("id").is_some())
        .collect()
}

#[tokio::test]
async fn polar_text_typst_mode_reaches_scene_mark() -> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let plot = Chart::<Polar>::new()
        .plot_size(200.0, 200.0)
        .data(theta_data(vec![FRAC_PI_2]))
        .mark(
            Text::<Polar>::new()
                .r(50.0)
                .theta_with(col("theta"), |c| c.no_scale())
                .text("$R^2$")
                .typst(),
        );

    let compiled = plot.compile(&ctx).await?;
    let text = rendered_texts(&compiled, &ctx).await.remove(0);
    assert_eq!(text.text_syntax, TextSyntaxMode::TypstMarkup);
    Ok(())
}

#[tokio::test]
async fn polar_text_display_space_preserves_cartesian_angle() -> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let plot = Chart::<Polar>::new()
        .plot_size(200.0, 200.0)
        .data(theta_data(vec![FRAC_PI_2]))
        .mark(
            Text::<Polar>::new()
                .r(50.0)
                .theta_with(col("theta"), |c| c.no_scale())
                .geometry_space(GeometrySpace::Display)
                .text("A")
                .angle(0.0),
        );

    let compiled = plot.compile(&ctx).await?;
    let text = rendered_texts(&compiled, &ctx).await.remove(0);

    assert_close(text.x.as_vec(text.len as usize, None)[0], 100.0);
    assert_close(text.y.as_vec(text.len as usize, None)[0], 150.0);
    assert_angles_close(text.angle.as_vec(text.len as usize, None), vec![0.0]);
    Ok(())
}

#[tokio::test]
async fn polar_text_coordinate_space_radial_angles() -> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let plot = Chart::<Polar>::new()
        .plot_size(200.0, 200.0)
        .data(theta_data(vec![0.0, FRAC_PI_2, PI]))
        .mark(
            Text::<Polar>::new()
                .r(50.0)
                .theta_with(col("theta"), |c| c.no_scale())
                .text("A")
                .angle(0.0),
        );

    let compiled = plot.compile(&ctx).await?;
    let text = rendered_texts(&compiled, &ctx).await.remove(0);
    assert_angles_close(
        text.angle.as_vec(text.len as usize, None),
        vec![0.0, 90.0, 180.0],
    );
    Ok(())
}

#[tokio::test]
async fn polar_text_coordinate_space_tangential_angles() -> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let plot = Chart::<Polar>::new()
        .plot_size(200.0, 200.0)
        .data(theta_data(vec![0.0, FRAC_PI_2]))
        .mark(
            Text::<Polar>::new()
                .r(50.0)
                .theta_with(col("theta"), |c| c.no_scale())
                .geometry_space(GeometrySpace::Coordinate)
                .text("A")
                .angle(90.0),
        );

    let compiled = plot.compile(&ctx).await?;
    let text = rendered_texts(&compiled, &ctx).await.remove(0);
    assert_angles_close(
        text.angle.as_vec(text.len as usize, None),
        vec![90.0, 180.0],
    );
    Ok(())
}

#[tokio::test]
async fn polar_text_coordinate_space_uses_scaled_theta() -> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let plot = Chart::<Polar>::new()
        .plot_size(200.0, 200.0)
        .data(theta_data(vec![0.0, 25.0, 50.0]))
        .mark(
            Text::<Polar>::new()
                .r(50.0)
                .theta_with(col("theta"), |c| {
                    c.scale_with::<Linear>(|s| s.domain((lit(0.0), lit(100.0))))
                })
                .text("A")
                .angle(0.0),
        );

    let compiled = plot.compile(&ctx).await?;
    let text = rendered_texts(&compiled, &ctx).await.remove(0);
    assert_angles_close(
        text.angle.as_vec(text.len as usize, None),
        vec![0.0, 90.0, 180.0],
    );
    Ok(())
}

#[tokio::test]
async fn polar_text_coordinate_space_uses_categorical_theta_scale() -> Result<(), AvengerChartError>
{
    let ctx = SessionContext::new();
    let plot = Chart::<Polar>::new()
        .plot_size(200.0, 200.0)
        .data(categorical_theta_data(vec!["N", "E", "S", "W"]))
        .mark(
            Text::<Polar>::new()
                .r(50.0)
                .theta_with(col("theta"), |c| {
                    c.scale_with::<Point>(|s| s.round(false))
                        .axis(|a| a.visible(false))
                })
                .text(col("theta"))
                .angle(0.0),
        );

    let compiled = plot.compile(&ctx).await?;
    let text = rendered_texts(&compiled, &ctx).await.remove(0);
    assert_eq!(text.len, 4);

    let x = text.x.as_vec(text.len as usize, None);
    let y = text.y.as_vec(text.len as usize, None);
    let x_span = x.iter().copied().fold(f32::NEG_INFINITY, f32::max)
        - x.iter().copied().fold(f32::INFINITY, f32::min);
    let y_span = y.iter().copied().fold(f32::NEG_INFINITY, f32::max)
        - y.iter().copied().fold(f32::INFINITY, f32::min);
    let distinct_positions = x
        .iter()
        .zip(&y)
        .map(|(x, y)| (x.round() as i32, y.round() as i32))
        .collect::<std::collections::HashSet<_>>();
    assert!(
        distinct_positions.len() > 2,
        "categorical theta text should spread around the polar plot, got {distinct_positions:?}"
    );
    assert!(
        x_span > 50.0 && y_span > 50.0,
        "categorical theta text should span the polar plot, got x={x:?} y={y:?}"
    );

    let angles = text.angle.as_vec(text.len as usize, None);
    let angle_span = angles.iter().copied().fold(f32::NEG_INFINITY, f32::max)
        - angles.iter().copied().fold(f32::INFINITY, f32::min);
    let distinct_angles = angles
        .iter()
        .map(|angle| angle.round() as i32)
        .collect::<std::collections::HashSet<_>>();
    assert!(
        distinct_angles.len() > 2,
        "categorical theta text should use scaled theta for orientation, got {angles:?}"
    );
    assert!(
        angle_span > 180.0,
        "categorical theta text should cover a broad angle span, got {angles:?}"
    );
    Ok(())
}

#[tokio::test]
async fn polar_text_mixed_scalar_array_positions_and_angles() -> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let plot = Chart::<Polar>::new()
        .plot_size(200.0, 200.0)
        .data(r_angle_data(vec![20.0, 40.0], vec![0.0, 90.0]))
        .mark(
            Text::<Polar>::new()
                .r_with(col("r"), |c| c.no_scale())
                .theta(FRAC_PI_2)
                .text("A")
                .angle(ChannelValue::from(col("angle")).no_scale()),
        );

    let compiled = plot.compile(&ctx).await?;
    let text = rendered_texts(&compiled, &ctx).await.remove(0);
    assert_eq!(text.len, 2);
    assert_close(text.x.as_vec(text.len as usize, None)[0], 100.0);
    assert_close(text.x.as_vec(text.len as usize, None)[1], 100.0);
    assert_close(text.y.as_vec(text.len as usize, None)[0], 120.0);
    assert_close(text.y.as_vec(text.len as usize, None)[1], 140.0);
    assert_angles_close(
        text.angle.as_vec(text.len as usize, None),
        vec![90.0, 180.0],
    );
    Ok(())
}

#[tokio::test]
async fn polar_text_display_geometry_space_survives_bincode_round_trip()
-> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let plot = Chart::<Polar>::new()
        .plot_size(200.0, 200.0)
        .data(theta_data(vec![FRAC_PI_2]))
        .mark(
            Text::<Polar>::new()
                .r(50.0)
                .theta_with(col("theta"), |c| c.no_scale())
                .geometry_space(GeometrySpace::Display)
                .text("A")
                .angle(0.0),
        );

    let compiled = plot.compile(&ctx).await?;
    let decoded = roundtrip_compiled_plot(&compiled);
    let text = rendered_texts(&decoded, &ctx).await.remove(0);
    assert_angles_close(text.angle.as_vec(text.len as usize, None), vec![0.0]);
    Ok(())
}

#[tokio::test]
async fn polar_text_default_coordinate_space_survives_bincode_round_trip()
-> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let plot = Chart::<Polar>::new()
        .plot_size(200.0, 200.0)
        .data(theta_data(vec![FRAC_PI_2]))
        .mark(
            Text::<Polar>::new()
                .r(50.0)
                .theta_with(col("theta"), |c| c.no_scale())
                .text("A")
                .angle(0.0),
        );

    let compiled = plot.compile(&ctx).await?;
    let decoded = roundtrip_compiled_plot(&compiled);
    let text = rendered_texts(&decoded, &ctx).await.remove(0);
    assert_angles_close(text.angle.as_vec(text.len as usize, None), vec![90.0]);
    Ok(())
}

#[tokio::test]
async fn polar_text_adjustments_see_computed_display_angle() -> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let plot = Chart::<Polar>::new()
        .plot_size(200.0, 200.0)
        .data(theta_data(vec![FRAC_PI_2]))
        .mark(
            Text::<Polar>::new()
                .r(50.0)
                .theta_with(col("theta"), |c| c.no_scale())
                .text("old")
                .angle(0.0)
                .adjust(|text| {
                    text.angle(text.channel("angle") + lit(180.0))
                        .text(lit("adjusted"))
                        .defined(lit(false))
                }),
        );

    let compiled = plot.compile(&ctx).await?;
    let text = rendered_texts(&compiled, &ctx).await.remove(0);
    assert_angles_close(text.angle.as_vec(text.len as usize, None), vec![270.0]);
    assert_eq!(
        text.text.as_vec(text.len as usize, None),
        vec!["adjusted".to_string()]
    );
    assert_eq!(text.defined.as_vec(text.len as usize, None), vec![false]);
    Ok(())
}

#[tokio::test]
async fn polar_text_expression_adjustment_updates_position_text_defined_and_source_data()
-> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let df = read_batch(
        vec![
            Field::new("shift", DataType::Float32, false),
            Field::new("label", DataType::Utf8, false),
            Field::new("theta", DataType::Float64, false),
        ],
        vec![
            Arc::new(Float32Array::from(vec![1.0, 2.0])) as ArrayRef,
            Arc::new(StringArray::from(vec!["a", "b"])) as ArrayRef,
            Arc::new(Float64Array::from(vec![0.0, 0.0])) as ArrayRef,
        ],
    );
    let plot = Chart::<Polar>::new().plot_size(200.0, 200.0).data(df).mark(
        Text::<Polar>::new()
            .r(50.0)
            .theta_with(col("theta"), |c| c.no_scale())
            .text("old")
            .angle(0.0)
            .adjust(|text| {
                text.x(text.channel("x") + text.data("shift"))
                    .y(text.channel("y") + lit(3.0))
                    .angle(text.channel("angle") + lit(180.0))
                    .defined(lit(false))
                    .text(text.data("label"))
            }),
    );

    let compiled = plot.compile(&ctx).await?;
    let text = rendered_texts(&compiled, &ctx).await.remove(0);
    assert_eq!(text.x.as_vec(text.len as usize, None), vec![151.0, 152.0]);
    assert_eq!(text.y.as_vec(text.len as usize, None), vec![103.0, 103.0]);
    assert_angles_close(
        text.angle.as_vec(text.len as usize, None),
        vec![180.0, 180.0],
    );
    assert_eq!(
        text.text.as_vec(text.len as usize, None),
        vec!["a".to_string(), "b".to_string()]
    );
    assert_eq!(
        text.defined.as_vec(text.len as usize, None),
        vec![false, false]
    );
    Ok(())
}

#[tokio::test]
async fn polar_text_keep_upright_adjustment_flips_left_half_radial() -> Result<(), AvengerChartError>
{
    let ctx = SessionContext::new();
    let plot = Chart::<Polar>::new()
        .plot_size(200.0, 200.0)
        .data(theta_data(vec![0.0, FRAC_PI_2, PI, 3.0 * FRAC_PI_2]))
        .mark(
            Text::<Polar>::new()
                .r(50.0)
                .theta_with(col("theta"), |c| c.no_scale())
                .text("A")
                .angle(0.0)
                .adjust_transform(KeepUprightText::new(), |text, upright| {
                    text.angle(upright.angle())
                }),
        );

    let compiled = plot.compile(&ctx).await?;
    let text = rendered_texts(&compiled, &ctx).await.remove(0);
    assert_angles_close(
        text.angle.as_vec(text.len as usize, None),
        vec![0.0, 90.0, 360.0, 270.0],
    );
    Ok(())
}

#[tokio::test]
async fn polar_text_keep_upright_adjustment_flips_tangential_labels()
-> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let plot = Chart::<Polar>::new()
        .plot_size(200.0, 200.0)
        .data(theta_data(vec![0.0, FRAC_PI_2, PI]))
        .mark(
            Text::<Polar>::new()
                .r(50.0)
                .theta_with(col("theta"), |c| c.no_scale())
                .text("A")
                .angle(90.0)
                .adjust_transform(KeepUprightText::new(), |text, upright| {
                    text.angle(upright.angle())
                }),
        );

    let compiled = plot.compile(&ctx).await?;
    let text = rendered_texts(&compiled, &ctx).await.remove(0);
    assert_angles_close(
        text.angle.as_vec(text.len as usize, None),
        vec![90.0, 360.0, 270.0],
    );
    Ok(())
}

#[tokio::test]
async fn polar_text_event_datums_keep_source_rows() -> Result<(), AvengerChartError> {
    let ctx = SessionContext::new();
    let df = read_batch(
        vec![
            Field::new("id", DataType::Utf8, false),
            Field::new("theta", DataType::Float64, false),
        ],
        vec![
            Arc::new(StringArray::from(vec!["a", "b"])) as ArrayRef,
            Arc::new(Float64Array::from(vec![0.0, FRAC_PI_2])) as ArrayRef,
        ],
    );
    let plot = Chart::<Polar>::new()
        .plot_size(200.0, 200.0)
        .data(df)
        .mark(
            Text::<Polar>::new()
                .r(50.0)
                .theta_with(col("theta"), |c| c.no_scale())
                .text(col("id"))
                .angle(0.0),
        )
        .event_binding(
            ChartEventBinding::on(ChartEventType::Click)
                .filter(avenger_chart::event::datum("id").is_not_null()),
        );

    let evaluated = plot
        .compile(&ctx)
        .await?
        .evaluate(&ctx, None)
        .await
        .expect("evaluate polar text plot");
    let mut texts = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_texts(mark, &mut texts);
    }
    assert_eq!(texts.len(), 1, "expected one rendered polar text mark");
    assert_eq!(texts[0].len, 2);

    let rows_with_id = event_datum_rows_with_id(&evaluated.event_datums.rows);
    assert_eq!(rows_with_id.len(), 1);
    assert_eq!(rows_with_id[0].rows.num_rows(), 2);

    let second_text_hit = MarkInstance {
        name: "text".to_string(),
        mark_path: rows_with_id[0].mark_path.clone(),
        instance_index: Some(1),
    };
    assert_eq!(
        evaluated
            .event_datums
            .datum_for_mark_instance(Some(&second_text_hit), "id"),
        Some(ScalarValue::Utf8(Some("b".to_string())))
    );
    Ok(())
}
