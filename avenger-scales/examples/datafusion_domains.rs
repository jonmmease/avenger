//! Fits scale domains in Rust to summaries that DataFusion queries compute: an x scale for circles
//! whose radii come from a size scale, shared by two facets, and a band scale whose categories are
//! in order of their totals. The `datafusion_plan` example fits domains inside one plan instead.
//!
//! Run with `cargo run --release -p avenger-scales --features datafusion --example datafusion_domains`.

use std::error::Error;
use std::sync::Arc;

use arrow::array::{Array, ArrayRef, AsArray, Float64Array, RecordBatch, StringArray};
use arrow::compute::concat;
use avenger_scales::datafusion::{config_expr, scale_expr, summary_expr};
use avenger_scales::scales::band::BandScale;
use avenger_scales::scales::domain_summary::{
    CategoryOptions, CategoryOrder, DomainOptions, DomainSummary, IntervalOptions,
};
use avenger_scales::scales::linear::LinearScale;
use datafusion::functions_aggregate::expr_fn::sum;
use datafusion::logical_expr::col;
use datafusion::prelude::{DataFrame, SessionContext};

/// The summaries in a query's last column, row by row.
async fn summaries(df: DataFrame) -> Result<Vec<DomainSummary>, Box<dyn Error>> {
    let batches = df.collect().await?;
    let mut summaries = Vec::new();
    for batch in batches {
        let column = batch.column(batch.num_columns() - 1);
        for row in 0..column.len() {
            summaries.push(DomainSummary::from_arrow(column.as_ref(), row)?);
        }
    }
    Ok(summaries)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let batch = RecordBatch::try_from_iter([
        (
            "facet",
            Arc::new(StringArray::from(vec!["a", "a", "a", "b", "b"])) as ArrayRef,
        ),
        (
            "category",
            Arc::new(StringArray::from(vec!["x", "y", "z", "y", "z"])),
        ),
        (
            "x",
            Arc::new(Float64Array::from(vec![2.0, 5.0, 9.0, 3.0, 12.0])),
        ),
        (
            "size",
            Arc::new(Float64Array::from(vec![1.0, 4.0, 2.0, 8.0, 3.0])),
        ),
        (
            "total",
            Arc::new(Float64Array::from(vec![10.0, 3.0, 7.0, 5.0, 2.0])),
        ),
    ])?;
    let df = SessionContext::new().read_batch(batch)?;
    let nice_options = DomainOptions::Interval(IntervalOptions {
        nice: Some(10.0),
        ..Default::default()
    });

    // The size scale maps sizes to circle radii from 2 to 12 px.
    let size = LinearScale::unfitted((2.0, 12.0));
    let size_summary = summary_expr(&size, col("size"), None)?;
    let size_summary = summaries(df.clone().aggregate(vec![], vec![size_summary])?).await?;
    let size = size.fit_domain(&nice_options, &size_summary[0])?;

    // Each facet summarizes its circles' positions with their radii, and the facets share x.
    let radius = scale_expr(&LinearScale, col("size"), config_expr(&size.config)?)?;
    let x = LinearScale::unfitted((0.0, 300.0));
    let x_summary = summary_expr(&x, col("x"), Some((radius.clone(), radius)))?;
    let facets = summaries(df.clone().aggregate(vec![col("facet")], vec![x_summary])?).await?;
    let shared = facets[0].merge(&facets[1])?;
    let x = x.fit_domain(&nice_options, &shared)?;
    println!("x domain: {:?}", x.numeric_interval_domain()?);

    // The band scale's categories are in descending order of their totals.
    let order = df
        .clone()
        .aggregate(vec![col("category")], vec![sum(col("total")).alias("sum")])?
        .sort(vec![col("sum").sort(false, true)])?
        .select(vec![col("category")])?
        .collect()
        .await?;
    let order = concat(
        &order
            .iter()
            .map(|batch| batch.column(0).as_ref())
            .collect::<Vec<_>>(),
    )?;
    let band = BandScale::unfitted((0.0, 300.0));
    let categories = summary_expr(&band, col("category"), None)?;
    let categories = summaries(df.aggregate(vec![], vec![categories])?).await?;
    let options = DomainOptions::Categories(CategoryOptions {
        order: CategoryOrder::Explicit(order),
    });
    let band = band.fit_domain(&options, &categories[0])?;
    let band_domain: Vec<_> = band
        .config
        .domain
        .as_string::<i32>()
        .iter()
        .flatten()
        .collect();
    println!("band domain: {band_domain:?}");
    Ok(())
}
