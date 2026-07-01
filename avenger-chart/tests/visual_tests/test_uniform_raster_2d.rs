use std::{collections::BTreeMap, sync::Arc};

use avenger_chart::prelude::*;
use datafusion::{
    arrow::{
        array::{
            Array, ArrayRef, Float64Array, Float64Builder, ListBuilder, StringArray, StringBuilder,
            StructArray, UInt32Array,
        },
        datatypes::{DataType, Field},
        record_batch::RecordBatch,
    },
    common::ScalarValue,
    prelude::{ParquetReadOptions, SessionContext, col},
};

use super::helpers::assert_visual_match;

#[derive(Clone, Copy)]
struct UniformAxis {
    coord: &'static str,
    start: f64,
    stop: f64,
    count: u32,
}

impl UniformAxis {
    fn new(coord: &'static str, start: f64, stop: f64, count: u32) -> Self {
        Self {
            coord,
            start,
            stop,
            count,
        }
    }
}

struct RasterRow {
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
            Arc::new(StringArray::from(vec![None::<&str>; axes.len()])) as ArrayRef,
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

fn raster_array(columns: StructArray, rows: StructArray, values_data: ArrayRef) -> ArrayRef {
    let len = values_data.len();
    let columns = Arc::new(columns) as ArrayRef;
    let rows = Arc::new(rows) as ArrayRef;
    let geometry = Arc::new(StructArray::from(vec![
        (
            Arc::new(Field::new("kind", DataType::Utf8, false)),
            Arc::new(StringArray::from(vec!["uniform"; len])) as ArrayRef,
        ),
        (
            Arc::new(Field::new("columns", columns.data_type().clone(), false)),
            columns,
        ),
        (
            Arc::new(Field::new("rows", rows.data_type().clone(), false)),
            rows,
        ),
    ])) as ArrayRef;
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

fn raster_batch(rows: Vec<RasterRow>) -> RecordBatch {
    let raster = raster_array(
        axis_array(rows.iter().map(|row| row.columns)),
        axis_array(rows.iter().map(|row| row.rows)),
        f64_list_rows(
            &rows
                .iter()
                .map(|row| row.values.clone())
                .collect::<Vec<_>>(),
        ),
    );
    RecordBatch::try_from_iter(vec![("raster", raster)]).expect("raster batch")
}

fn direct_color_batch() -> RecordBatch {
    let raster = raster_array(
        axis_array([UniformAxis::new("x", 0.0, 3.0, 3)]),
        axis_array([UniformAxis::new("y", 0.0, 2.0, 2)]),
        string_list_rows(&[vec![
            "#d7191c",
            "#fdae61",
            "#ffffbf",
            "#abd9e9",
            "#2c7bb6",
            "#00000080",
        ]]),
    );
    RecordBatch::try_from_iter(vec![("raster", raster)]).expect("raster batch")
}

fn multiple_rows_batch() -> RecordBatch {
    let raster = raster_array(
        axis_array([
            UniformAxis::new("x", 0.0, 3.0, 3),
            UniformAxis::new("x", 1.0, 4.0, 3),
        ]),
        axis_array([
            UniformAxis::new("y", 0.0, 2.0, 2),
            UniformAxis::new("y", 1.0, 3.0, 2),
        ]),
        f64_list_rows(&[
            vec![
                Some(0.0),
                Some(1.0),
                Some(2.0),
                Some(3.0),
                Some(4.0),
                Some(5.0),
            ],
            vec![
                Some(5.0),
                Some(4.0),
                Some(3.0),
                Some(2.0),
                Some(1.0),
                Some(0.0),
            ],
        ]),
    );
    RecordBatch::try_from_iter(vec![
        ("raster", raster),
        (
            "alpha",
            Arc::new(Float64Array::from(vec![1.0, 0.45])) as ArrayRef,
        ),
    ])
    .expect("raster batch")
}

fn scalar_to_f64(value: ScalarValue, label: &str) -> f64 {
    match value {
        ScalarValue::Float64(Some(value)) => value,
        ScalarValue::Float32(Some(value)) => value as f64,
        ScalarValue::Int64(Some(value)) => value as f64,
        ScalarValue::Int32(Some(value)) => value as f64,
        ScalarValue::UInt64(Some(value)) => value as f64,
        ScalarValue::UInt32(Some(value)) => value as f64,
        other => panic!("unexpected {label} scalar: {other:?}"),
    }
}

async fn cars_density_batch(ctx: &SessionContext) -> (RecordBatch, f64) {
    const X0: f64 = 1500.0;
    const X1: f64 = 5200.0;
    const NX: usize = 48;
    const Y0: f64 = 8.0;
    const Y1: f64 = 48.0;
    const NY: usize = 32;

    let cars_path = format!("{}/tests/data/cars.parquet", env!("CARGO_MANIFEST_DIR"));
    let cars = ctx
        .read_parquet(cars_path, ParquetReadOptions::default())
        .await
        .expect("load cars dataset")
        .collect()
        .await
        .expect("collect cars");

    let mut by_origin: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for batch in cars {
        let origin_array = batch.column_by_name("Origin").expect("Origin column");
        let weight_array = batch
            .column_by_name("Weight_in_lbs")
            .expect("Weight_in_lbs column");
        let mpg_array = batch
            .column_by_name("Miles_per_Gallon")
            .expect("Miles_per_Gallon column");
        for row in 0..batch.num_rows() {
            if origin_array.is_null(row) || weight_array.is_null(row) || mpg_array.is_null(row) {
                continue;
            }
            let origin =
                match ScalarValue::try_from_array(origin_array, row).expect("origin scalar") {
                    ScalarValue::Utf8(Some(value)) | ScalarValue::LargeUtf8(Some(value)) => value,
                    ScalarValue::Utf8View(Some(value)) => value,
                    other => panic!("unexpected Origin scalar: {other:?}"),
                };
            let weight = ScalarValue::try_from_array(weight_array, row)
                .map(|scalar| scalar_to_f64(scalar, "weight"))
                .expect("weight scalar");
            let mpg = ScalarValue::try_from_array(mpg_array, row)
                .map(|scalar| scalar_to_f64(scalar, "mpg"))
                .expect("mpg scalar");
            if !(X0..=X1).contains(&weight) || !(Y0..=Y1).contains(&mpg) {
                continue;
            }
            let x_bin = if weight == X1 {
                NX - 1
            } else {
                (((weight - X0) / (X1 - X0)) * NX as f64).floor() as usize
            };
            let y_bin = if mpg == Y1 {
                NY - 1
            } else {
                (((mpg - Y0) / (Y1 - Y0)) * NY as f64).floor() as usize
            };
            by_origin
                .entry(origin)
                .or_insert_with(|| vec![0.0; NX * NY])[y_bin * NX + x_bin] += 1.0;
        }
    }

    let max_bin_count = by_origin
        .values()
        .flat_map(|values| values.iter().copied())
        .fold(0.0_f64, f64::max);
    let origins = by_origin.keys().cloned().collect::<Vec<_>>();
    let rows = by_origin
        .into_values()
        .map(|values| RasterRow {
            columns: UniformAxis::new("Weight_in_lbs", X0, X1, NX as u32),
            rows: UniformAxis::new("Miles_per_Gallon", Y0, Y1, NY as u32),
            values: values.into_iter().map(Some).collect(),
        })
        .collect::<Vec<_>>();
    let raster = raster_array(
        axis_array(rows.iter().map(|row| row.columns)),
        axis_array(rows.iter().map(|row| row.rows)),
        f64_list_rows(
            &rows
                .iter()
                .map(|row| row.values.clone())
                .collect::<Vec<_>>(),
        ),
    );
    let batch = RecordBatch::try_from_iter(vec![
        ("raster", raster),
        ("origin", Arc::new(StringArray::from(origins)) as ArrayRef),
    ])
    .expect("cars density batch");
    (batch, max_bin_count)
}

#[tokio::test]
async fn uniform_raster_2d_scaled_inferred_domain() {
    let ctx = SessionContext::new();
    let df = ctx
        .read_batch(raster_batch(vec![RasterRow {
            columns: UniformAxis::new("x", 0.0, 4.0, 4),
            rows: UniformAxis::new("y", 0.0, 3.0, 3),
            values: (0..12).map(|value| Some(value as f64)).collect(),
        }]))
        .unwrap();
    let plot = Plot::<Cartesian>::new()
        .plot_size(240.0, 180.0)
        .data(df)
        .mark(UniformRaster2D::new().raster(col("raster")).smooth(false));

    let compiled = plot.compile(&ctx).await.unwrap();
    assert_visual_match(
        &compiled,
        &ctx,
        None,
        "uniform_raster_2d",
        "scaled_inferred_domain",
        0.9999,
    )
    .await;
}

#[tokio::test]
async fn uniform_raster_2d_scaled_explicit_domain() {
    let ctx = SessionContext::new();
    let df = ctx
        .read_batch(raster_batch(vec![RasterRow {
            columns: UniformAxis::new("x", 0.0, 4.0, 4),
            rows: UniformAxis::new("y", 0.0, 3.0, 3),
            values: (0..12).map(|value| Some(value as f64)).collect(),
        }]))
        .unwrap();
    let plot = Plot::<Cartesian>::new()
        .plot_size(240.0, 180.0)
        .data(df)
        .mark(
            UniformRaster2D::new()
                .raster_with(col("raster"), |r| {
                    r.fill(|fill| {
                        fill.scale_with::<Linear>(|scale| {
                            scale.domain((0.0, 100.0)).nice(false).zero(false)
                        })
                        .legend(|legend| legend.title("Intensity"))
                    })
                })
                .smooth(false),
        );

    let compiled = plot.compile(&ctx).await.unwrap();
    assert_visual_match(
        &compiled,
        &ctx,
        None,
        "uniform_raster_2d",
        "scaled_explicit_domain",
        0.9999,
    )
    .await;
}

#[tokio::test]
async fn uniform_raster_2d_unscaled_direct_colors() {
    let ctx = SessionContext::new();
    let df = ctx.read_batch(direct_color_batch()).unwrap();
    let plot = Plot::<Cartesian>::new()
        .plot_size(220.0, 150.0)
        .data(df)
        .mark(
            UniformRaster2D::new().raster_with(col("raster"), |r| r.fill(|fill| fill.no_scale())),
        );

    let compiled = plot.compile(&ctx).await.unwrap();
    assert_visual_match(
        &compiled,
        &ctx,
        None,
        "uniform_raster_2d",
        "unscaled_direct_colors",
        0.9999,
    )
    .await;
}

#[tokio::test]
async fn uniform_raster_2d_multiple_rows_single_plot() {
    let ctx = SessionContext::new();
    let df = ctx.read_batch(multiple_rows_batch()).unwrap();
    let plot = Plot::<Cartesian>::new()
        .plot_size(240.0, 180.0)
        .data(df)
        .mark(
            UniformRaster2D::new()
                .raster(col("raster"))
                .opacity_with(col("alpha"), |opacity| opacity.no_scale())
                .smooth(false),
        );

    let compiled = plot.compile(&ctx).await.unwrap();
    assert_visual_match(
        &compiled,
        &ctx,
        None,
        "uniform_raster_2d",
        "multiple_rows_single_plot",
        0.9999,
    )
    .await;
}

#[tokio::test]
async fn uniform_raster_2d_transposed_axis_binding() {
    let ctx = SessionContext::new();
    let df = ctx
        .read_batch(raster_batch(vec![RasterRow {
            columns: UniformAxis::new("column_coord", 0.0, 2.0, 2),
            rows: UniformAxis::new("row_coord", 0.0, 3.0, 3),
            values: vec![
                Some(0.0),
                Some(5.0),
                Some(2.0),
                Some(8.0),
                Some(4.0),
                Some(10.0),
            ],
        }]))
        .unwrap();
    let plot = Plot::<Cartesian>::new()
        .plot_size(240.0, 160.0)
        .data(df)
        .mark(
            UniformRaster2D::new()
                .raster(col("raster"))
                .transpose()
                .smooth(false),
        );

    let compiled = plot.compile(&ctx).await.unwrap();
    assert_visual_match(
        &compiled,
        &ctx,
        None,
        "uniform_raster_2d",
        "transposed_axis_binding",
        0.9999,
    )
    .await;
}

#[tokio::test]
async fn uniform_raster_2d_dataset_cars_density_by_origin() {
    let ctx = SessionContext::new();
    let (batch, max_bin_count) = cars_density_batch(&ctx).await;
    let df = ctx.read_batch(batch).unwrap();
    let leaf = Plot::<Cartesian>::new().mark(
        UniformRaster2D::new()
            .raster_with(col("raster"), move |r| {
                r.fill(move |fill| {
                    fill.scale_with::<Sqrt>(move |scale| {
                        scale.domain((0.0, max_bin_count)).nice(false).zero(false)
                    })
                    .legend(|legend| legend.title("Cars"))
                })
                .x(|x| x.axis(|axis| axis.title("Weight")))
                .y(|y| y.axis(|axis| axis.title("MPG")))
            })
            .smooth(false),
    );
    let plot = Plot::<FacetColumn>::new()
        .data(df)
        .plot_size(180.0, 140.0)
        .mark(Subplot::new(leaf).column(col("origin")));

    let compiled = plot.compile(&ctx).await.unwrap();
    assert_visual_match(
        &compiled,
        &ctx,
        None,
        "uniform_raster_2d",
        "dataset_cars_density_by_origin",
        0.9999,
    )
    .await;
}
