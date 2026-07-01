use std::sync::Arc;

use avenger_chart::prelude::*;
use avenger_scenegraph::marks::{image::SceneImageMark, mark::SceneMark};
use datafusion::{
    arrow::{
        array::{
            ArrayRef, Float64Array, Float64Builder, ListBuilder, StringArray, StringBuilder,
            StructArray, UInt32Array, UInt32Builder,
        },
        datatypes::{DataType, Field},
        record_batch::RecordBatch,
    },
    common::ScalarValue,
    dataframe::DataFrame,
    prelude::{SessionContext, col},
};
use indexmap::IndexMap;

#[derive(Clone, Copy)]
struct UniformAxis {
    coord: &'static str,
    start: f64,
    stop: f64,
    count: u32,
    sampling: Option<&'static str>,
}

impl UniformAxis {
    fn new(coord: &'static str, start: f64, stop: f64, count: u32) -> Self {
        Self {
            coord,
            start,
            stop,
            count,
            sampling: None,
        }
    }
}

struct F64Raster {
    columns: UniformAxis,
    rows: UniformAxis,
    values: Vec<Option<f64>>,
}

fn axis_array(axes: impl IntoIterator<Item = UniformAxis>) -> StructArray {
    let axes = axes.into_iter().collect::<Vec<_>>();
    StructArray::from(vec![
        (
            Arc::new(Field::new("coord", DataType::Utf8, false)),
            Arc::new(StringArray::from(
                axes.iter().map(|axis| axis.coord).collect::<Vec<_>>(),
            )) as ArrayRef,
        ),
        (
            Arc::new(Field::new("sampling", DataType::Utf8, true)),
            Arc::new(StringArray::from(
                axes.iter().map(|axis| axis.sampling).collect::<Vec<_>>(),
            )) as ArrayRef,
        ),
        (
            Arc::new(Field::new("start", DataType::Float64, false)),
            Arc::new(Float64Array::from(
                axes.iter().map(|axis| axis.start).collect::<Vec<_>>(),
            )) as ArrayRef,
        ),
        (
            Arc::new(Field::new("stop", DataType::Float64, false)),
            Arc::new(Float64Array::from(
                axes.iter().map(|axis| axis.stop).collect::<Vec<_>>(),
            )) as ArrayRef,
        ),
        (
            Arc::new(Field::new("count", DataType::UInt32, false)),
            Arc::new(UInt32Array::from(
                axes.iter().map(|axis| axis.count).collect::<Vec<_>>(),
            )) as ArrayRef,
        ),
    ])
}

fn f64_list_rows(rows: &[Vec<Option<f64>>]) -> ArrayRef {
    let mut builder = ListBuilder::new(Float64Builder::new());
    for row in rows {
        for value in row {
            if let Some(value) = value {
                builder.values().append_value(*value);
            } else {
                builder.values().append_null();
            }
        }
        builder.append(true);
    }
    Arc::new(builder.finish()) as ArrayRef
}

fn string_list_rows(rows: &[Vec<&str>]) -> ArrayRef {
    let mut builder = ListBuilder::new(StringBuilder::new());
    for row in rows {
        for value in row {
            builder.values().append_value(*value);
        }
        builder.append(true);
    }
    Arc::new(builder.finish()) as ArrayRef
}

fn u32_list_rows(rows: &[Vec<u32>]) -> ArrayRef {
    let mut builder = ListBuilder::new(UInt32Builder::new());
    for row in rows {
        for value in row {
            builder.values().append_value(*value);
        }
        builder.append(true);
    }
    Arc::new(builder.finish()) as ArrayRef
}

fn raster_array(
    columns: StructArray,
    rows: StructArray,
    values_data: ArrayRef,
    coordinate_space: Option<&str>,
) -> ArrayRef {
    raster_array_with_kind(columns, rows, values_data, coordinate_space, "uniform")
}

fn raster_array_with_kind(
    columns: StructArray,
    rows: StructArray,
    values_data: ArrayRef,
    coordinate_space: Option<&str>,
    kind: &str,
) -> ArrayRef {
    let len = values_data.len();
    let columns = Arc::new(columns) as ArrayRef;
    let rows = Arc::new(rows) as ArrayRef;

    let mut geometry_fields = vec![
        (
            Arc::new(Field::new("kind", DataType::Utf8, false)),
            Arc::new(StringArray::from(vec![kind; len])) as ArrayRef,
        ),
        (
            Arc::new(Field::new("columns", columns.data_type().clone(), false)),
            columns,
        ),
        (
            Arc::new(Field::new("rows", rows.data_type().clone(), false)),
            rows,
        ),
    ];
    if let Some(coordinate_space) = coordinate_space {
        geometry_fields.push((
            Arc::new(Field::new("coordinate_space", DataType::Utf8, true)),
            Arc::new(StringArray::from(vec![Some(coordinate_space); len])) as ArrayRef,
        ));
    }

    let geometry = Arc::new(StructArray::from(geometry_fields)) as ArrayRef;
    let values = Arc::new(StructArray::from(vec![(
        Arc::new(Field::new("data", values_data.data_type().clone(), false)),
        values_data,
    )])) as ArrayRef;

    Arc::new(StructArray::from(vec![
        (
            Arc::new(Field::new("geometry", geometry.data_type().clone(), false)),
            geometry,
        ),
        (
            Arc::new(Field::new("values", values.data_type().clone(), false)),
            values,
        ),
    ])) as ArrayRef
}

fn f64_raster_batch(rasters: Vec<F64Raster>, alpha: Option<Vec<f64>>) -> RecordBatch {
    let columns = axis_array(rasters.iter().map(|raster| raster.columns));
    let rows = axis_array(rasters.iter().map(|raster| raster.rows));
    let values = f64_list_rows(
        &rasters
            .iter()
            .map(|raster| raster.values.clone())
            .collect::<Vec<_>>(),
    );
    let raster = raster_array(columns, rows, values, None);
    let mut columns = vec![("raster", raster)];
    if let Some(alpha) = alpha {
        columns.push(("alpha", Arc::new(Float64Array::from(alpha)) as ArrayRef));
    }
    RecordBatch::try_from_iter(columns).expect("raster batch")
}

fn f64_raster_batch_with_coordinate_space(coordinate_space: &str) -> RecordBatch {
    let raster = F64Raster {
        columns: UniformAxis::new("x", 0.0, 2.0, 2),
        rows: UniformAxis::new("y", 0.0, 2.0, 2),
        values: vec![Some(0.0), Some(1.0), Some(2.0), Some(3.0)],
    };
    let columns = axis_array([raster.columns]);
    let rows = axis_array([raster.rows]);
    let values = f64_list_rows(&[raster.values]);
    let raster = raster_array(columns, rows, values, Some(coordinate_space));
    RecordBatch::try_from_iter(vec![("raster", raster)]).expect("raster batch")
}

fn f64_raster_batch_with_kind(kind: &str) -> RecordBatch {
    let raster = F64Raster {
        columns: UniformAxis::new("x", 0.0, 2.0, 2),
        rows: UniformAxis::new("y", 0.0, 2.0, 2),
        values: vec![Some(0.0), Some(1.0), Some(2.0), Some(3.0)],
    };
    let columns = axis_array([raster.columns]);
    let rows = axis_array([raster.rows]);
    let values = f64_list_rows(&[raster.values]);
    let raster = raster_array_with_kind(columns, rows, values, None, kind);
    RecordBatch::try_from_iter(vec![("raster", raster)]).expect("raster batch")
}

fn f64_raster_batch_with_values(values: Vec<Option<f64>>) -> RecordBatch {
    f64_raster_batch(
        vec![F64Raster {
            columns: UniformAxis::new("x", 0.0, 2.0, 2),
            rows: UniformAxis::new("y", 0.0, 2.0, 2),
            values,
        }],
        None,
    )
}

fn direct_color_raster_batch() -> RecordBatch {
    let raster = raster_array(
        axis_array([UniformAxis::new("x", 0.0, 2.0, 2)]),
        axis_array([UniformAxis::new("y", 0.0, 2.0, 2)]),
        string_list_rows(&[vec!["#ff0000", "#00ff00", "#0000ff", "#ffffff"]]),
        None,
    );
    RecordBatch::try_from_iter(vec![("raster", raster)]).expect("raster batch")
}

fn u32_raster_batch() -> RecordBatch {
    let raster = raster_array(
        axis_array([UniformAxis::new("x", 0.0, 2.0, 2)]),
        axis_array([UniformAxis::new("y", 0.0, 2.0, 2)]),
        u32_list_rows(&[vec![0, 1, 2, 3]]),
        None,
    );
    RecordBatch::try_from_iter(vec![("raster", raster)]).expect("raster batch")
}

fn dataframe(ctx: &SessionContext, batch: RecordBatch) -> DataFrame {
    ctx.read_batch(batch).expect("dataframe")
}

fn assert_domain_close(
    scales: &std::collections::HashMap<String, avenger_chart::scales::ConfiguredScaleWithSpec>,
    name: &str,
    expected: (f32, f32),
) {
    let actual = scales
        .get(name)
        .unwrap_or_else(|| panic!("missing scale {name}"))
        .configured()
        .numeric_interval_domain()
        .unwrap_or_else(|err| panic!("domain for {name}: {err}"));
    assert!(
        (actual.0 - expected.0).abs() < 1e-4 && (actual.1 - expected.1).abs() < 1e-4,
        "{name} domain: expected {expected:?}, got {actual:?}"
    );
}

fn collect_images<'a>(mark: &'a SceneMark, images: &mut Vec<&'a SceneImageMark>) {
    match mark {
        SceneMark::Image(image) => images.push(image),
        SceneMark::Group(group) => {
            for mark in &group.marks {
                collect_images(mark, images);
            }
        }
        _ => {}
    }
}

fn evaluated_images(evaluated: &avenger_chart::render::EvaluatedPlot) -> Vec<&SceneImageMark> {
    let mut images = Vec::new();
    for mark in &evaluated.scene_graph.marks {
        collect_images(mark, &mut images);
    }
    images
}

fn default_params() -> IndexMap<String, ScalarValue> {
    IndexMap::new()
}

async fn raster_evaluate_error(batch: RecordBatch) -> AvengerChartError {
    let ctx = SessionContext::new();
    let df = dataframe(&ctx, batch);
    let plot = Plot::<Cartesian>::new().data(df).mark(
        UniformRaster2D::new().raster_with(col("raster"), |r| r.fill(|fill| fill.no_scale())),
    );
    match plot
        .compile(&ctx)
        .await
        .expect("compile plot")
        .evaluate(&ctx, None)
        .await
    {
        Ok(_) => panic!("raster plot should error"),
        Err(err) => err,
    }
}

#[tokio::test]
async fn scaled_uniform_raster_infers_list_fill_and_geometry_domains() {
    let ctx = SessionContext::new();
    let batch = f64_raster_batch(
        vec![F64Raster {
            columns: UniformAxis::new("x", 0.0, 2.0, 2),
            rows: UniformAxis::new("y", 0.0, 2.0, 2),
            values: vec![Some(0.0), Some(1.0), Some(2.0), Some(3.0)],
        }],
        None,
    );
    let df = dataframe(&ctx, batch);
    let plot = Plot::<Cartesian>::new()
        .plot_size(200.0, 120.0)
        .data(df.clone())
        .mark(UniformRaster2D::new().raster(col("raster")).smooth(false));
    let compiled = plot.compile(&ctx).await.expect("compile plot");

    let scales = compiled
        .build_scales_for_dataframe(&df, 200.0, 120.0, &ctx, &default_params())
        .await
        .expect("build scales");
    assert_domain_close(&scales, "x", (0.0, 2.0));
    assert_domain_close(&scales, "y", (0.0, 2.0));
    assert_domain_close(&scales, "fill", (0.0, 3.0));

    let evaluated = compiled.evaluate(&ctx, None).await.expect("evaluate plot");
    let images = evaluated_images(&evaluated);
    assert_eq!(images.len(), 1);
    let image = images[0]
        .image
        .first()
        .and_then(|source| source.inline_image())
        .expect("inline image");
    assert_eq!((image.width, image.height), (2, 2));
    assert!(!images[0].smooth);
}

#[tokio::test]
async fn explicit_fill_domain_overrides_flattened_list_inference() {
    let ctx = SessionContext::new();
    let batch = f64_raster_batch(
        vec![F64Raster {
            columns: UniformAxis::new("x", 0.0, 2.0, 2),
            rows: UniformAxis::new("y", 0.0, 2.0, 2),
            values: vec![Some(10.0), Some(20.0), Some(30.0), Some(40.0)],
        }],
        None,
    );
    let df = dataframe(&ctx, batch);
    let plot = Plot::<Cartesian>::new()
        .data(df.clone())
        .mark(UniformRaster2D::new().raster_with(col("raster"), |r| {
            r.fill(|fill| {
                fill.scale_with::<Linear>(|scale| {
                    scale.domain((0.0, 100.0)).nice(false).zero(false)
                })
                .legend(|legend| legend.title("Intensity"))
            })
        }));
    let compiled = plot.compile(&ctx).await.expect("compile plot");
    assert!(compiled.scale_specs().contains_key("fill"));
    assert!(compiled.legends().contains_key("fill"));

    let scales = compiled
        .build_scales_for_dataframe(&df, 200.0, 120.0, &ctx, &default_params())
        .await
        .expect("build scales");
    assert_domain_close(&scales, "fill", (0.0, 100.0));
}

#[tokio::test]
async fn direct_color_uniform_raster_does_not_build_fill_scale() {
    let ctx = SessionContext::new();
    let df = dataframe(&ctx, direct_color_raster_batch());
    let plot = Plot::<Cartesian>::new().data(df.clone()).mark(
        UniformRaster2D::new().raster_with(col("raster"), |r| r.fill(|fill| fill.no_scale())),
    );
    let compiled = plot.compile(&ctx).await.expect("compile plot");
    assert!(!compiled.scale_specs().contains_key("fill"));

    let scales = compiled
        .build_scales_for_dataframe(&df, 200.0, 120.0, &ctx, &default_params())
        .await
        .expect("build scales");
    assert!(!scales.contains_key("fill"));

    let evaluated = compiled.evaluate(&ctx, None).await.expect("evaluate plot");
    let images = evaluated_images(&evaluated);
    assert_eq!(images.len(), 1);
    let image = images[0]
        .image
        .first()
        .and_then(|source| source.inline_image())
        .expect("inline image");
    assert_eq!(
        image.data,
        vec![
            0, 0, 255, 255, 255, 255, 255, 255, 255, 0, 0, 255, 0, 255, 0, 255,
        ]
    );
}

#[tokio::test]
async fn integer_value_plane_renders_through_scaled_fill_path() {
    let ctx = SessionContext::new();
    let df = dataframe(&ctx, u32_raster_batch());
    let plot = Plot::<Cartesian>::new()
        .data(df.clone())
        .mark(UniformRaster2D::new().raster(col("raster")));
    let compiled = plot.compile(&ctx).await.expect("compile plot");

    let scales = compiled
        .build_scales_for_dataframe(&df, 200.0, 120.0, &ctx, &default_params())
        .await
        .expect("build scales");
    assert_domain_close(&scales, "fill", (0.0, 3.0));

    let evaluated = compiled.evaluate(&ctx, None).await.expect("evaluate plot");
    let images = evaluated_images(&evaluated);
    assert_eq!(images.len(), 1);
    let image = images[0]
        .image
        .first()
        .and_then(|source| source.inline_image())
        .expect("inline image");
    assert_eq!((image.width, image.height), (2, 2));
}

#[tokio::test]
async fn multiple_raster_rows_union_domains_and_render_multiple_images() {
    let ctx = SessionContext::new();
    let batch = f64_raster_batch(
        vec![
            F64Raster {
                columns: UniformAxis::new("x", 0.0, 2.0, 2),
                rows: UniformAxis::new("y", 0.0, 2.0, 2),
                values: vec![Some(0.0), Some(1.0), Some(2.0), Some(3.0)],
            },
            F64Raster {
                columns: UniformAxis::new("x", 5.0, 9.0, 2),
                rows: UniformAxis::new("y", -1.0, 3.0, 2),
                values: vec![Some(10.0), Some(11.0), Some(12.0), Some(13.0)],
            },
        ],
        Some(vec![1.0, 0.5]),
    );
    let df = dataframe(&ctx, batch);
    let plot = Plot::<Cartesian>::new().data(df.clone()).mark(
        UniformRaster2D::new()
            .raster(col("raster"))
            .opacity_with(col("alpha"), |opacity| opacity.no_scale()),
    );
    let compiled = plot.compile(&ctx).await.expect("compile plot");

    let scales = compiled
        .build_scales_for_dataframe(&df, 200.0, 120.0, &ctx, &default_params())
        .await
        .expect("build scales");
    assert_domain_close(&scales, "x", (0.0, 9.0));
    assert_domain_close(&scales, "y", (-1.0, 3.0));
    assert_domain_close(&scales, "fill", (0.0, 13.0));

    let evaluated = compiled.evaluate(&ctx, None).await.expect("evaluate plot");
    let images = evaluated_images(&evaluated);
    assert_eq!(images.len(), 2);
    let first = images[0]
        .image
        .first()
        .and_then(|source| source.inline_image())
        .expect("first inline image");
    let second = images[1]
        .image
        .first()
        .and_then(|source| source.inline_image())
        .expect("second inline image");
    assert!(first.data.chunks_exact(4).all(|rgba| rgba[3] == 255));
    assert!(second.data.chunks_exact(4).all(|rgba| rgba[3] == 128));
}

#[tokio::test]
async fn non_null_coordinate_space_errors_in_phase_1() {
    let err = raster_evaluate_error(f64_raster_batch_with_coordinate_space("EPSG:3857")).await;
    assert!(
        err.to_string().contains("CRS-backed rasters require"),
        "{err}"
    );
}

#[tokio::test]
async fn invalid_uniform_raster_schema_errors_are_clear() {
    let err = raster_evaluate_error(f64_raster_batch_with_kind("quadmesh")).await;
    assert!(
        err.to_string()
            .contains("requires geometry.kind = 'uniform'"),
        "{err}"
    );

    let err = raster_evaluate_error(f64_raster_batch(
        vec![F64Raster {
            columns: UniformAxis::new("x", 0.0, 2.0, 0),
            rows: UniformAxis::new("y", 0.0, 2.0, 2),
            values: Vec::new(),
        }],
        None,
    ))
    .await;
    assert!(
        err.to_string()
            .contains("rows.count and columns.count must be greater than zero"),
        "{err}"
    );

    let err = raster_evaluate_error(f64_raster_batch_with_values(vec![
        Some(0.0),
        Some(1.0),
        Some(2.0),
    ]))
    .await;
    assert!(
        err.to_string()
            .contains("rows.count * columns.count requires 4"),
        "{err}"
    );
}
