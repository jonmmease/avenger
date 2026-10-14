//! Fits scale domains inside one DataFusion plan: circles whose radii come from a size scale
//! share an x scale with labels, and a band scale leaves room for the circles at its outer bands.
//!
//! Run with `cargo run --release -p avenger-scales --features datafusion --example datafusion_plan`.

use std::error::Error;
use std::sync::Arc;

use arrow::array::{ArrayRef, Float64Array, RecordBatch, StringArray};
use arrow::util::pretty::pretty_format_batches;
use avenger_scales::datafusion::{fit_expr, scale_expr, summary_expr};
use avenger_scales::scales::band::BandScale;
use avenger_scales::scales::domain_summary::{CategoryOptions, DomainOptions, IntervalOptions};
use avenger_scales::scales::linear::LinearScale;
use datafusion::common::Result;
use datafusion::logical_expr::expr_fn::scalar_subquery;
use datafusion::logical_expr::{col, lit, Expr};
use datafusion::prelude::{DataFrame, SessionContext};

/// The configuration that `fit` returns over all of `df`'s rows, as a one-row subquery.
fn fitted_config(df: DataFrame, fit: Expr) -> Result<Expr> {
    let plan = df.aggregate(vec![], vec![fit.alias("config")])?;
    Ok(scalar_subquery(Arc::new(plan.into_unoptimized_plan())))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let ctx = SessionContext::new();
    let points = ctx.read_batch(RecordBatch::try_from_iter([
        (
            "category",
            Arc::new(StringArray::from(vec!["a", "b", "c", "b", "c"])) as ArrayRef,
        ),
        (
            "x",
            Arc::new(Float64Array::from(vec![2.0, 5.0, 9.0, 3.0, 12.0])),
        ),
        (
            "size",
            Arc::new(Float64Array::from(vec![1.0, 4.0, 2.0, 8.0, 3.0])),
        ),
    ])?)?;
    let labels = ctx.read_batch(RecordBatch::try_from_iter([(
        "x",
        Arc::new(Float64Array::from(vec![-3.0, 15.0])) as ArrayRef,
    )])?)?;

    // The size scale maps sizes to circle radii from 2 to 12 px.
    let size_scale = LinearScale::unfitted((2.0, 12.0));
    let size_options = DomainOptions::Interval(IntervalOptions::default());
    let size_summary = summary_expr(&size_scale, col("size"), None)?;
    let size_fit = fit_expr(&size_scale, size_summary, &size_options)?;
    let size_config = fitted_config(points.clone(), size_fit)?;
    let radius = scale_expr(size_scale.scale_impl.as_ref(), size_config, col("size"))?;
    let circles = points.select(vec![col("category"), col("x"), radius.alias("r")])?;

    // Circles and labels share x: one summary of the union of their rows, with nice ends.
    let x_scale = LinearScale::unfitted((0.0, 400.0));
    let x_options = DomainOptions::Interval(IntervalOptions {
        nice: Some(10.0),
        ..Default::default()
    });
    let x_rows = circles
        .clone()
        .select(vec![col("x"), col("r")])?
        .union(labels.select(vec![col("x"), lit(0.0_f32).alias("r")])?)?;
    let x_summary = summary_expr(&x_scale, col("x"), Some((col("r"), col("r"))))?;
    let x_config = fitted_config(x_rows, fit_expr(&x_scale, x_summary, &x_options)?)?;

    // Bands for the categories, with clip padding that keeps the outer circles in the range.
    let y_scale = BandScale::unfitted((300.0, 0.0));
    let y_options = DomainOptions::Categories(CategoryOptions::default());
    let y_summary = summary_expr(&y_scale, col("category"), Some((col("r"), col("r"))))?;
    let y_config = fitted_config(circles.clone(), fit_expr(&y_scale, y_summary, &y_options)?)?;

    // One plan from rows to pixels.
    let marks = circles.select(vec![
        scale_expr(x_scale.scale_impl.as_ref(), x_config, col("x"))?.alias("px"),
        scale_expr(y_scale.scale_impl.as_ref(), y_config, col("category"))?.alias("py"),
        col("r"),
    ])?;
    println!("{}", pretty_format_batches(&marks.collect().await?)?);
    Ok(())
}
