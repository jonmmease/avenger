use std::sync::Arc;

use avenger_chart::prelude::*;
use avenger_chart::{plot::CompiledPlot, render::EvaluatedEventDatumRows};
use avenger_scenegraph::marks::{
    line::SceneLineMark,
    mark::{MarkInstance, SceneMark},
};
use datafusion::arrow::{
    array::{Array, Float64Array, Int32Array, StringArray},
    datatypes::{DataType, Field, Schema},
    record_batch::RecordBatch,
};
use datafusion::prelude::*;

fn theta_data() -> DataFrame {
    let schema = Arc::new(Schema::new(vec![Field::new(
        "theta",
        DataType::Float64,
        false,
    )]));
    let batch = RecordBatch::try_new(schema, vec![Arc::new(Float64Array::from(vec![0.0, 1.0]))])
        .expect("create polar line data");
    SessionContext::new()
        .read_batch(batch)
        .expect("read polar line data")
}

fn id_theta_data() -> DataFrame {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("theta", DataType::Float64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(vec!["a", "b"])),
            Arc::new(Float64Array::from(vec![0.0, 1.0])),
        ],
    )
    .expect("create polar line event data");
    SessionContext::new()
        .read_batch(batch)
        .expect("read polar line event data")
}

fn ordered_theta_data() -> DataFrame {
    let schema = Arc::new(Schema::new(vec![
        Field::new("theta", DataType::Float64, false),
        Field::new("order", DataType::Int32, false),
    ]));
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Float64Array::from(vec![
                std::f64::consts::PI,
                0.0,
                std::f64::consts::FRAC_PI_2,
            ])),
            Arc::new(Int32Array::from(vec![3, 1, 2])),
        ],
    )
    .expect("create ordered polar line data");
    SessionContext::new()
        .read_batch(batch)
        .expect("read ordered polar line data")
}

fn collect_lines<'a>(mark: &'a SceneMark, lines: &mut Vec<&'a SceneLineMark>) {
    if let SceneMark::Line(line) = mark
        && line.name == "line"
    {
        lines.push(line);
    }
    for child in mark.children() {
        collect_lines(child, lines);
    }
}

async fn rendered_line_len(compiled: &CompiledPlot, ctx: &SessionContext) -> u32 {
    let evaluated = compiled
        .evaluate(ctx, None)
        .await
        .expect("evaluate polar line plot");

    let mut lines = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_lines(mark, &mut lines);
    }
    assert_eq!(lines.len(), 1, "expected one rendered polar line mark");
    lines[0].len
}

fn event_datum_rows_with_id(rows: &[EvaluatedEventDatumRows]) -> Vec<&EvaluatedEventDatumRows> {
    rows.iter()
        .filter(|rows| rows.rows.column_by_name("id").is_some())
        .collect()
}

fn event_ids(rows: &EvaluatedEventDatumRows) -> Vec<String> {
    let values = rows
        .rows
        .column_by_name("id")
        .expect("id datum")
        .as_any()
        .downcast_ref::<StringArray>()
        .expect("string id datum");
    (0..values.len())
        .map(|index| values.value(index).to_string())
        .collect()
}

#[tokio::test]
async fn polar_line_defaults_to_coordinate_space_densification() {
    let ctx = SessionContext::new();
    let plot = Plot::<Polar>::new()
        .data(theta_data())
        .mark(Line::<Polar>::new().r(50.0).theta(col("theta")));
    let compiled = plot.compile(&ctx).await.expect("compile polar line plot");
    let len = rendered_line_len(&compiled, &ctx).await;
    assert!(
        len > 2,
        "coordinate-space polar line should densify beyond source rows"
    );
}

#[tokio::test]
async fn polar_line_display_space_keeps_source_vertices() {
    let ctx = SessionContext::new();
    let plot = Plot::<Polar>::new().data(theta_data()).mark(
        Line::<Polar>::new()
            .r(50.0)
            .theta(col("theta"))
            .geometry_space(GeometrySpace::Display),
    );
    let compiled = plot.compile(&ctx).await.expect("compile polar line plot");
    let len = rendered_line_len(&compiled, &ctx).await;
    assert_eq!(len, 2);
}

#[tokio::test]
async fn polar_line_display_geometry_space_survives_bincode_round_trip() {
    let ctx = SessionContext::new();
    let plot = Plot::<Polar>::new().data(theta_data()).mark(
        Line::<Polar>::new()
            .r(50.0)
            .theta(col("theta"))
            .geometry_space(GeometrySpace::Display),
    );
    let compiled = plot.compile(&ctx).await.expect("compile polar line plot");
    let serialized = bincode::serialize(&compiled).expect("serialize compiled polar line plot");
    let deserialized: CompiledPlot =
        bincode::deserialize(&serialized).expect("deserialize compiled polar line plot");

    let len = rendered_line_len(&deserialized, &ctx).await;
    assert_eq!(len, 2);
}

#[tokio::test]
async fn polar_line_event_datums_keep_source_rows_for_densified_mark_level_line() {
    let ctx = SessionContext::new();
    let plot = Plot::<Polar>::new()
        .data(id_theta_data())
        .mark(Line::<Polar>::new().r(50.0).theta(col("theta")))
        .event_binding(
            ChartEventBinding::on(ChartEventType::Click)
                .filter(avenger_chart::event::datum("id").is_not_null()),
        );
    let evaluated = plot
        .compile(&ctx)
        .await
        .expect("compile polar line plot")
        .evaluate(&ctx, None)
        .await
        .expect("evaluate polar line plot");

    let mut lines = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_lines(mark, &mut lines);
    }
    assert_eq!(lines.len(), 1, "expected one rendered polar line mark");
    assert!(
        lines[0].len as usize > 2,
        "coordinate-space line should be densified"
    );

    let rows_with_id = event_datum_rows_with_id(&evaluated.event_datums.rows);
    assert_eq!(rows_with_id.len(), 1);
    assert_eq!(rows_with_id[0].rows.num_rows(), 2);
    assert_eq!(event_ids(rows_with_id[0]), vec!["a", "b"]);

    let mark_level_line_hit = MarkInstance {
        name: "line".to_string(),
        mark_path: rows_with_id[0].mark_path.clone(),
        instance_index: None,
    };
    assert_eq!(
        evaluated
            .event_datums
            .datum_for_mark_instance(Some(&mark_level_line_hit), "id"),
        None
    );
}

#[tokio::test]
async fn polar_line_order_channel_controls_vertex_order() {
    let ctx = SessionContext::new();
    let plot = Plot::<Polar>::new()
        .plot_size(200.0, 200.0)
        .data(ordered_theta_data())
        .mark(
            Line::<Polar>::new()
                .r(50.0)
                .theta_with(col("theta"), |c| c.no_scale())
                .order(col("order"))
                .geometry_space(GeometrySpace::Display),
        );
    let compiled = plot.compile(&ctx).await.expect("compile polar line plot");
    let evaluated = compiled
        .evaluate(&ctx, None)
        .await
        .expect("evaluate polar line plot");

    let mut lines = Vec::new();
    for mark in evaluated.scene_graph.children() {
        collect_lines(mark, &mut lines);
    }
    assert_eq!(lines.len(), 1, "expected one rendered polar line mark");
    assert_eq!(lines[0].len, 3);

    let x = lines[0].x.as_vec(3, None);
    let y = lines[0].y.as_vec(3, None);
    assert!((x[0] - 150.0).abs() < 1e-4);
    assert!((y[0] - 100.0).abs() < 1e-4);
    assert!((x[1] - 100.0).abs() < 1e-4);
    assert!((y[1] - 150.0).abs() < 1e-4);
    assert!((x[2] - 50.0).abs() < 1e-4);
    assert!((y[2] - 100.0).abs() < 1e-4);
}
