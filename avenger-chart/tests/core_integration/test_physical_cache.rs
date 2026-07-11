//! Physical-plan cache integration: coverage census probe and (later
//! phases) the cache-on/off equivalence suite.
//!
//! Phase 0 (plan: `scratch/physical-cache-avenger-integration-plan.md`):
//! evaluate a representative chart set on a context with the cache rule
//! installed and PROVE, before any avenger-chart wiring exists, that
//! (a) the fingerprinter covers real chart plans (`excluded_unsupported == 0`),
//! and (b) a second evaluation of an unchanged chart serves hits.

use std::sync::Arc;

use avenger_chart::{plot::CompiledPlot, prelude::*};
use avenger_datafusion_cache::{
    CacheMetricsSnapshot, EvaluationCache, EvaluationCacheConfig, EvaluationCachePlanner,
};
use datafusion::{
    arrow::{
        array::{ArrayRef, Float64Array, StringArray, TimestampMillisecondArray},
        datatypes::{DataType, Field, Schema, TimeUnit},
        record_batch::RecordBatch,
    },
    common::ScalarValue,
    dataframe::DataFrame,
    execution::session_state::SessionStateBuilder,
    prelude::{SessionContext, col, lit},
};
use indexmap::IndexMap;

// ---------------------------------------------------------------------------
// Context + fixtures
// ---------------------------------------------------------------------------

/// A context with the cache rule installed as the LAST physical optimizer
/// rule — the Phase 0 probe wires this by hand; Phase 1 replaces the body
/// with the blessed helper.
fn cached_ctx() -> (SessionContext, Arc<EvaluationCache>) {
    let cache = EvaluationCache::new(EvaluationCacheConfig::default());
    let planner = EvaluationCachePlanner::new(Arc::clone(&cache));
    let state = SessionStateBuilder::new()
        .with_default_features()
        .with_physical_optimizer_rule(Arc::new(planner))
        .build();
    (SessionContext::new_with_state(state), cache)
}

fn record_batch(fields: Vec<Field>, columns: Vec<ArrayRef>) -> RecordBatch {
    RecordBatch::try_new(Arc::new(Schema::new(fields)), columns).expect("record batch")
}

fn sales_batch() -> RecordBatch {
    record_batch(
        vec![
            Field::new("region", DataType::Utf8, false),
            Field::new("value", DataType::Float64, false),
        ],
        vec![
            Arc::new(StringArray::from(vec!["EU", "EU", "NA", "NA", "APAC"])) as ArrayRef,
            Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0])) as ArrayRef,
        ],
    )
}

fn segmented_sales_batch() -> RecordBatch {
    record_batch(
        vec![
            Field::new("region", DataType::Utf8, false),
            Field::new("segment", DataType::Utf8, false),
            Field::new("value", DataType::Float64, false),
        ],
        vec![
            Arc::new(StringArray::from(vec!["EU", "EU", "EU", "NA", "NA", "NA"])) as ArrayRef,
            Arc::new(StringArray::from(vec![
                "core", "core", "edge", "core", "edge", "edge",
            ])) as ArrayRef,
            Arc::new(Float64Array::from(vec![1.0, 2.0, 4.0, 8.0, 16.0, 32.0])) as ArrayRef,
        ],
    )
}

fn grouped_bar_df(ctx: &SessionContext) -> DataFrame {
    let batch = record_batch(
        vec![
            Field::new("quarter", DataType::Utf8, false),
            Field::new("team", DataType::Utf8, false),
            Field::new("value", DataType::Float64, false),
        ],
        vec![
            Arc::new(StringArray::from(vec![
                "Q1", "Q1", "Q1", "Q2", "Q2", "Q3", "Q3", "Q3",
            ])) as ArrayRef,
            Arc::new(StringArray::from(vec![
                "North", "South", "East", "North", "East", "North", "South", "East",
            ])) as ArrayRef,
            Arc::new(Float64Array::from(vec![
                42.0, 30.0, 34.0, 47.0, 38.0, 51.0, 39.0, 44.0,
            ])) as ArrayRef,
        ],
    );
    ctx.read_batch(batch).expect("dataframe")
}

fn revenue_mix_df(ctx: &SessionContext) -> DataFrame {
    let batch = record_batch(
        vec![
            Field::new("department", DataType::Utf8, false),
            Field::new("segment", DataType::Utf8, false),
            Field::new("value", DataType::Float64, false),
        ],
        vec![
            Arc::new(StringArray::from(vec![
                "Sales",
                "Sales",
                "Sales",
                "Engineering",
                "Engineering",
                "Engineering",
            ])) as ArrayRef,
            Arc::new(StringArray::from(vec![
                "Core", "Cloud", "Support", "Core", "Cloud", "Support",
            ])) as ArrayRef,
            Arc::new(Float64Array::from(vec![40.0, 25.0, 15.0, 30.0, 45.0, 20.0])) as ArrayRef,
        ],
    );
    ctx.read_batch(batch).expect("dataframe")
}

fn temporal_df(ctx: &SessionContext) -> DataFrame {
    let base_ms: i64 = 1_700_000_000_000;
    let day_ms: i64 = 24 * 60 * 60 * 1000;
    let timestamps: Vec<i64> = (0..30).map(|i| base_ms + i * day_ms).collect();
    let values: Vec<f64> = (0..30).map(|i| ((i * 7) % 13) as f64 + 1.0).collect();
    let batch = record_batch(
        vec![
            Field::new(
                "ts",
                DataType::Timestamp(TimeUnit::Millisecond, None),
                false,
            ),
            Field::new("value", DataType::Float64, false),
        ],
        vec![
            Arc::new(TimestampMillisecondArray::from(timestamps)) as ArrayRef,
            Arc::new(Float64Array::from(values)) as ArrayRef,
        ],
    );
    ctx.read_batch(batch).expect("dataframe")
}

fn params(pairs: &[(&str, f64)]) -> IndexMap<String, ScalarValue> {
    pairs
        .iter()
        .map(|(name, value)| (name.to_string(), ScalarValue::Float64(Some(*value))))
        .collect()
}

// ---------------------------------------------------------------------------
// Representative charts (compiled against the given context)
// ---------------------------------------------------------------------------

/// Unfaceted scatter over a `$min`-filtered aggregate (scale UDF
/// projections; the bake-census flagship shape).
async fn scatter_param_filter(ctx: &SessionContext) -> CompiledPlot {
    ctx.register_batch("sales", sales_batch())
        .expect("register sales");
    let data = ctx
        .sql(
            "SELECT * FROM (SELECT region, SUM(value) AS total \
             FROM sales GROUP BY region) q WHERE total > $min ORDER BY region",
        )
        .await
        .expect("sales query");
    Plot::<Cartesian>::new()
        .data(data)
        .mark(Symbol::new().x(col("total")).y(col("total")).size(64.0))
        .compile(ctx)
        .await
        .expect("compile scatter_param_filter")
}

/// Scatter with a discrete fill scale and legend (legend/domain queries).
async fn colored_scatter_legend(ctx: &SessionContext) -> CompiledPlot {
    let data = ctx.read_batch(sales_batch()).expect("dataframe");
    Plot::<Cartesian>::new()
        .data(data)
        .mark(
            Symbol::new()
                .x(col("value"))
                .y(col("value"))
                .fill_with(col("region"), |c| c.legend(|l| l.title("Region")))
                .size(80.0),
        )
        .compile(ctx)
        .await
        .expect("compile colored_scatter_legend")
}

/// Grouped bar on a NESTED band scale (the `named_struct` component-expr
/// shape from the DF54 `push_down_leaf_projections` fix, b35412b99).
async fn nested_band_bar(ctx: &SessionContext) -> CompiledPlot {
    Plot::<Cartesian>::new()
        .data(grouped_bar_df(ctx))
        .legend("fill", |legend| legend.title("Team"))
        .mark(
            Rect::new()
                .x_with(nested(["quarter", "team"]), |x| {
                    x.axis(|a| a.title("Quarter").grid(false))
                        .level(0, |l| l.padding_inner(0.45).padding_outer(0.15))
                        .level(1, |l| {
                            l.nest_scope(NestScope::Shared)
                                .padding_inner(0.08)
                                .axis(|a| a.visible(false))
                        })
                })
                .x2_with(col(":x"), |x| x.band(1.0))
                .y_with(lit(0.0), |y| {
                    y.scale(|s| s.domain((0.0, 60.0)))
                        .axis(|a| a.title("Value").grid(true))
                })
                .y2(col("value"))
                .fill_with(col("team"), |fill| fill)
                .stroke("#ffffff")
                .stroke_width(1.0),
        )
        .compile(ctx)
        .await
        .expect("compile nested_band_bar")
}

/// Facet wrap with a live per-cell aggregate chain (the workload the bake
/// layer deliberately leaves live).
async fn facet_wrap_aggregate(ctx: &SessionContext) -> CompiledPlot {
    ctx.register_batch("segmented_sales", segmented_sales_batch())
        .expect("register segmented_sales");
    let data = ctx
        .sql("SELECT * FROM segmented_sales WHERE value > $min")
        .await
        .expect("segmented query");
    let leaf = Plot::<Cartesian>::new().mark(MarkGroup::new().transform(
        Aggregate::new().sum("total", col("value")),
        |group, aggregate| {
            group.mark(
                Symbol::new()
                    .x(aggregate.output("total"))
                    .y(aggregate.output("total"))
                    .size(72.0),
            )
        },
    ));
    Plot::<FacetWrap>::new()
        .canvas_size(520.0, 260.0)
        .data(data)
        .mark(Subplot::new(leaf).wrap_with(col("region"), |c| c.columns(2)))
        .compile(ctx)
        .await
        .expect("compile facet_wrap_aggregate")
}

/// Faceted JoinAggregate (window-rewrite transform) percent-of-group bars.
async fn join_aggregate_faceted(ctx: &SessionContext) -> CompiledPlot {
    let leaf = Plot::<Cartesian>::new().mark(
        Rect::new().transform_shared_no_output(
            JoinAggregate::new()
                .group_by([col("department")])
                .sum("department_total", col("value")),
            |mark| {
                mark.x_with(col("segment"), |c| {
                    c.scale_with::<Band>(|s| {
                        s.domain_discrete(vec![lit("Cloud"), lit("Core"), lit("Support")])
                    })
                    .axis(|a| a.title("Segment").grid(false))
                })
                .x2_with(col(":x"), |c| c.band(1.0))
                .y_with(lit(0.0), |c| {
                    c.scale_with::<Linear>(|s| s.domain_interval(lit(0.0), lit(0.7)))
                        .axis(|a| a.title("Share of department"))
                })
                .y2(col("value") / col("department_total"))
                .fill_with(col("segment"), |c| c)
                .stroke("#ffffff")
                .stroke_width(1.0)
            },
        ),
    );
    Plot::<FacetColumn>::new()
        .canvas_size(700.0, 360.0)
        .data(revenue_mix_df(ctx))
        .mark(Subplot::new(leaf).column(col("department")))
        .compile(ctx)
        .await
        .expect("compile join_aggregate_faceted")
}

/// Temporal x axis (timestamp scale + datetime tick machinery).
async fn temporal_scatter(ctx: &SessionContext) -> CompiledPlot {
    Plot::<Cartesian>::new()
        .data(temporal_df(ctx))
        .mark(Symbol::new().x(col("ts")).y(col("value")).size(36.0))
        .compile(ctx)
        .await
        .expect("compile temporal_scatter")
}

// ---------------------------------------------------------------------------
// Phase 0 probe
// ---------------------------------------------------------------------------

/// Evaluate `compiled` three times on a cached context; assert full
/// fingerprint coverage, scene-byte stability, and cached reuse by the
/// third evaluation. (Charts whose compile-time queries overlap their
/// evaluation queries hit on the second evaluation already; charts whose
/// compile pass runs different query shapes — the temporal axis is one —
/// need evaluation 2 to admit and evaluation 3 to hit, with the default
/// seen-twice admission policy.)
async fn probe(
    label: &str,
    ctx: &SessionContext,
    cache: &Arc<EvaluationCache>,
    compiled: &CompiledPlot,
    chart_params: Option<IndexMap<String, ScalarValue>>,
) -> (CacheMetricsSnapshot, CacheMetricsSnapshot) {
    let first = compiled
        .evaluate(ctx, chart_params.clone())
        .await
        .unwrap_or_else(|err| panic!("census `{label}`: first evaluation failed: {err}"));
    assert!(first.scene_graph.width > 0.0);
    let m1 = cache.metrics();

    let second = compiled
        .evaluate(ctx, chart_params.clone())
        .await
        .unwrap_or_else(|err| panic!("census `{label}`: second evaluation failed: {err}"));
    let m2 = cache.metrics();

    let third = compiled
        .evaluate(ctx, chart_params)
        .await
        .unwrap_or_else(|err| panic!("census `{label}`: third evaluation failed: {err}"));
    let m3 = cache.metrics();

    let first_bytes = bincode::serialize(&first.scene_graph).expect("scene bytes");
    assert_eq!(
        first_bytes,
        bincode::serialize(&second.scene_graph).expect("scene bytes"),
        "census `{label}`: repeated evaluation must be scene-byte identical"
    );
    assert_eq!(
        first_bytes,
        bincode::serialize(&third.scene_graph).expect("scene bytes"),
        "census `{label}`: cached (hit-serving) evaluation must be scene-byte identical"
    );

    println!(
        "census `{label}`: run1 misses={} hits={} | run2 hits={} | run3 hits={} | \
         admitted={} committed={} entries={} bytes={} | excl dyn={} vol={} unb={} unsup={}",
        m1.misses,
        m1.hits,
        m2.hits,
        m3.hits,
        m3.admitted_writes,
        m3.committed_writes,
        m3.entries,
        m3.bytes,
        m3.excluded_dynamic,
        m3.excluded_volatile,
        m3.excluded_unbounded,
        m3.excluded_unsupported,
    );

    assert_eq!(
        m3.excluded_unsupported, 0,
        "census `{label}`: fingerprinter must cover every chart plan node: {m3:?}"
    );
    assert_eq!(
        m3.excluded_volatile, 0,
        "census `{label}`: no volatile expressions in this chart set: {m3:?}"
    );
    assert!(
        m3.hits > 0,
        "census `{label}`: repeated evaluations must reuse cached results by \
         the third run: {m3:?}"
    );
    (m1, m3)
}

#[tokio::test]
async fn census_scatter_param_filter() {
    let (ctx, cache) = cached_ctx();
    let compiled = scatter_param_filter(&ctx).await;
    probe(
        "scatter_param_filter",
        &ctx,
        &cache,
        &compiled,
        Some(params(&[("min", 2.5)])),
    )
    .await;
}

#[tokio::test]
async fn census_colored_scatter_legend() {
    let (ctx, cache) = cached_ctx();
    let compiled = colored_scatter_legend(&ctx).await;
    probe("colored_scatter_legend", &ctx, &cache, &compiled, None).await;
}

#[tokio::test]
async fn census_nested_band_bar() {
    let (ctx, cache) = cached_ctx();
    let compiled = nested_band_bar(&ctx).await;
    probe("nested_band_bar", &ctx, &cache, &compiled, None).await;
}

#[tokio::test]
async fn census_facet_wrap_aggregate() {
    let (ctx, cache) = cached_ctx();
    let compiled = facet_wrap_aggregate(&ctx).await;
    probe(
        "facet_wrap_aggregate",
        &ctx,
        &cache,
        &compiled,
        Some(params(&[("min", 0.5)])),
    )
    .await;
}

#[tokio::test]
async fn census_join_aggregate_faceted() {
    let (ctx, cache) = cached_ctx();
    let compiled = join_aggregate_faceted(&ctx).await;
    probe("join_aggregate_faceted", &ctx, &cache, &compiled, None).await;
}

#[tokio::test]
async fn census_temporal_scatter() {
    let (ctx, cache) = cached_ctx();
    let compiled = temporal_scatter(&ctx).await;
    probe("temporal_scatter", &ctx, &cache, &compiled, None).await;
}

/// A BAKED chart evaluated on a cached context: the two layers composing.
/// Baked `MemTable` leaves version through the decode-memoized content
/// hash; the residual param-dependent work is what the cache reuses.
#[tokio::test]
async fn census_baked_chart_on_cached_context() {
    // Bake on a plain server context.
    let server_ctx = SessionContext::new();
    let compiled = scatter_param_filter(&server_ctx).await;
    let (baked, _report) = compiled
        .bake(&server_ctx, &BakePolicy::default())
        .await
        .expect("bake");
    let encoded = bincode::serialize(&baked).expect("serialize baked");
    let decoded: CompiledPlot = bincode::deserialize(&encoded).expect("deserialize baked");

    // Evaluate the self-contained artifact on a fresh CACHED context.
    let (ctx, cache) = cached_ctx();
    probe(
        "baked_chart",
        &ctx,
        &cache,
        &decoded,
        Some(params(&[("min", 2.5)])),
    )
    .await;
}
