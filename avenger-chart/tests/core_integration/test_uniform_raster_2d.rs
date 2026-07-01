use std::sync::Arc;

use avenger_chart::prelude::*;
use avenger_scenegraph::marks::{image::SceneImageMark, mark::SceneMark};
use datafusion::{
    arrow::{
        array::{
            ArrayRef, Float64Array, Float64Builder, ListArray, ListBuilder, StringArray,
            StringBuilder, StructArray, UInt32Array, UInt32Builder,
        },
        buffer::OffsetBuffer,
        datatypes::{DataType, Field},
        record_batch::RecordBatch,
    },
    common::ScalarValue,
    dataframe::DataFrame,
    prelude::{SessionContext, col, lit},
};
use indexmap::IndexMap;

#[derive(Clone)]
enum RasterDimension {
    Uniform {
        name: &'static str,
        start: f64,
        stop: f64,
        count: u32,
        sampling: Option<&'static str>,
    },
    Categorical {
        name: &'static str,
        values: Vec<&'static str>,
    },
}

impl RasterDimension {
    fn uniform(name: &'static str, start: f64, stop: f64, count: u32) -> Self {
        Self::Uniform {
            name,
            start,
            stop,
            count,
            sampling: None,
        }
    }

    fn categorical(name: &'static str, values: Vec<&'static str>) -> Self {
        Self::Categorical { name, values }
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Uniform { name, .. } | Self::Categorical { name, .. } => name,
        }
    }
}

struct RasterRow<T> {
    dimensions: Vec<RasterDimension>,
    values_dims: Vec<&'static str>,
    values: Vec<Option<T>>,
}

fn uniform_raster<T>(
    x_start: f64,
    x_stop: f64,
    x_count: u32,
    y_start: f64,
    y_stop: f64,
    y_count: u32,
    values: Vec<Option<T>>,
) -> RasterRow<T> {
    RasterRow {
        dimensions: vec![
            RasterDimension::uniform("x", x_start, x_stop, x_count),
            RasterDimension::uniform("y", y_start, y_stop, y_count),
        ],
        values_dims: vec!["y", "x"],
        values,
    }
}

fn list_array_from_rows(values: ArrayRef, lengths: impl IntoIterator<Item = usize>) -> ArrayRef {
    let offsets = OffsetBuffer::from_lengths(lengths);
    Arc::new(
        ListArray::try_new(
            Arc::new(Field::new_list_field(values.data_type().clone(), true)),
            offsets,
            values,
            None,
        )
        .expect("list array"),
    ) as ArrayRef
}

fn dimensions_list_rows(rows: &[Vec<RasterDimension>]) -> ArrayRef {
    let dimensions = rows
        .iter()
        .flat_map(|row| row.iter().cloned())
        .collect::<Vec<_>>();
    let names = dimensions
        .iter()
        .map(RasterDimension::name)
        .collect::<Vec<_>>();
    let kinds = dimensions
        .iter()
        .map(|dimension| match dimension {
            RasterDimension::Uniform { .. } => "uniform",
            RasterDimension::Categorical { .. } => "categorical",
        })
        .collect::<Vec<_>>();
    let samplings = dimensions
        .iter()
        .map(|dimension| match dimension {
            RasterDimension::Uniform { sampling, .. } => *sampling,
            RasterDimension::Categorical { .. } => None,
        })
        .collect::<Vec<_>>();
    let starts = dimensions
        .iter()
        .map(|dimension| match dimension {
            RasterDimension::Uniform { start, .. } => Some(*start),
            RasterDimension::Categorical { .. } => None,
        })
        .collect::<Vec<_>>();
    let stops = dimensions
        .iter()
        .map(|dimension| match dimension {
            RasterDimension::Uniform { stop, .. } => Some(*stop),
            RasterDimension::Categorical { .. } => None,
        })
        .collect::<Vec<_>>();
    let counts = dimensions
        .iter()
        .map(|dimension| match dimension {
            RasterDimension::Uniform { count, .. } => Some(*count),
            RasterDimension::Categorical { .. } => None,
        })
        .collect::<Vec<_>>();
    let mut values_builder = ListBuilder::new(StringBuilder::new());
    for dimension in &dimensions {
        match dimension {
            RasterDimension::Uniform { .. } => values_builder.append(false),
            RasterDimension::Categorical { values, .. } => {
                for value in values {
                    values_builder.values().append_value(*value);
                }
                values_builder.append(true);
            }
        }
    }
    let coord_values = Arc::new(values_builder.finish()) as ArrayRef;
    let coords = Arc::new(StructArray::from(vec![
        (
            Arc::new(Field::new("kind", DataType::Utf8, false)),
            Arc::new(StringArray::from(kinds)) as ArrayRef,
        ),
        (
            Arc::new(Field::new("sampling", DataType::Utf8, true)),
            Arc::new(StringArray::from(samplings)) as ArrayRef,
        ),
        (
            Arc::new(Field::new("start", DataType::Float64, true)),
            Arc::new(Float64Array::from(starts)) as ArrayRef,
        ),
        (
            Arc::new(Field::new("stop", DataType::Float64, true)),
            Arc::new(Float64Array::from(stops)) as ArrayRef,
        ),
        (
            Arc::new(Field::new("count", DataType::UInt32, true)),
            Arc::new(UInt32Array::from(counts)) as ArrayRef,
        ),
        (
            Arc::new(Field::new("values", coord_values.data_type().clone(), true)),
            coord_values,
        ),
    ])) as ArrayRef;
    let dimension_values = Arc::new(StructArray::from(vec![
        (
            Arc::new(Field::new("name", DataType::Utf8, false)),
            Arc::new(StringArray::from(names)) as ArrayRef,
        ),
        (
            Arc::new(Field::new("coords", coords.data_type().clone(), false)),
            coords,
        ),
    ])) as ArrayRef;
    list_array_from_rows(dimension_values, rows.iter().map(Vec::len))
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

fn u32_list_rows(rows: &[Vec<Option<u32>>]) -> ArrayRef {
    let mut builder = ListBuilder::new(UInt32Builder::new());
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

fn raster_array<T>(rows: &[RasterRow<T>], values_data: ArrayRef, kind: &str) -> ArrayRef {
    let dimensions = dimensions_list_rows(
        &rows
            .iter()
            .map(|row| row.dimensions.clone())
            .collect::<Vec<_>>(),
    );
    let values_dims = string_list_rows(
        &rows
            .iter()
            .map(|row| row.values_dims.clone())
            .collect::<Vec<_>>(),
    );
    let len = values_data.len();
    let geometry = Arc::new(StructArray::from(vec![
        (
            Arc::new(Field::new("kind", DataType::Utf8, false)),
            Arc::new(StringArray::from(vec![kind; len])) as ArrayRef,
        ),
        (
            Arc::new(Field::new(
                "dimensions",
                dimensions.data_type().clone(),
                false,
            )),
            dimensions,
        ),
    ])) as ArrayRef;
    let values = Arc::new(StructArray::from(vec![
        (
            Arc::new(Field::new("dims", values_dims.data_type().clone(), false)),
            values_dims,
        ),
        (
            Arc::new(Field::new("data", values_data.data_type().clone(), false)),
            values_data,
        ),
    ])) as ArrayRef;

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

fn f64_raster_batch(rasters: Vec<RasterRow<f64>>, alpha: Option<Vec<f64>>) -> RecordBatch {
    let values = f64_list_rows(
        &rasters
            .iter()
            .map(|raster| raster.values.clone())
            .collect::<Vec<_>>(),
    );
    let raster = raster_array(&rasters, values, "grid");
    let mut columns = vec![("raster", raster)];
    if let Some(alpha) = alpha {
        columns.push(("alpha", Arc::new(Float64Array::from(alpha)) as ArrayRef));
    }
    RecordBatch::try_from_iter(columns).expect("raster batch")
}

fn f64_raster_batch_with_kind(kind: &str) -> RecordBatch {
    let rasters = vec![uniform_raster(
        0.0,
        2.0,
        2,
        0.0,
        2.0,
        2,
        vec![Some(0.0), Some(1.0), Some(2.0), Some(3.0)],
    )];
    let values = f64_list_rows(&[rasters[0].values.clone()]);
    let raster = raster_array(&rasters, values, kind);
    RecordBatch::try_from_iter(vec![("raster", raster)]).expect("raster batch")
}

fn f64_raster_batch_with_values(values: Vec<Option<f64>>) -> RecordBatch {
    f64_raster_batch(vec![uniform_raster(0.0, 2.0, 2, 0.0, 2.0, 2, values)], None)
}

fn direct_color_raster_batch() -> RecordBatch {
    let rasters = vec![RasterRow {
        dimensions: vec![
            RasterDimension::uniform("x", 0.0, 2.0, 2),
            RasterDimension::uniform("y", 0.0, 2.0, 2),
        ],
        values_dims: vec!["y", "x"],
        values: vec![
            Some("#ff0000"),
            Some("#00ff00"),
            Some("#0000ff"),
            Some("#ffffff"),
        ],
    }];
    let values = string_list_rows(&[rasters[0].values.iter().map(|v| v.unwrap()).collect()]);
    let raster = raster_array(&rasters, values, "grid");
    RecordBatch::try_from_iter(vec![("raster", raster)]).expect("raster batch")
}

fn string_value_raster_batch() -> RecordBatch {
    let rasters = vec![RasterRow {
        dimensions: vec![
            RasterDimension::uniform("x", 0.0, 2.0, 2),
            RasterDimension::uniform("y", 0.0, 2.0, 2),
        ],
        values_dims: vec!["y", "x"],
        values: vec![Some("b"), Some("a"), Some("a"), Some("b")],
    }];
    let values = string_list_rows(&[rasters[0].values.iter().map(|v| v.unwrap()).collect()]);
    let raster = raster_array(&rasters, values, "grid");
    RecordBatch::try_from_iter(vec![("raster", raster)]).expect("raster batch")
}

fn u32_raster_batch() -> RecordBatch {
    let rasters = vec![uniform_raster(
        0.0,
        2.0,
        2,
        0.0,
        2.0,
        2,
        vec![Some(0), Some(1), Some(2), Some(3)],
    )];
    let values = u32_list_rows(&[rasters[0].values.clone()]);
    let raster = raster_array(&rasters, values, "grid");
    RecordBatch::try_from_iter(vec![("raster", raster)]).expect("raster batch")
}

fn categorical_raster_batch() -> RecordBatch {
    let rasters = vec![RasterRow {
        dimensions: vec![
            RasterDimension::uniform("x", 0.0, 2.0, 2),
            RasterDimension::categorical("group", vec!["A", "B"]),
        ],
        values_dims: vec!["group", "x"],
        values: vec![Some(0.0), Some(1.0), Some(2.0), Some(3.0)],
    }];
    let values = f64_list_rows(&[rasters[0].values.clone()]);
    let raster = raster_array(&rasters, values, "grid");
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

fn assert_string_domain(
    scales: &std::collections::HashMap<String, avenger_chart::scales::ConfiguredScaleWithSpec>,
    name: &str,
    expected: &[&str],
) {
    let scale = scales
        .get(name)
        .unwrap_or_else(|| panic!("missing scale {name}"));
    let values = &scale.configured().config.domain;
    let actual = (0..values.len())
        .map(
            |row| match ScalarValue::try_from_array(values, row).expect("domain scalar") {
                ScalarValue::Utf8(Some(value))
                | ScalarValue::LargeUtf8(Some(value))
                | ScalarValue::Utf8View(Some(value)) => value,
                other => panic!("unexpected domain value: {other:?}"),
            },
        )
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
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
    let plot = Plot::<Cartesian>::new()
        .data(df)
        .mark(UniformRaster2D::new().raster_with(col("raster"), |r| r.x(dim("x")).y(dim("y"))));
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
        vec![uniform_raster(
            0.0,
            2.0,
            2,
            0.0,
            2.0,
            2,
            vec![Some(0.0), Some(1.0), Some(2.0), Some(3.0)],
        )],
        None,
    );
    let df = dataframe(&ctx, batch);
    let plot = Plot::<Cartesian>::new()
        .plot_size(200.0, 120.0)
        .data(df.clone())
        .mark(
            UniformRaster2D::new()
                .raster_with(col("raster"), |r| r.x(dim("x")).y(dim("y")))
                .smooth(false),
        );
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
        vec![uniform_raster(
            0.0,
            2.0,
            2,
            0.0,
            2.0,
            2,
            vec![Some(10.0), Some(20.0), Some(30.0), Some(40.0)],
        )],
        None,
    );
    let df = dataframe(&ctx, batch);
    let plot = Plot::<Cartesian>::new()
        .data(df.clone())
        .mark(UniformRaster2D::new().raster_with(col("raster"), |r| {
            r.x(dim("x")).y(dim("y")).fill(|fill| {
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
async fn categorical_dimension_infers_band_domain_when_scale_is_categorical() {
    let ctx = SessionContext::new();
    let df = dataframe(&ctx, categorical_raster_batch());
    let plot = Plot::<Cartesian>::new()
        .data(df.clone())
        .mark(UniformRaster2D::new().raster_with(col("raster"), |r| {
            r.x(dim("x"))
                .y_with(dim("group"), |y| y.scale_with::<Band>(|scale| scale))
        }));
    let compiled = plot.compile(&ctx).await.expect("compile plot");
    let scales = compiled
        .build_scales_for_dataframe(&df, 200.0, 120.0, &ctx, &default_params())
        .await
        .expect("build scales");
    assert_string_domain(&scales, "y", &["A", "B"]);
    assert_domain_close(&scales, "fill", (0.0, 3.0));

    let evaluated = compiled.evaluate(&ctx, None).await.expect("evaluate plot");
    assert_eq!(evaluated_images(&evaluated).len(), 2);
}

#[tokio::test]
async fn string_value_plane_infers_discrete_fill_domain_and_legend() {
    let ctx = SessionContext::new();
    let df = dataframe(&ctx, string_value_raster_batch());
    let plot = Plot::<Cartesian>::new()
        .data(df.clone())
        .mark(UniformRaster2D::new().raster_with(col("raster"), |r| {
            r.x(dim("x")).y(dim("y")).fill(|fill| {
                fill.scale_with::<Ordinal>(|scale| {
                    scale
                        .domain_discrete(vec![lit("a"), lit("b")])
                        .range_discrete(vec!["#e8f5e9", "#1b5e20"])
                })
                .legend(|legend| legend.title("Band"))
            })
        }));
    let compiled = plot.compile(&ctx).await.expect("compile plot");
    assert!(compiled.scale_specs().contains_key("fill"));
    assert!(compiled.legends().contains_key("fill"));

    let scales = compiled
        .build_scales_for_dataframe(&df, 200.0, 120.0, &ctx, &default_params())
        .await
        .expect("build scales");
    assert_string_domain(&scales, "fill", &["a", "b"]);

    let evaluated = compiled.evaluate(&ctx, None).await.expect("evaluate plot");
    assert_eq!(evaluated_images(&evaluated).len(), 1);
}

#[tokio::test]
async fn direct_color_uniform_raster_does_not_build_fill_scale() {
    let ctx = SessionContext::new();
    let df = dataframe(&ctx, direct_color_raster_batch());
    let plot = Plot::<Cartesian>::new().data(df.clone()).mark(
        UniformRaster2D::new().raster_with(col("raster"), |r| {
            r.x(dim("x")).y(dim("y")).fill(|fill| fill.no_scale())
        }),
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
        .mark(UniformRaster2D::new().raster_with(col("raster"), |r| r.x(dim("x")).y(dim("y"))));
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
            uniform_raster(
                0.0,
                2.0,
                2,
                0.0,
                2.0,
                2,
                vec![Some(0.0), Some(1.0), Some(2.0), Some(3.0)],
            ),
            uniform_raster(
                5.0,
                9.0,
                2,
                -1.0,
                3.0,
                2,
                vec![Some(10.0), Some(11.0), Some(12.0), Some(13.0)],
            ),
        ],
        Some(vec![1.0, 0.5]),
    );
    let df = dataframe(&ctx, batch);
    let plot = Plot::<Cartesian>::new().data(df.clone()).mark(
        UniformRaster2D::new()
            .raster_with(col("raster"), |r| r.x(dim("x")).y(dim("y")))
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
async fn invalid_uniform_raster_schema_errors_are_clear() {
    let err = raster_evaluate_error(f64_raster_batch_with_kind("quadmesh")).await;
    assert!(
        err.to_string().contains("requires geometry.kind = 'grid'"),
        "{err}"
    );

    let err = raster_evaluate_error(f64_raster_batch(
        vec![uniform_raster(0.0, 2.0, 0, 0.0, 2.0, 2, Vec::new())],
        None,
    ))
    .await;
    assert!(
        err.to_string()
            .contains("dimension 'x' coords.count must be greater than zero"),
        "{err}"
    );

    let err = raster_evaluate_error(f64_raster_batch_with_values(vec![
        Some(0.0),
        Some(1.0),
        Some(2.0),
    ]))
    .await;
    assert!(
        err.to_string().contains("values.data row 0 has 3 cells"),
        "{err}"
    );
}
