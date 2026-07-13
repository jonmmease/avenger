//! Categorical raster overlay baselines: Rasterize2D::by + fill_by render
//! K category planes as ONE Oklab-mixed image with density-driven alpha
//! (scratch/categorical-raster-plan.md T7).

use std::sync::Arc;

use avenger_chart::prelude::*;
use datafusion::{
    arrow::{
        array::{ArrayRef, Float64Array, ListBuilder, StringArray, StringBuilder, StructArray},
        buffer::OffsetBuffer,
        datatypes::{DataType, Field},
        record_batch::RecordBatch,
    },
    prelude::{SessionContext, col, lit},
};

use super::helpers::assert_visual_match;

const CATEGORY: &str = "categorical_raster";

/// Point rows (x, y, cat) — one row per (point, category, count) so counts
/// are explicit and deterministic.
fn points_dataframe(ctx: &SessionContext, rows: &[(f64, f64, &str, usize)]) -> DataFrame {
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    let mut cats = Vec::new();
    for (x, y, cat, count) in rows {
        for _ in 0..*count {
            xs.push(*x);
            ys.push(*y);
            cats.push(*cat);
        }
    }
    let batch = RecordBatch::try_new(
        Arc::new(datafusion::arrow::datatypes::Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
            Field::new("cat", DataType::Utf8, false),
        ])),
        vec![
            Arc::new(Float64Array::from(xs)) as ArrayRef,
            Arc::new(Float64Array::from(ys)) as ArrayRef,
            Arc::new(StringArray::from(cats)) as ArrayRef,
        ],
    )
    .expect("points batch");
    ctx.read_batch(batch).expect("points dataframe")
}

/// Two categories with pure regions and an overlap band; inferred domain
/// and swatch legend. Pins pure-color exactness and the Oklab mix.
#[tokio::test]
async fn categorical_raster_overlay_two_categories() {
    let ctx = SessionContext::new();
    // 4x3 grid over [0,4]x[0,3]: left cells pure "a", right cells pure
    // "b", middle column mixed at varying ratios.
    let df = points_dataframe(
        &ctx,
        &[
            (0.5, 0.5, "a", 8),
            (0.5, 1.5, "a", 6),
            (0.5, 2.5, "a", 4),
            (1.5, 0.5, "a", 6),
            (1.5, 1.5, "a", 4),
            (1.5, 2.5, "a", 2),
            // overlap column: a/b ratios 3:1, 1:1, 1:3
            (2.5, 0.5, "a", 6),
            (2.5, 0.5, "b", 2),
            (2.5, 1.5, "a", 4),
            (2.5, 1.5, "b", 4),
            (2.5, 2.5, "a", 2),
            (2.5, 2.5, "b", 6),
            (3.5, 0.5, "b", 8),
            (3.5, 1.5, "b", 6),
            (3.5, 2.5, "b", 4),
        ],
    );
    let plot = Chart::<Cartesian>::new()
        .plot_size(260.0, 180.0)
        .data(df)
        .mark(
            UniformRaster2D::new().transform(
                Rasterize2D::new(col("x"), col("y"))
                    .x(|x| x.extent(0.0, 4.0).bins(4))
                    .y(|y| y.extent(0.0, 3.0).bins(3))
                    .by(col("cat"))
                    .agg("count"),
                |mark, hist| {
                    mark.raster_with(hist.raster(), |r| {
                        r.x(hist.x_dim())
                            .y(hist.y_dim())
                            .fill_by(hist.by_dim(), |fill| fill.legend(|l| l.title("Group")))
                    })
                },
            ),
        );
    let compiled = plot.compile(&ctx).await.expect("compile");
    assert_visual_match(
        &compiled,
        &ctx,
        None,
        CATEGORY,
        "overlay_two_categories",
        0.9999,
    )
    .await;
}

/// Six categories through the theme's default categorical scheme
/// (Okabe-Ito), six-entry legend.
#[tokio::test]
async fn categorical_raster_overlay_six_categories() {
    let ctx = SessionContext::new();
    let cats = ["c1", "c2", "c3", "c4", "c5", "c6"];
    let mut rows: Vec<(f64, f64, &str, usize)> = Vec::new();
    for (index, cat) in cats.iter().enumerate() {
        // A pure cell per category along the diagonal, plus a shared
        // center cell where all six mix equally.
        rows.push((index as f64 + 0.5, (index % 3) as f64 + 0.5, cat, 6));
        rows.push((3.5, 1.5, cat, 2));
    }
    let df = points_dataframe(&ctx, &rows);
    let plot = Chart::<Cartesian>::new()
        .plot_size(280.0, 180.0)
        .data(df)
        .mark(
            UniformRaster2D::new().transform(
                Rasterize2D::new(col("x"), col("y"))
                    .x(|x| x.extent(0.0, 6.0).bins(6))
                    .y(|y| y.extent(0.0, 3.0).bins(3))
                    .by(col("cat"))
                    .agg("count"),
                |mark, hist| {
                    mark.raster_with(hist.raster(), |r| {
                        r.x(hist.x_dim())
                            .y(hist.y_dim())
                            .fill_by(hist.by_dim(), |fill| fill.legend(|l| l.title("Class")))
                    })
                },
            ),
        );
    let compiled = plot.compile(&ctx).await.expect("compile");
    assert_visual_match(
        &compiled,
        &ctx,
        None,
        CATEGORY,
        "overlay_six_categories",
        0.9999,
    )
    .await;
}

/// Density gradient through a Sqrt opacity scale with a visible range
/// floor (the datashader min_alpha analog).
#[tokio::test]
async fn categorical_raster_overlay_opacity_sqrt() {
    let ctx = SessionContext::new();
    let mut rows: Vec<(f64, f64, &str, usize)> = Vec::new();
    for step in 0..8 {
        let count = 1 + step * step; // 1, 2, 5, 10, 17, 26, 37, 50
        rows.push((step as f64 + 0.5, 0.5, "a", count));
        rows.push((step as f64 + 0.5, 1.5, "b", count));
    }
    let df = points_dataframe(&ctx, &rows);
    let plot = Chart::<Cartesian>::new()
        .plot_size(280.0, 140.0)
        .data(df)
        .mark(
            UniformRaster2D::new().transform(
                Rasterize2D::new(col("x"), col("y"))
                    .x(|x| x.extent(0.0, 8.0).bins(8))
                    .y(|y| y.extent(0.0, 2.0).bins(2))
                    .by(col("cat"))
                    .agg("count"),
                |mark, hist| {
                    mark.raster_with(hist.raster(), |r| {
                        r.x(hist.x_dim())
                            .y(hist.y_dim())
                            .fill_by(hist.by_dim(), |fill| fill.legend(|l| l.title("Group")))
                            .opacity_by_total(|o| {
                                o.scale_with::<Sqrt>(|s| {
                                    s.domain((0.0, 50.0))
                                        .range_interval(lit(0.25), lit(1.0))
                                        .clamp(true)
                                        .nice(false)
                                        .zero(false)
                                })
                            })
                    })
                },
            ),
        );
    let compiled = plot.compile(&ctx).await.expect("compile");
    assert_visual_match(
        &compiled,
        &ctx,
        None,
        CATEGORY,
        "overlay_opacity_sqrt",
        0.9999,
    )
    .await;
}

/// Explicit domain includes a category absent from the data: the legend
/// keeps all three entries with stable colors, planes match BY VALUE.
#[tokio::test]
async fn categorical_raster_overlay_domain_stability() {
    let ctx = SessionContext::new();
    let df = points_dataframe(
        &ctx,
        &[
            (0.5, 0.5, "bike", 6),
            (1.5, 0.5, "walk", 6),
            (0.5, 1.5, "bike", 3),
            (0.5, 1.5, "walk", 3),
        ],
    );
    let plot = Chart::<Cartesian>::new()
        .plot_size(240.0, 180.0)
        .data(df)
        .mark(
            UniformRaster2D::new().transform(
                Rasterize2D::new(col("x"), col("y"))
                    .x(|x| x.extent(0.0, 2.0).bins(2))
                    .y(|y| y.extent(0.0, 2.0).bins(2))
                    .by(col("cat"))
                    .agg("count"),
                |mark, hist| {
                    mark.raster_with(hist.raster(), |r| {
                        r.x(hist.x_dim())
                            .y(hist.y_dim())
                            .fill_by(hist.by_dim(), |fill| {
                                fill.scale(|s| {
                                    s.domain_discrete(vec![lit("bike"), lit("bus"), lit("walk")])
                                })
                                .legend(|l| l.title("Mode"))
                            })
                    })
                },
            ),
        );
    let compiled = plot.compile(&ctx).await.expect("compile");
    assert_visual_match(
        &compiled,
        &ctx,
        None,
        CATEGORY,
        "overlay_domain_stability",
        0.9999,
    )
    .await;
}

/// External (mark-only) 3D raster input: a hand-built raster struct with a
/// categorical dimension renders through the same overlay path with the
/// dim-values domain inference.
#[tokio::test]
async fn categorical_raster_external_3d() {
    let ctx = SessionContext::new();
    let batch = external_3d_raster_batch();
    let df = ctx.read_batch(batch).expect("raster dataframe");
    let plot = Chart::<Cartesian>::new()
        .plot_size(240.0, 180.0)
        .data(df)
        .mark(
            UniformRaster2D::new()
                .raster_with(col("raster"), |r| {
                    r.x(dim("x"))
                        .y(dim("y"))
                        .fill_by(dim("species"), |fill| fill.legend(|l| l.title("Species")))
                })
                .smooth(false),
        );
    let compiled = plot.compile(&ctx).await.expect("compile");
    assert_visual_match(&compiled, &ctx, None, CATEGORY, "external_3d", 0.9999).await;
}

/// Hand-built raster struct row: dims x (uniform, 3), y (uniform, 2),
/// species (categorical, ["fir", "oak"]); values.dims [species, y, x].
fn external_3d_raster_batch() -> RecordBatch {
    let mut values_builder = ListBuilder::new(StringBuilder::new());
    values_builder.append(false); // x
    values_builder.append(false); // y
    values_builder.values().append_value("fir");
    values_builder.values().append_value("oak");
    values_builder.append(true); // species
    let coord_values = Arc::new(values_builder.finish()) as ArrayRef;

    let coords = Arc::new(StructArray::from(vec![
        (
            Arc::new(Field::new("kind", DataType::Utf8, false)),
            Arc::new(StringArray::from(vec!["uniform", "uniform", "categorical"])) as ArrayRef,
        ),
        (
            Arc::new(Field::new("sampling", DataType::Utf8, true)),
            Arc::new(StringArray::from(vec![
                Some("linear"),
                Some("linear"),
                None,
            ])) as ArrayRef,
        ),
        (
            Arc::new(Field::new("start", DataType::Float64, true)),
            Arc::new(Float64Array::from(vec![Some(0.0), Some(0.0), None])) as ArrayRef,
        ),
        (
            Arc::new(Field::new("stop", DataType::Float64, true)),
            Arc::new(Float64Array::from(vec![Some(3.0), Some(2.0), None])) as ArrayRef,
        ),
        (
            Arc::new(Field::new("count", DataType::UInt32, true)),
            Arc::new(datafusion::arrow::array::UInt32Array::from(vec![
                Some(3),
                Some(2),
                None,
            ])) as ArrayRef,
        ),
        (
            Arc::new(Field::new("values", coord_values.data_type().clone(), true)),
            coord_values,
        ),
    ])) as ArrayRef;

    let dimension_names = Arc::new(StringArray::from(vec!["x", "y", "species"])) as ArrayRef;
    let dimensions = Arc::new(StructArray::from(vec![
        (
            Arc::new(Field::new("name", DataType::Utf8, false)),
            dimension_names,
        ),
        (
            Arc::new(Field::new("coords", coords.data_type().clone(), false)),
            coords,
        ),
    ])) as ArrayRef;
    let dimensions = Arc::new(datafusion::arrow::array::ListArray::new(
        Arc::new(Field::new("item", dimensions.data_type().clone(), false)),
        OffsetBuffer::from_lengths([3]),
        dimensions,
        None,
    )) as ArrayRef;

    let geometry = Arc::new(StructArray::from(vec![
        (
            Arc::new(Field::new("kind", DataType::Utf8, false)),
            Arc::new(StringArray::from(vec!["grid"])) as ArrayRef,
        ),
        (
            Arc::new(Field::new("crs", DataType::Utf8, true)),
            Arc::new(StringArray::from(vec![None::<&str>])) as ArrayRef,
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

    let mut dims_builder = ListBuilder::new(StringBuilder::new());
    dims_builder.values().append_value("species");
    dims_builder.values().append_value("y");
    dims_builder.values().append_value("x");
    dims_builder.append(true);
    let dims = Arc::new(dims_builder.finish()) as ArrayRef;

    // Plane-major [species, y, x]: fir dominates the left, oak the right,
    // the center column mixes, one empty cell.
    let data_values = Arc::new(Float64Array::from(vec![
        // fir plane (y0: x0..x2, y1: x0..x2)
        Some(9.0),
        Some(3.0),
        Some(0.0),
        Some(6.0),
        Some(3.0),
        Some(0.0),
        // oak plane
        Some(0.0),
        Some(3.0),
        Some(9.0),
        Some(0.0),
        Some(0.0),
        Some(0.0),
    ])) as ArrayRef;
    let data = Arc::new(datafusion::arrow::array::ListArray::new(
        Arc::new(Field::new("item", DataType::Float64, true)),
        OffsetBuffer::from_lengths([12]),
        data_values,
        None,
    )) as ArrayRef;

    let values = Arc::new(StructArray::from(vec![
        (
            Arc::new(Field::new("dims", dims.data_type().clone(), false)),
            dims,
        ),
        (
            Arc::new(Field::new("data", data.data_type().clone(), false)),
            data,
        ),
    ])) as ArrayRef;

    let raster = Arc::new(StructArray::from(vec![
        (
            Arc::new(Field::new("geometry", geometry.data_type().clone(), false)),
            geometry,
        ),
        (
            Arc::new(Field::new("values", values.data_type().clone(), false)),
            values,
        ),
    ])) as ArrayRef;

    RecordBatch::try_from_iter(vec![("raster", raster)]).expect("raster batch")
}

use datafusion::dataframe::DataFrame;
