use std::{collections::BTreeMap, sync::Arc};

use avenger_chart::prelude::*;
use datafusion::{
    arrow::{
        array::{
            Array, ArrayRef, Float64Array, Float64Builder, ListArray, ListBuilder, StringArray,
            StringBuilder, StructArray, UInt32Array,
        },
        buffer::OffsetBuffer,
        datatypes::{DataType, Field},
        record_batch::RecordBatch,
    },
    common::ScalarValue,
    prelude::{CsvReadOptions, ParquetReadOptions, SessionContext, col, lit},
};

use super::helpers::assert_visual_match;

const TAXI_X_MIN: f64 = -8_242_500.0;
const TAXI_X_MAX: f64 = -8_226_500.0;
const TAXI_Y_MIN: f64 = 4_968_000.0;
const TAXI_Y_MAX: f64 = 4_983_000.0;

#[derive(Clone)]
enum RasterDimension {
    Uniform {
        name: &'static str,
        start: f64,
        stop: f64,
        count: u32,
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
    x_name: &'static str,
    x_start: f64,
    x_stop: f64,
    x_count: u32,
    y_name: &'static str,
    y_start: f64,
    y_stop: f64,
    y_count: u32,
    values: Vec<Option<T>>,
) -> RasterRow<T> {
    RasterRow {
        dimensions: vec![
            RasterDimension::uniform(x_name, x_start, x_stop, x_count),
            RasterDimension::uniform(y_name, y_start, y_stop, y_count),
        ],
        values_dims: vec![y_name, x_name],
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
    let mut coord_values_builder = ListBuilder::new(StringBuilder::new());
    for dimension in &dimensions {
        match dimension {
            RasterDimension::Uniform { .. } => coord_values_builder.append(false),
            RasterDimension::Categorical { values, .. } => {
                for value in values {
                    coord_values_builder.values().append_value(*value);
                }
                coord_values_builder.append(true);
            }
        }
    }
    let coord_values = Arc::new(coord_values_builder.finish()) as ArrayRef;
    let coords = Arc::new(StructArray::from(vec![
        (
            Arc::new(Field::new("kind", DataType::Utf8, false)),
            Arc::new(StringArray::from(kinds)) as ArrayRef,
        ),
        (
            Arc::new(Field::new("sampling", DataType::Utf8, true)),
            Arc::new(StringArray::from(vec![Some("linear"); dimensions.len()])) as ArrayRef,
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

fn raster_array<T>(rows: &[RasterRow<T>], values_data: ArrayRef) -> ArrayRef {
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
            Arc::new(StringArray::from(vec!["grid"; len])) as ArrayRef,
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

fn raster_batch(rows: Vec<RasterRow<f64>>) -> RecordBatch {
    let values = f64_list_rows(
        &rows
            .iter()
            .map(|row| row.values.clone())
            .collect::<Vec<_>>(),
    );
    let raster = raster_array(&rows, values);
    RecordBatch::try_from_iter(vec![("raster", raster)]).expect("raster batch")
}

fn direct_color_batch() -> RecordBatch {
    let rows = vec![RasterRow {
        dimensions: vec![
            RasterDimension::uniform("x", 0.0, 3.0, 3),
            RasterDimension::uniform("y", 0.0, 2.0, 2),
        ],
        values_dims: vec!["y", "x"],
        values: vec![
            Some("#d7191c"),
            Some("#fdae61"),
            Some("#ffffbf"),
            Some("#abd9e9"),
            Some("#2c7bb6"),
            Some("#00000080"),
        ],
    }];
    let values = string_list_rows(&[rows[0].values.iter().map(|v| v.unwrap()).collect()]);
    let raster = raster_array(&rows, values);
    RecordBatch::try_from_iter(vec![("raster", raster)]).expect("raster batch")
}

fn multiple_rows_batch() -> RecordBatch {
    let rows = vec![
        uniform_raster(
            "x",
            0.0,
            3.0,
            3,
            "y",
            0.0,
            2.0,
            2,
            vec![
                Some(0.0),
                Some(1.0),
                Some(2.0),
                Some(3.0),
                Some(4.0),
                Some(5.0),
            ],
        ),
        uniform_raster(
            "x",
            1.0,
            4.0,
            3,
            "y",
            1.0,
            3.0,
            2,
            vec![
                Some(5.0),
                Some(4.0),
                Some(3.0),
                Some(2.0),
                Some(1.0),
                Some(0.0),
            ],
        ),
    ];
    let values = f64_list_rows(
        &rows
            .iter()
            .map(|row| row.values.clone())
            .collect::<Vec<_>>(),
    );
    let raster = raster_array(&rows, values);
    RecordBatch::try_from_iter(vec![
        ("raster", raster),
        (
            "alpha",
            Arc::new(Float64Array::from(vec![1.0, 0.45])) as ArrayRef,
        ),
    ])
    .expect("raster batch")
}

fn categorical_dimension_batch() -> RecordBatch {
    let rows = vec![RasterRow {
        dimensions: vec![
            RasterDimension::uniform("x", 0.0, 3.0, 3),
            RasterDimension::categorical("group", vec!["A", "B", "C"]),
        ],
        values_dims: vec!["group", "x"],
        values: vec![
            Some(0.0),
            Some(1.0),
            Some(2.0),
            Some(3.0),
            Some(4.0),
            Some(5.0),
            Some(6.0),
            Some(7.0),
            Some(8.0),
        ],
    }];
    let values = f64_list_rows(&[rows[0].values.clone()]);
    let raster = raster_array(&rows, values);
    RecordBatch::try_from_iter(vec![("raster", raster)]).expect("raster batch")
}

fn discrete_fill_batch() -> RecordBatch {
    let rows = vec![RasterRow {
        dimensions: vec![
            RasterDimension::uniform("x", 0.0, 4.0, 4),
            RasterDimension::uniform("y", 0.0, 3.0, 3),
        ],
        values_dims: vec!["y", "x"],
        values: vec![
            Some("low"),
            Some("medium"),
            Some("high"),
            Some("medium"),
            Some("high"),
            Some("low"),
            Some("medium"),
            Some("high"),
            Some("medium"),
            Some("high"),
            Some("low"),
            Some("medium"),
        ],
    }];
    let values = string_list_rows(&[rows[0].values.iter().map(|value| value.unwrap()).collect()]);
    let raster = raster_array(&rows, values);
    RecordBatch::try_from_iter(vec![("raster", raster)]).expect("raster batch")
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
        .map(|values| {
            uniform_raster(
                "Weight_in_lbs",
                X0,
                X1,
                NX as u32,
                "Miles_per_Gallon",
                Y0,
                Y1,
                NY as u32,
                values.into_iter().map(Some).collect(),
            )
        })
        .collect::<Vec<_>>();
    let values = f64_list_rows(
        &rows
            .iter()
            .map(|row| row.values.clone())
            .collect::<Vec<_>>(),
    );
    let raster = raster_array(&rows, values);
    let batch = RecordBatch::try_from_iter(vec![
        ("raster", raster),
        ("origin", Arc::new(StringArray::from(origins)) as ArrayRef),
    ])
    .expect("cars density batch");
    (batch, max_bin_count)
}

async fn taxi_dataframe(
    ctx: &SessionContext,
    x_col: &'static str,
    y_col: &'static str,
) -> datafusion::dataframe::DataFrame {
    let taxi_path = format!(
        "{}/tests/data/nyc_taxi_2015/nyc_taxi.csv",
        env!("CARGO_MANIFEST_DIR")
    );
    ctx.read_csv(taxi_path, CsvReadOptions::new())
        .await
        .expect("load NYC taxi fixture")
        .filter(
            col(x_col)
                .gt_eq(lit(TAXI_X_MIN))
                .and(col(x_col).lt_eq(lit(TAXI_X_MAX)))
                .and(col(y_col).gt_eq(lit(TAXI_Y_MIN)))
                .and(col(y_col).lt_eq(lit(TAXI_Y_MAX))),
        )
        .expect("filter taxi fixture to valid projected coordinates")
}

#[tokio::test]
async fn uniform_raster_2d_scaled_inferred_domain() {
    let ctx = SessionContext::new();
    let df = ctx
        .read_batch(raster_batch(vec![uniform_raster(
            "x",
            0.0,
            4.0,
            4,
            "y",
            0.0,
            3.0,
            3,
            (0..12).map(|value| Some(value as f64)).collect(),
        )]))
        .unwrap();
    let plot = Plot::<Cartesian>::new()
        .plot_size(240.0, 180.0)
        .data(df)
        .mark(
            UniformRaster2D::new()
                .raster_with(col("raster"), |r| r.x(dim("x")).y(dim("y")))
                .smooth(false),
        );

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
        .read_batch(raster_batch(vec![uniform_raster(
            "x",
            0.0,
            4.0,
            4,
            "y",
            0.0,
            3.0,
            3,
            (0..12).map(|value| Some(value as f64)).collect(),
        )]))
        .unwrap();
    let plot = Plot::<Cartesian>::new()
        .plot_size(240.0, 180.0)
        .data(df)
        .mark(
            UniformRaster2D::new()
                .raster_with(col("raster"), |r| {
                    r.x(dim("x")).y(dim("y")).fill(|fill| {
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
async fn uniform_raster_2d_axis_scale_fill_legend_config() {
    let ctx = SessionContext::new();
    let df = ctx
        .read_batch(raster_batch(vec![uniform_raster(
            "x",
            0.0,
            4.0,
            4,
            "y",
            0.0,
            3.0,
            3,
            (0..12).map(|value| Some(value as f64)).collect(),
        )]))
        .unwrap();
    let plot = Plot::<Cartesian>::new()
        .plot_size(270.0, 190.0)
        .data(df)
        .mark(
            UniformRaster2D::new()
                .raster_with(col("raster"), |r| {
                    r.x_with(dim("x"), |x| {
                        x.scale_with::<Linear>(|scale| {
                            scale.domain((-1.0, 5.0)).nice(false).zero(false)
                        })
                        .axis(|axis| axis.title("Configured X"))
                    })
                    .y_with(dim("y"), |y| {
                        y.scale_with::<Linear>(|scale| {
                            scale.domain((-0.5, 3.5)).nice(false).zero(false)
                        })
                        .axis(|axis| axis.title("Configured Y"))
                    })
                    .fill(|fill| {
                        fill.scale_with::<Linear>(|scale| {
                            scale.domain((0.0, 12.0)).nice(false).zero(false)
                        })
                        .legend(|legend| legend.title("Configured Fill"))
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
        "axis_scale_fill_legend_config",
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
        .mark(UniformRaster2D::new().raster_with(col("raster"), |r| {
            r.x(dim("x")).y(dim("y")).fill(|fill| fill.no_scale())
        }));

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
                .raster_with(col("raster"), |r| r.x(dim("x")).y(dim("y")))
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
async fn uniform_raster_2d_values_dims_storage_order() {
    let ctx = SessionContext::new();
    let rows = vec![RasterRow {
        dimensions: vec![
            RasterDimension::uniform("column_coord", 0.0, 2.0, 2),
            RasterDimension::uniform("row_coord", 0.0, 3.0, 3),
        ],
        values_dims: vec!["column_coord", "row_coord"],
        values: vec![
            Some(0.0),
            Some(5.0),
            Some(2.0),
            Some(8.0),
            Some(4.0),
            Some(10.0),
        ],
    }];
    let df = ctx.read_batch(raster_batch(rows)).unwrap();
    let plot = Plot::<Cartesian>::new()
        .plot_size(240.0, 160.0)
        .data(df)
        .mark(
            UniformRaster2D::new()
                .raster_with(col("raster"), |r| {
                    r.x(dim("column_coord"))
                        .y(dim("row_coord"))
                        .fill(|fill| fill.legend(|legend| legend.title("Value")))
                })
                .smooth(false),
        );

    let compiled = plot.compile(&ctx).await.unwrap();
    assert_visual_match(
        &compiled,
        &ctx,
        None,
        "uniform_raster_2d",
        "values_dims_storage_order",
        0.9999,
    )
    .await;
}

#[tokio::test]
async fn uniform_raster_2d_categorical_dimension() {
    let ctx = SessionContext::new();
    let df = ctx.read_batch(categorical_dimension_batch()).unwrap();
    let plot = Plot::<Cartesian>::new()
        .plot_size(250.0, 170.0)
        .data(df)
        .mark(
            UniformRaster2D::new()
                .raster_with(col("raster"), |r| {
                    r.x_with(dim("x"), |x| x.axis(|axis| axis.title("X")))
                        .y_with(dim("group"), |y| {
                            y.scale_with::<Band>(|scale| scale)
                                .axis(|axis| axis.title("Group"))
                        })
                        .fill(|fill| fill.legend(|legend| legend.title("Value")))
                })
                .smooth(false),
        );

    let compiled = plot.compile(&ctx).await.unwrap();
    assert_visual_match(
        &compiled,
        &ctx,
        None,
        "uniform_raster_2d",
        "categorical_dimension",
        0.9999,
    )
    .await;
}

#[tokio::test]
async fn uniform_raster_2d_discrete_fill_rect_legend_config() {
    let ctx = SessionContext::new();
    let df = ctx.read_batch(discrete_fill_batch()).unwrap();
    let plot = Plot::<Cartesian>::new()
        .plot_size(260.0, 180.0)
        .data(df)
        .mark(
            UniformRaster2D::new()
                .raster_with(col("raster"), |r| {
                    r.x(dim("x")).y(dim("y")).fill(|fill| {
                        fill.scale_with::<Ordinal>(|scale| {
                            scale
                                .domain_discrete(vec![lit("low"), lit("medium"), lit("high")])
                                .range_discrete(vec!["#e8f5e9", "#66bb6a", "#1b5e20"])
                        })
                        .legend(|legend| legend.title("Band"))
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
        "discrete_fill_rect_legend_config",
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
                r.x_with(dim("Weight_in_lbs"), |x| {
                    x.axis(|axis| axis.title("Weight"))
                })
                .y_with(dim("Miles_per_Gallon"), |y| {
                    y.axis(|axis| axis.title("MPG"))
                })
                .fill(move |fill| {
                    fill.scale_with::<Sqrt>(move |scale| {
                        scale.domain((0.0, max_bin_count)).nice(false).zero(false)
                    })
                    .legend(|legend| legend.title("Cars"))
                })
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

#[tokio::test]
async fn uniform_raster_2d_rasterize_taxi_dropoff_count_inferred_extent() {
    let ctx = SessionContext::new();
    let df = taxi_dataframe(&ctx, "dropoff_x", "dropoff_y").await;
    let hist = Rasterize2D::new(col("dropoff_x"), col("dropoff_y"))
        .x(|x| x.bins(96))
        .y(|y| y.bins(96))
        .agg("count");
    let plot = Plot::with_coord(Cartesian::new().unit_aspect(1.0))
        .plot_size(320.0, 250.0)
        .data(df)
        .mark(
            UniformRaster2D::new()
                .transform(hist, |mark, hist| {
                    mark.raster_with(hist.raster(), |r| {
                        r.x_with(hist.x_dim(), |x| {
                            x.scale_with::<Linear>(|scale| scale.nice(false).zero(false))
                                .axis(|axis| axis.title("Dropoff x").tick_count(4).format(".4~s"))
                        })
                        .y_with(hist.y_dim(), |y| {
                            y.scale_with::<Linear>(|scale| scale.nice(false).zero(false))
                                .axis(|axis| axis.title("Dropoff y").tick_count(4).format(".4~s"))
                        })
                        .fill(|fill| {
                            fill.scale_with::<Sqrt>(|scale| scale)
                                .legend(|legend| legend.title("Trips"))
                        })
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
        "rasterize_taxi_dropoff_count_inferred_extent",
        0.9999,
    )
    .await;
}

#[tokio::test]
async fn uniform_raster_2d_rasterize_taxi_pickup_count_explicit_domain() {
    let ctx = SessionContext::new();
    let df = taxi_dataframe(&ctx, "pickup_x", "pickup_y").await;
    let hist = Rasterize2D::new(col("pickup_x"), col("pickup_y"))
        .x(|x| x.extent(TAXI_X_MIN, TAXI_X_MAX).bins(96))
        .y(|y| y.extent(TAXI_Y_MIN, TAXI_Y_MAX).bins(96))
        .agg("count");
    let plot = Plot::with_coord(Cartesian::new().unit_aspect(1.0))
        .plot_size(320.0, 250.0)
        .data(df)
        .mark(
            UniformRaster2D::new()
                .transform(hist, |mark, hist| {
                    mark.raster_with(hist.raster(), |r| {
                        r.x_with(hist.x_dim(), |x| {
                            x.scale_with::<Linear>(|scale| scale.nice(false).zero(false))
                                .axis(|axis| axis.title("Pickup x").tick_count(4).format(".4~s"))
                        })
                        .y_with(hist.y_dim(), |y| {
                            y.scale_with::<Linear>(|scale| scale.nice(false).zero(false))
                                .axis(|axis| axis.title("Pickup y").tick_count(4).format(".4~s"))
                        })
                        .fill(|fill| {
                            fill.scale_with::<Sqrt>(|scale| {
                                scale.domain((0.0, 120.0)).nice(false).zero(false)
                            })
                            .legend(|legend| legend.title("Trips"))
                        })
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
        "rasterize_taxi_pickup_count_explicit_domain",
        0.9999,
    )
    .await;
}

#[tokio::test]
async fn uniform_raster_2d_rasterize_taxi_pickup_count_facet_payment_type() {
    let ctx = SessionContext::new();
    let df = taxi_dataframe(&ctx, "pickup_x", "pickup_y").await;
    let hist = Rasterize2D::new(col("pickup_x"), col("pickup_y"))
        .x(|x| x.extent(TAXI_X_MIN, TAXI_X_MAX).bins(64))
        .y(|y| y.extent(TAXI_Y_MIN, TAXI_Y_MAX).bins(64))
        .partition_by([col("payment_type")])
        .agg("count");
    let leaf = Plot::with_coord(Cartesian::new().unit_aspect(1.0)).mark(
        UniformRaster2D::new()
            .transform_shared(hist, |mark, hist| {
                mark.raster_with(hist.raster(), |r| {
                    r.x_with(hist.x_dim(), |x| {
                        x.scale_with::<Linear>(|scale| scale.nice(false).zero(false))
                            .axis(|axis| axis.title("Pickup x").tick_count(3).format(".4~s"))
                    })
                    .y_with(hist.y_dim(), |y| {
                        y.scale_with::<Linear>(|scale| scale.nice(false).zero(false))
                            .axis(|axis| axis.title("Pickup y").tick_count(3).format(".4~s"))
                    })
                    .fill(|fill| {
                        fill.scale_with::<Sqrt>(|scale| {
                            scale.domain((0.0, 80.0)).nice(false).zero(false)
                        })
                        .legend(|legend| legend.title("Trips"))
                    })
                })
            })
            .smooth(false),
    );
    let plot = Plot::<FacetColumn>::new()
        .plot_size(175.0, 145.0)
        .data(df)
        .mark(Subplot::new(leaf).column(col("payment_type")));

    let compiled = plot.compile(&ctx).await.unwrap();
    assert_visual_match(
        &compiled,
        &ctx,
        None,
        "uniform_raster_2d",
        "rasterize_taxi_pickup_count_facet_payment_type",
        0.9999,
    )
    .await;
}

#[tokio::test]
async fn uniform_raster_2d_rasterize_taxi_pickup_count_facet_payment_type_free_fill() {
    let ctx = SessionContext::new();
    let df = taxi_dataframe(&ctx, "pickup_x", "pickup_y").await;
    let hist = Rasterize2D::new(col("pickup_x"), col("pickup_y"))
        .x(|x| x.extent(TAXI_X_MIN, TAXI_X_MAX).bins(64))
        .y(|y| y.extent(TAXI_Y_MIN, TAXI_Y_MAX).bins(64))
        .partition_by([col("payment_type")])
        .agg("count");
    let leaf = Plot::with_coord(Cartesian::new().unit_aspect(1.0)).mark(
        UniformRaster2D::new()
            .transform_shared(hist, |mark, hist| {
                mark.raster_with(hist.raster(), |r| {
                    r.x_with(hist.x_dim(), |x| {
                        x.scale_with::<Linear>(|scale| scale.nice(false).zero(false))
                            .axis(|axis| axis.title("Pickup x").tick_count(3).format(".4~s"))
                    })
                    .y_with(hist.y_dim(), |y| {
                        y.scale_with::<Linear>(|scale| scale.nice(false).zero(false))
                            .axis(|axis| axis.title("Pickup y").tick_count(3).format(".4~s"))
                    })
                    .fill(|fill| {
                        fill.scale_with::<Sqrt>(|scale| scale.nice(false).zero(false))
                            .free_domain()
                            .legend(|legend| legend.title("Trips"))
                    })
                })
            })
            .smooth(false),
    );
    let plot = Plot::<FacetColumn>::new()
        .plot_size(175.0, 145.0)
        .data(df)
        .mark(Subplot::new(leaf).column(col("payment_type")));

    let compiled = plot.compile(&ctx).await.unwrap();
    assert_visual_match(
        &compiled,
        &ctx,
        None,
        "uniform_raster_2d",
        "rasterize_taxi_pickup_count_facet_payment_type_free_fill",
        0.9999,
    )
    .await;
}
