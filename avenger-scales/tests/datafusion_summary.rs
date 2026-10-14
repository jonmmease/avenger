use std::sync::Arc;

use arrow::array::{
    Array, ArrayRef, AsArray, Date32Array, Float64Array, Int64Array, RecordBatch, StringArray,
    StructArray,
};
use arrow::compute::concat_batches;
use arrow::datatypes::{DataType, Field, Fields, Float32Type, Schema};
use avenger_scales::datafusion::{config_literal, scale_expr, summary_expr, ScaleExtensionCodec};
use avenger_scales::scales::band::BandScale;
use avenger_scales::scales::domain_summary::{
    DomainOptions, DomainSummary, Extents, IntervalOptions,
};
use avenger_scales::scales::linear::LinearScale;
use avenger_scales::scales::log::LogScale;
use avenger_scales::scales::nested_band::NestedBandScale;
use avenger_scales::scales::quantile::QuantileScale;
use avenger_scales::scales::threshold::ThresholdScale;
use avenger_scales::scales::time::TimeScale;
use avenger_scales::scales::ConfiguredScale;
use datafusion::common::Result;
use datafusion::datasource::MemTable;
use datafusion::logical_expr::{col, lit, Expr};
use datafusion::prelude::{DataFrame, SessionConfig, SessionContext};
use datafusion_proto::logical_plan::from_proto::parse_expr;
use datafusion_proto::logical_plan::to_proto::serialize_expr;

const ROWS: usize = 500;

/// Deterministic values in `[0, 1)`.
fn random(count: usize, seed: u64) -> Vec<f64> {
    let mut state = seed;
    (0..count)
        .map(|_| {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 11) as f64 / (1u64 << 53) as f64
        })
        .collect()
}

fn numbers(values: Vec<f64>) -> ArrayRef {
    Arc::new(Float64Array::from(values))
}

fn strings(values: Vec<String>) -> ArrayRef {
    Arc::new(StringArray::from(values))
}

/// A table of `columns` whose rows are spread over three partitions of small batches, so that
/// queries merge partial summaries.
fn table(columns: Vec<(&str, ArrayRef)>) -> DataFrame {
    let schema = Arc::new(Schema::new(
        columns
            .iter()
            .map(|(name, column)| Field::new(*name, column.data_type().clone(), true))
            .collect::<Vec<_>>(),
    ));
    let arrays: Vec<ArrayRef> = columns.into_iter().map(|(_, column)| column).collect();
    let batch = RecordBatch::try_new(schema.clone(), arrays).unwrap();
    let mut partitions = vec![Vec::new(); 3];
    for (index, offset) in (0..batch.num_rows()).step_by(40).enumerate() {
        let length = 40.min(batch.num_rows() - offset);
        partitions[index % 3].push(batch.slice(offset, length));
    }
    let config = SessionConfig::new()
        .with_target_partitions(4)
        .with_batch_size(16);
    let ctx = SessionContext::new_with_config(config);
    let table = MemTable::try_new(schema, partitions).unwrap();
    ctx.read_table(Arc::new(table)).unwrap()
}

fn linear() -> ConfiguredScale {
    LinearScale::unfitted((0.0, 1.0))
}

/// The summaries of a query's only column, row by row.
async fn summaries(df: DataFrame) -> Result<Vec<DomainSummary>> {
    let schema = Arc::new(df.schema().as_arrow().clone());
    let batch = concat_batches(&schema, &df.collect().await?)?;
    let column = batch.column(batch.num_columns() - 1);
    Ok((0..column.len())
        .map(|row| DomainSummary::from_arrow(column.as_ref(), row).unwrap())
        .collect())
}

async fn summary(df: DataFrame, expr: Expr) -> Result<DomainSummary> {
    Ok(summaries(df.aggregate(vec![], vec![expr])?)
        .await?
        .remove(0))
}

/// The summary of `scale` over `values` in Rust.
fn rust_summary(scale: &ConfiguredScale, values: &ArrayRef, extents: Extents) -> DomainSummary {
    scale.summarize(values, extents).unwrap()
}

#[tokio::test]
async fn query_summaries_equal_the_summaries_of_all_rows() {
    let x = numbers(random(ROWS, 1).iter().map(|v| v * 100.0 - 50.0).collect());
    let positive = numbers(random(ROWS, 2).iter().map(|v| v * 1000.0 + 1.0).collect());
    let lower = random(ROWS, 3).iter().map(|v| v * 20.0).collect::<Vec<_>>();
    let upper = random(ROWS, 4).iter().map(|v| v * 20.0).collect::<Vec<_>>();
    let days: ArrayRef = Arc::new(Date32Array::from(
        random(ROWS, 5)
            .iter()
            .map(|v| (v * 3650.0) as i32)
            .collect::<Vec<_>>(),
    ));
    let names = |seed| {
        strings(
            random(ROWS, seed)
                .iter()
                .map(|v| format!("c{}", (v * 30.0) as i32))
                .collect(),
        )
    };
    let (category, group) = (names(6), names(7));
    let paths: ArrayRef = Arc::new(StructArray::new(
        Fields::from(vec![
            Field::new("group", DataType::Utf8, true),
            Field::new("item", DataType::Utf8, true),
        ]),
        vec![group, category.clone()],
        None,
    ));
    let df = table(vec![
        ("x", x.clone()),
        ("positive", positive.clone()),
        ("lower", numbers(lower.clone())),
        ("upper", numbers(upper.clone())),
        ("day", days.clone()),
        ("category", category.clone()),
        ("path", paths.clone()),
    ]);
    let per_row = Extents::PerRow {
        lower: &lower,
        upper: &upper,
    };
    // Each scale, the column it summarizes, and whether the marks have extents.
    let colors = strings(vec!["red".into(), "blue".into()]);
    let cases: [(ConfiguredScale, &str, &ArrayRef, bool); 6] = [
        (linear(), "x", &x, true),
        (LogScale::unfitted((0.0, 1.0)), "positive", &positive, false),
        (
            TimeScale::unfitted((0.0, 1.0), &DataType::Date32),
            "day",
            &days,
            true,
        ),
        (BandScale::unfitted((0.0, 1.0)), "category", &category, true),
        (NestedBandScale::unfitted((0.0, 1.0)), "path", &paths, false),
        (QuantileScale::unfitted(colors), "x", &x, false),
    ];
    for (scale, column, values, with_extents) in cases {
        let (extent_exprs, extents) = match with_extents {
            true => (Some((col("lower"), col("upper"))), per_row),
            false => (None, Extents::None),
        };
        let expr = summary_expr(&scale, col(column), extent_exprs).unwrap();
        assert_eq!(
            summary(df.clone(), expr).await.unwrap(),
            rust_summary(&scale, values, extents),
            "{}",
            scale.scale_impl.scale_type()
        );
    }
}

#[tokio::test]
async fn grouped_queries_summarize_each_group() {
    let x = random(ROWS, 8).iter().map(|v| v * 10.0).collect::<Vec<_>>();
    let facets: Vec<i64> = random(ROWS, 9).iter().map(|v| (v * 3.0) as i64).collect();
    let df = table(vec![
        ("facet", Arc::new(Int64Array::from(facets.clone()))),
        ("x", numbers(x.clone())),
    ]);
    let expr = summary_expr(&linear(), col("x"), None).unwrap();
    let grouped = df
        .aggregate(vec![col("facet")], vec![expr])
        .unwrap()
        .sort(vec![col("facet").sort(true, false)])
        .unwrap();
    let actual = summaries(grouped).await.unwrap();
    for (facet, summary) in actual.iter().enumerate() {
        let values = numbers(
            x.iter()
                .zip(&facets)
                .filter(|(_, f)| **f == facet as i64)
                .map(|(x, _)| *x)
                .collect(),
        );
        assert_eq!(summary, &rust_summary(&linear(), &values, Extents::None));
    }
    assert_eq!(actual.len(), 3);
}

#[tokio::test]
async fn literal_extents_and_missing_values() {
    let x: ArrayRef = Arc::new(Float64Array::from(vec![
        Some(1.0),
        None,
        Some(4.0),
        Some(2.0),
    ]));
    let lower: ArrayRef = Arc::new(Float64Array::from(vec![
        Some(1.0),
        None,
        Some(2.0),
        Some(3.0),
    ]));
    let gaps: ArrayRef = Arc::new(Float64Array::from(vec![
        Some(1.0),
        Some(5.0),
        None,
        Some(3.0),
    ]));
    let df = table(vec![("x", x.clone()), ("lower", lower), ("gaps", gaps)]);
    let constant = summary_expr(&linear(), col("x"), Some((lit(5.0), lit(3)))).unwrap();
    let extents = Extents::Constant {
        lower: 5.0,
        upper: 3.0,
    };
    assert_eq!(
        summary(df.clone(), constant).await.unwrap(),
        rust_summary(&linear(), &x, extents)
    );
    // A missing value has no extent, but a value without an extent is an error.
    let per_row = summary_expr(&linear(), col("x"), Some((col("lower"), lit(0.0)))).unwrap();
    assert!(summary(df.clone(), per_row).await.is_ok());
    let missing = summary_expr(&linear(), col("x"), Some((col("gaps"), lit(0.0)))).unwrap();
    assert!(summary(df, missing).await.is_err());
}

#[tokio::test]
async fn no_rows_give_the_summary_of_no_data() {
    let df = table(vec![("x", numbers(vec![1.0]))])
        .filter(lit(false))
        .unwrap();
    let expr = summary_expr(&linear(), col("x"), None).unwrap();
    let empty = summary(df, expr).await.unwrap();
    assert_eq!(
        empty,
        rust_summary(&linear(), &numbers(vec![]), Extents::None)
    );
    let options = DomainOptions::Interval(IntervalOptions::default());
    let fitted = LinearScale::unfitted((0.0, 100.0))
        .fit_domain(&options, &empty)
        .unwrap();
    assert_eq!(fitted.numeric_interval_domain().unwrap(), (0.0, 1.0));
}

#[test]
fn invalid_summaries_fail_at_planning() {
    let df = table(vec![
        ("x", numbers(vec![1.0])),
        ("name", strings(vec!["a".into()])),
    ]);
    let plan = |scale: ConfiguredScale, column: &str| {
        let expr = summary_expr(&scale, col(column), None).unwrap();
        df.clone().aggregate(vec![], vec![expr])
    };
    // Threshold scales have no summaries, and numeric and time scales need their kinds of values.
    let threshold =
        ThresholdScale::configured(vec![0.5], strings(vec!["low".into(), "high".into()]));
    assert!(plan(threshold, "x").is_err());
    assert!(plan(linear(), "name").is_err());
    assert!(plan(TimeScale::unfitted((0.0, 1.0), &DataType::Date32), "name").is_err());
}

#[test]
fn plans_decode_summaries_without_registration() {
    let codec = ScaleExtensionCodec::default();
    let ctx = SessionContext::new();
    let expr = summary_expr(&linear(), col("x"), Some((lit(1.0), col("y")))).unwrap();
    let proto = serialize_expr(&expr, &codec).unwrap();
    assert_eq!(
        parse_expr(&proto, ctx.task_ctx().as_ref(), &codec).unwrap(),
        expr
    );
}

/// Circles whose radius comes from a size scale, and a second layer of points, share an x scale.
/// Fitting from query summaries gives the domain that fitting from Rust summaries gives.
#[tokio::test]
async fn query_summaries_fit_the_domain_that_rust_summaries_fit() {
    let x = random(ROWS, 10)
        .iter()
        .map(|v| v * 40.0)
        .collect::<Vec<_>>();
    let size = random(ROWS, 11)
        .iter()
        .map(|v| v * 9.0 + 1.0)
        .collect::<Vec<_>>();
    let df = table(vec![
        ("x", numbers(x.clone())),
        ("size", numbers(size.clone())),
    ]);
    let interval = DomainOptions::Interval(IntervalOptions::default());
    // The size scale's domain comes from its own summary.
    let size_expr = summary_expr(&linear(), col("size"), None).unwrap();
    let size_summary = summary(df.clone(), size_expr).await.unwrap();
    let radius = LinearScale::unfitted((2.0, 10.0))
        .fit_domain(&interval, &size_summary)
        .unwrap();
    let radius_config = config_literal(&radius.config).unwrap();
    let radius_expr = scale_expr(radius.scale_impl.as_ref(), radius_config, col("size")).unwrap();
    let circles = summary_expr(
        &linear(),
        col("x"),
        Some((radius_expr.clone(), radius_expr)),
    )
    .unwrap();
    let points = numbers(vec![-5.0, 60.0]);
    let layers = summary(df, circles)
        .await
        .unwrap()
        .merge(&rust_summary(&linear(), &points, Extents::None))
        .unwrap();
    let x_scale = LinearScale::unfitted((0.0, 400.0));
    let fitted = x_scale.clone().fit_domain(&interval, &layers).unwrap();
    // The same fit in Rust.
    let radii = radius.scale(&numbers(size)).unwrap();
    let radii: Vec<f64> = radii
        .as_primitive::<Float32Type>()
        .values()
        .iter()
        .map(|r| *r as f64)
        .collect();
    let extents = Extents::PerRow {
        lower: &radii,
        upper: &radii,
    };
    let expected = rust_summary(&linear(), &numbers(x), extents)
        .merge(&rust_summary(&linear(), &points, Extents::None))
        .unwrap();
    let expected = x_scale.fit_domain(&interval, &expected).unwrap();
    assert_eq!(
        fitted.numeric_interval_domain().unwrap(),
        expected.numeric_interval_domain().unwrap()
    );
}
