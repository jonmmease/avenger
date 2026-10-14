use std::sync::Arc;

use arrow::array::{
    Array, ArrayRef, AsArray, Date32Array, Float64Array, Int64Array, LargeStringArray, RecordBatch,
    StringArray, StructArray,
};
use arrow::compute::concat_batches;
use arrow::datatypes::{DataType, Field, Fields, Float32Type};
use avenger_scales::datafusion::{
    config_expr, config_from_arrow, fit_expr, scale_expr, summary_expr, ScaleExtensionCodec,
};
use avenger_scales::scales::band::BandScale;
use avenger_scales::scales::domain_summary::{
    CategoryOptions, CategoryOrder, DomainOptions, DomainSummary, Extents, IntervalOptions,
};
use avenger_scales::scales::linear::LinearScale;
use avenger_scales::scales::log::LogScale;
use avenger_scales::scales::nested_band::NestedBandScale;
use avenger_scales::scales::ordinal::OrdinalScale;
use avenger_scales::scales::quantile::QuantileScale;
use avenger_scales::scales::quantize::QuantizeScale;
use avenger_scales::scales::threshold::ThresholdScale;
use avenger_scales::scales::time::TimeScale;
use avenger_scales::scales::{ConfiguredScale, ScaleConfig};
use datafusion::common::{JoinType, Result};
use datafusion::logical_expr::expr_fn::scalar_subquery;
use datafusion::logical_expr::{col, lit, Expr, LogicalPlanBuilder};
use datafusion::prelude::{DataFrame, SessionConfig, SessionContext};
use datafusion_proto::bytes::{
    logical_plan_from_bytes_with_extension_codec, logical_plan_to_bytes_with_extension_codec,
};

fn numbers(values: Vec<f64>) -> ArrayRef {
    Arc::new(Float64Array::from(values))
}

fn strings(values: &[&str]) -> ArrayRef {
    Arc::new(StringArray::from(values.to_vec()))
}

fn interval(nice: Option<f64>) -> DomainOptions {
    DomainOptions::Interval(IntervalOptions {
        nice,
        ..Default::default()
    })
}

fn categories() -> DomainOptions {
    DomainOptions::Categories(CategoryOptions::default())
}

/// A table of `columns` whose rows are spread over three partitions of small batches.
fn table(ctx: &SessionContext, columns: Vec<(&str, ArrayRef)>) -> DataFrame {
    let batch = RecordBatch::try_from_iter(columns).unwrap();
    let mut partitions = vec![Vec::new(); 3];
    for (index, offset) in (0..batch.num_rows()).step_by(4).enumerate() {
        let length = 4.min(batch.num_rows() - offset);
        partitions[index % 3].push(batch.slice(offset, length));
    }
    let table = datafusion::datasource::MemTable::try_new(batch.schema(), partitions).unwrap();
    ctx.read_table(Arc::new(table)).unwrap()
}

/// A context that plans several partitions of small batches, so that queries merge partial
/// summaries.
fn context() -> SessionContext {
    let config = SessionConfig::new()
        .with_target_partitions(4)
        .with_batch_size(16);
    SessionContext::new_with_config(config)
}

async fn collect(df: DataFrame) -> Result<RecordBatch> {
    let schema = Arc::new(df.schema().as_arrow().clone());
    Ok(concat_batches(&schema, &df.collect().await?)?)
}

/// The summary that a query computes over all of `df`.
async fn summary(df: DataFrame, expr: Expr) -> Result<DomainSummary> {
    let batch = collect(df.aggregate(vec![], vec![expr])?).await?;
    Ok(DomainSummary::from_arrow(batch.column(0).as_ref(), 0).unwrap())
}

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

/// The configuration that a query fits over all of `df`.
async fn fitted_in_query(
    df: DataFrame,
    scale: &ConfiguredScale,
    values: Expr,
    extents: Option<(Expr, Expr)>,
    options: &DomainOptions,
) -> Result<ScaleConfig> {
    let summary = summary_expr(scale, values, extents)?;
    let fit = fit_expr(scale, summary, options)?;
    let batch = collect(df.aggregate(vec![], vec![fit])?).await?;
    config_from_arrow(batch.column(0).as_ref(), 0)
}

fn assert_same_config(actual: &ScaleConfig, expected: &ScaleConfig, name: &str) {
    assert_eq!(
        actual.domain.to_data(),
        expected.domain.to_data(),
        "{name} domain"
    );
    assert_eq!(
        actual.range.to_data(),
        expected.range.to_data(),
        "{name} range"
    );
    let mut keys: Vec<_> = expected.options.keys().collect();
    keys.sort();
    let mut actual_keys: Vec<_> = actual.options.keys().collect();
    actual_keys.sort();
    assert_eq!(actual_keys, keys, "{name} option names");
    for key in keys {
        assert_eq!(
            actual.options[key].0.to_data(),
            expected.options[key].0.to_data(),
            "{name} option {key}"
        );
    }
}

#[tokio::test]
async fn query_summaries_equal_the_summaries_of_all_rows() {
    const ROWS: usize = 500;
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
    let names = |seed| -> ArrayRef {
        let values = random(ROWS, seed);
        let names = values.iter().map(|v| format!("c{}", (v * 30.0) as i32));
        Arc::new(StringArray::from_iter_values(names))
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
    let df = table(
        &context(),
        vec![
            ("x", x.clone()),
            ("positive", positive.clone()),
            ("lower", numbers(lower.clone())),
            ("upper", numbers(upper.clone())),
            ("day", days.clone()),
            ("category", category.clone()),
            ("path", paths.clone()),
        ],
    );
    let per_row = Extents::PerRow {
        lower: &lower,
        upper: &upper,
    };
    // Each scale, the column it summarizes, and whether the marks have extents.
    let colors = strings(&["red", "blue"]);
    let cases: [(ConfiguredScale, &str, &ArrayRef, bool); 6] = [
        (LinearScale::unfitted((0.0, 1.0)), "x", &x, true),
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
            scale.summarize(values, extents).unwrap(),
            "{}",
            scale.scale_impl.scale_type()
        );
    }
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
    let df = table(
        &context(),
        vec![("x", x.clone()), ("lower", lower), ("gaps", gaps)],
    );
    let linear = LinearScale::unfitted((0.0, 1.0));
    let constant = summary_expr(&linear, col("x"), Some((lit(5.0), lit(3)))).unwrap();
    let extents = Extents::Constant {
        lower: 5.0,
        upper: 3.0,
    };
    assert_eq!(
        summary(df.clone(), constant).await.unwrap(),
        linear.summarize(&x, extents).unwrap()
    );
    // A missing value has no extent, but a value without an extent is an error.
    let per_row = summary_expr(&linear, col("x"), Some((col("lower"), lit(0.0)))).unwrap();
    assert!(summary(df.clone(), per_row).await.is_ok());
    let missing = summary_expr(&linear, col("x"), Some((col("gaps"), lit(0.0)))).unwrap();
    assert!(summary(df, missing).await.is_err());
}

#[test]
fn invalid_summaries_fail_at_planning() {
    let df = table(
        &context(),
        vec![("x", numbers(vec![1.0])), ("name", strings(&["a"]))],
    );
    let plan = |scale: ConfiguredScale, column: &str| {
        let expr = summary_expr(&scale, col(column), None).unwrap();
        df.clone().aggregate(vec![], vec![expr])
    };
    // Threshold scales have no summaries, and numeric and time scales need their kinds of values.
    let threshold = ThresholdScale::configured(vec![0.5], strings(&["low", "high"]));
    assert!(plan(threshold, "x").is_err());
    assert!(plan(LinearScale::unfitted((0.0, 1.0)), "name").is_err());
    assert!(plan(TimeScale::unfitted((0.0, 1.0), &DataType::Date32), "name").is_err());
}

#[tokio::test]
async fn fits_in_queries_equal_fits_in_rust() {
    let ctx = context();
    let x: Vec<f64> = vec![3.0, -4.0, 12.5, 7.0, 0.5, 9.0, 21.0, 2.0, 6.0, 14.0];
    let extents = vec![4.0, 1.0, 6.0, 0.0, 2.0, 3.0, 5.0, 1.5, 0.5, 2.5];
    let names = ["b", "a", "d", "c", "a", "b", "e", "d", "c", "a"];
    let groups = ["g", "f", "g", "f", "f", "g", "g", "f", "g", "f"];
    let days: ArrayRef = Arc::new(Date32Array::from(vec![10, 3, 40, 22, 8, 15, 31, 2, 19, 27]));
    let paths: ArrayRef = Arc::new(StructArray::new(
        Fields::from(vec![
            Field::new("group", DataType::Utf8, true),
            Field::new("item", DataType::Utf8, true),
        ]),
        vec![strings(&groups), strings(&names)],
        None,
    ));
    let df = table(
        &ctx,
        vec![
            ("x", numbers(x.clone())),
            ("extent", numbers(extents.clone())),
            ("day", days.clone()),
            ("name", strings(&names)),
            ("path", paths.clone()),
        ],
    );
    let colors = strings(&["red", "green", "blue"]);
    let order = strings(&["e", "a", "c"]);
    let explicit = DomainOptions::Categories(CategoryOptions {
        order: CategoryOrder::Explicit(order),
    });
    let per_row = Extents::PerRow {
        lower: &extents,
        upper: &extents,
    };
    // Each scale, the column it fits, whether its marks have extents, and its domain options.
    let (x_values, name_values) = (numbers(x.clone()), strings(&names));
    let cases: Vec<(ConfiguredScale, &str, &ArrayRef, bool, DomainOptions)> = vec![
        (
            LinearScale::unfitted((0.0, 200.0)),
            "x",
            &x_values,
            true,
            interval(Some(10.0)),
        ),
        (
            TimeScale::unfitted((0.0, 200.0), &DataType::Date32),
            "day",
            &days,
            true,
            interval(None),
        ),
        (
            NestedBandScale::unfitted((0.0, 200.0)),
            "path",
            &paths,
            false,
            categories(),
        ),
        (
            OrdinalScale::unfitted(colors.clone()),
            "name",
            &name_values,
            false,
            explicit,
        ),
        (
            QuantizeScale::unfitted(colors.clone()),
            "x",
            &x_values,
            false,
            interval(Some(5.0)),
        ),
        (
            QuantileScale::unfitted(colors),
            "x",
            &x_values,
            false,
            DomainOptions::Sample,
        ),
    ];
    for (scale, column, values, with_extents, options) in cases {
        let name = scale.scale_impl.scale_type();
        let (extent_exprs, extents) = match with_extents {
            true => (Some((col("extent"), col("extent"))), per_row),
            false => (None, Extents::None),
        };
        let actual = fitted_in_query(df.clone(), &scale, col(column), extent_exprs, &options)
            .await
            .unwrap();
        let summary = scale.summarize(values, extents).unwrap();
        let expected = scale.fit_domain(&options, &summary).unwrap().config;
        assert_same_config(&actual, &expected, name);
    }
}

/// Circles take their radii from a size scale, share an x scale with labels, and have bands for
/// their categories, all fitted and applied in one plan.
#[tokio::test]
async fn one_plan_goes_from_rows_to_pixels() {
    let ctx = context();
    let x = vec![2.0, 5.0, 9.0, 3.0, 12.0, 7.0, 1.0, 10.0];
    let size = vec![1.0, 4.0, 2.0, 8.0, 3.0, 6.0, 5.0, 7.0];
    let names = ["a", "b", "c", "b", "c", "a", "d", "b"];
    let points = table(
        &ctx,
        vec![
            (
                "id",
                Arc::new(Int64Array::from_iter_values(0..8)) as ArrayRef,
            ),
            ("category", strings(&names)),
            ("x", numbers(x.clone())),
            ("size", numbers(size.clone())),
        ],
    );
    let labels = table(&ctx, vec![("x", numbers(vec![-3.0, 15.0]))]);
    let fitted = |df: DataFrame, fit: Expr| -> Result<Expr> {
        let plan = df.aggregate(vec![], vec![fit.alias("config")])?;
        Ok(scalar_subquery(Arc::new(plan.into_unoptimized_plan())))
    };

    let size_scale = LinearScale::unfitted((2.0, 12.0));
    let size_summary = summary_expr(&size_scale, col("size"), None).unwrap();
    let size_fit = fit_expr(&size_scale, size_summary, &interval(None)).unwrap();
    let size_config = fitted(points.clone(), size_fit).unwrap();
    let radius = scale_expr(size_scale.scale_impl.as_ref(), col("size"), size_config).unwrap();
    let circles = points
        .select(vec![
            col("id"),
            col("category"),
            col("x"),
            radius.alias("r"),
        ])
        .unwrap();

    let x_scale = LinearScale::unfitted((0.0, 400.0));
    let x_rows = circles
        .clone()
        .select(vec![col("x"), col("r")])
        .unwrap()
        .union(
            labels
                .select(vec![col("x"), lit(0.0_f32).alias("r")])
                .unwrap(),
        )
        .unwrap();
    let x_summary = summary_expr(&x_scale, col("x"), Some((col("r"), col("r")))).unwrap();
    let x_fit = fit_expr(&x_scale, x_summary, &interval(Some(10.0))).unwrap();
    let x_config = fitted(x_rows, x_fit).unwrap();

    let y_scale = BandScale::unfitted((300.0, 0.0));
    let y_summary = summary_expr(&y_scale, col("category"), Some((col("r"), col("r")))).unwrap();
    let y_fit = fit_expr(&y_scale, y_summary, &categories()).unwrap();
    let y_config = fitted(circles.clone(), y_fit).unwrap();

    let marks = circles
        .select(vec![
            col("id"),
            scale_expr(x_scale.scale_impl.as_ref(), col("x"), x_config)
                .unwrap()
                .alias("px"),
            scale_expr(y_scale.scale_impl.as_ref(), col("category"), y_config)
                .unwrap()
                .alias("py"),
            col("r"),
        ])
        .unwrap()
        .sort(vec![col("id").sort(true, false)])
        .unwrap();
    let marks = collect(marks).await.unwrap();

    // The same scales fitted and applied in Rust.
    let none = Extents::None;
    let size_summary = size_scale.summarize(&numbers(size.clone()), none).unwrap();
    let size_fitted = size_scale
        .fit_domain(&interval(None), &size_summary)
        .unwrap();
    let radii = size_fitted.scale(&numbers(size)).unwrap();
    let radii_f64: Vec<f64> = radii
        .as_primitive::<Float32Type>()
        .values()
        .iter()
        .map(|r| *r as f64)
        .collect();
    let extents = Extents::PerRow {
        lower: &radii_f64,
        upper: &radii_f64,
    };
    let x_summary = x_scale
        .summarize(&numbers(x.clone()), extents)
        .unwrap()
        .merge(&x_scale.summarize(&numbers(vec![-3.0, 15.0]), none).unwrap())
        .unwrap();
    let x_fitted = x_scale
        .fit_domain(&interval(Some(10.0)), &x_summary)
        .unwrap();
    let y_summary = y_scale.summarize(&strings(&names), extents).unwrap();
    let y_fitted = y_scale.fit_domain(&categories(), &y_summary).unwrap();
    assert_eq!(
        marks.column(1).to_data(),
        x_fitted.scale(&numbers(x)).unwrap().to_data()
    );
    assert_eq!(
        marks.column(2).to_data(),
        y_fitted.scale(&strings(&names)).unwrap().to_data()
    );
    assert_eq!(marks.column(3).to_data(), radii.to_data());
}

#[tokio::test]
async fn facets_fit_and_apply_their_own_scales() {
    let ctx = context();
    let x = vec![1.0, 50.0, 3.0, 400.0, 7.0, 90.0, 2.0, 650.0, 5.0];
    let facets = vec![0, 1, 0, 2, 0, 1, 0, 2, 1];
    let df = table(
        &ctx,
        vec![
            (
                "id",
                Arc::new(Int64Array::from_iter_values(0..9)) as ArrayRef,
            ),
            ("facet", Arc::new(Int64Array::from(facets.clone()))),
            ("x", numbers(x.clone())),
        ],
    );
    let scale = LinearScale::unfitted((0.0, 100.0));
    let fit = fit_expr(
        &scale,
        summary_expr(&scale, col("x"), None).unwrap(),
        &interval(Some(10.0)),
    )
    .unwrap();
    let configs = df
        .clone()
        .aggregate(vec![col("facet")], vec![fit.alias("config")])
        .unwrap()
        .select(vec![col("facet").alias("f"), col("config")])
        .unwrap();
    let marks = df
        .join(configs, JoinType::Inner, &["facet"], &["f"], None)
        .unwrap()
        .select(vec![
            col("id"),
            scale_expr(scale.scale_impl.as_ref(), col("x"), col("config"))
                .unwrap()
                .alias("px"),
        ])
        .unwrap()
        .sort(vec![col("id").sort(true, false)])
        .unwrap();
    let marks = collect(marks).await.unwrap();
    let px = marks.column(1).as_primitive::<Float32Type>();
    for (row, (value, facet)) in x.iter().zip(&facets).enumerate() {
        let values: Vec<f64> = x
            .iter()
            .zip(&facets)
            .filter(|(_, f)| *f == facet)
            .map(|(v, _)| *v)
            .collect();
        let summary = scale.summarize(&numbers(values), Extents::None).unwrap();
        let fitted = scale
            .clone()
            .fit_domain(&interval(Some(10.0)), &summary)
            .unwrap();
        let expected = fitted.scale(&numbers(vec![*value])).unwrap();
        assert_eq!(
            px.value(row),
            expected.as_primitive::<Float32Type>().value(0),
            "row {row}"
        );
    }
}

#[tokio::test]
async fn domain_options_are_cast_to_the_fitted_domain_type() {
    let ctx = context();
    // An empty domain of integers for a linear scale, whose domain is f64.
    let empty: ArrayRef = Arc::new(Int64Array::from(vec![0, 10]));
    let options = DomainOptions::Interval(IntervalOptions {
        empty: Some(empty),
        ..Default::default()
    });
    let none = table(&ctx, vec![("x", numbers(vec![1.0]))])
        .filter(lit(false))
        .unwrap();
    let scale = LinearScale::unfitted((0.0, 100.0));
    let config = fitted_in_query(none, &scale, col("x"), None, &options)
        .await
        .unwrap();
    assert_eq!(config.domain.to_data(), numbers(vec![0.0, 10.0]).to_data());
    // Explicit categories of another string type than the data's.
    let order: ArrayRef = Arc::new(LargeStringArray::from(vec!["b", "a"]));
    let options = DomainOptions::Categories(CategoryOptions {
        order: CategoryOrder::Explicit(order),
    });
    let df = table(&ctx, vec![("name", strings(&["a", "b", "a"]))]);
    let band = BandScale::unfitted((0.0, 100.0));
    let config = fitted_in_query(df, &band, col("name"), None, &options)
        .await
        .unwrap();
    assert_eq!(config.domain.to_data(), strings(&["b", "a"]).to_data());
    // Time scales have no nice domains.
    let days = table(
        &ctx,
        vec![("day", Arc::new(Date32Array::from(vec![1, 9])) as ArrayRef)],
    );
    let time = TimeScale::unfitted((0.0, 100.0), &DataType::Date32);
    assert!(
        fitted_in_query(days, &time, col("day"), None, &interval(Some(10.0)))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn plans_with_fits_decode_without_registration() {
    let ctx = context();
    let values = LogicalPlanBuilder::values(vec![
        vec![lit(1.0_f64)],
        vec![lit(4.0_f64)],
        vec![lit(9.0_f64)],
    ])
    .unwrap()
    .build()
    .unwrap();
    let values = DataFrame::new(ctx.state(), values);
    let scale = LinearScale::unfitted((0.0, 100.0));
    let fit = fit_expr(
        &scale,
        summary_expr(&scale, col("column1"), None).unwrap(),
        &interval(Some(10.0)),
    )
    .unwrap();
    let config = values
        .clone()
        .aggregate(vec![], vec![fit.alias("config")])
        .unwrap()
        .into_unoptimized_plan();
    let px = scale_expr(
        scale.scale_impl.as_ref(),
        col("column1"),
        scalar_subquery(Arc::new(config)),
    )
    .unwrap();
    let plan = values
        .select(vec![px.alias("px")])
        .unwrap()
        .into_unoptimized_plan();
    let codec = ScaleExtensionCodec::default();
    let bytes = logical_plan_to_bytes_with_extension_codec(&plan, &codec).unwrap();
    let decoded =
        logical_plan_from_bytes_with_extension_codec(&bytes, &ctx.task_ctx(), &codec).unwrap();
    assert_eq!(decoded, plan);
    let batch = collect(DataFrame::new(ctx.state(), decoded)).await.unwrap();
    // The data from 1 to 9 has the nice domain from 1 to 9.
    let fitted = LinearScale::configured((1.0, 9.0), (0.0, 100.0));
    let expected = fitted.scale(&numbers(vec![1.0, 4.0, 9.0])).unwrap();
    assert_eq!(batch.column(0).to_data(), expected.to_data());
    // A configuration fitted in Rust is a literal.
    let literal = scale_expr(
        fitted.scale_impl.as_ref(),
        lit(4.0),
        config_expr(&fitted.config).unwrap(),
    )
    .unwrap();
    let batch = collect(ctx.read_empty().unwrap().select(vec![literal]).unwrap())
        .await
        .unwrap();
    assert_eq!(batch.column(0).as_primitive::<Float32Type>().value(0), 37.5);
}
