//! Physical-plan cache integration: coverage census probe and (later
//! phases) the cache-on/off equivalence suite.
//!
//! Phase 0 (plan: `scratch/physical-cache-avenger-integration-plan.md`):
//! evaluate a representative chart set on a context with the cache rule
//! installed and PROVE, before any avenger-chart wiring exists, that
//! (a) the fingerprinter covers real chart plans (`excluded_unsupported == 0`),
//! and (b) a second evaluation of an unchanged chart serves hits.

use std::sync::Arc;

use avenger_chart::{
    physical_cache::{
        CacheMetricsSnapshot, EvaluationCache, EvaluationCacheConfig, cached_session_context,
        install_physical_cache, physical_cache_from_ctx,
    },
    plot::CompiledPlot,
    prelude::*,
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
/// rule, via the blessed helper.
fn cached_ctx() -> (SessionContext, Arc<EvaluationCache>) {
    cached_session_context(EvaluationCacheConfig::default())
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

/// Evaluate the same chart shape on a PLAIN (uncached) context and return
/// its scene bytes — the cache-on/off equivalence baseline.
async fn plain_twin_scene<F, Fut>(
    build: F,
    chart_params: Option<IndexMap<String, ScalarValue>>,
) -> Vec<u8>
where
    F: FnOnce(SessionContext) -> Fut,
    Fut: std::future::Future<Output = (SessionContext, CompiledPlot)>,
{
    let (ctx, compiled) = build(SessionContext::new()).await;
    let evaluated = compiled
        .evaluate(&ctx, chart_params)
        .await
        .expect("plain twin evaluation");
    bincode::serialize(&evaluated.scene_graph).expect("scene bytes")
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
    plain_twin: Option<&[u8]>,
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
    if let Some(plain_scene) = plain_twin {
        assert_eq!(
            first_bytes, plain_scene,
            "census `{label}`: cached context must be scene-byte identical to an \
             uncached twin"
        );
    }
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
    let twin = plain_twin_scene(
        |ctx| async move {
            let compiled = scatter_param_filter(&ctx).await;
            (ctx, compiled)
        },
        Some(params(&[("min", 2.5)])),
    )
    .await;
    let (ctx, cache) = cached_ctx();
    let compiled = scatter_param_filter(&ctx).await;
    probe(
        "scatter_param_filter",
        &ctx,
        &cache,
        &compiled,
        Some(params(&[("min", 2.5)])),
        Some(&twin),
    )
    .await;
}

#[tokio::test]
async fn census_colored_scatter_legend() {
    let twin = plain_twin_scene(
        |ctx| async move {
            let compiled = colored_scatter_legend(&ctx).await;
            (ctx, compiled)
        },
        None,
    )
    .await;
    let (ctx, cache) = cached_ctx();
    let compiled = colored_scatter_legend(&ctx).await;
    probe(
        "colored_scatter_legend",
        &ctx,
        &cache,
        &compiled,
        None,
        Some(&twin),
    )
    .await;
}

#[tokio::test]
async fn census_nested_band_bar() {
    let twin = plain_twin_scene(
        |ctx| async move {
            let compiled = nested_band_bar(&ctx).await;
            (ctx, compiled)
        },
        None,
    )
    .await;
    let (ctx, cache) = cached_ctx();
    let compiled = nested_band_bar(&ctx).await;
    probe(
        "nested_band_bar",
        &ctx,
        &cache,
        &compiled,
        None,
        Some(&twin),
    )
    .await;
}

#[tokio::test]
async fn census_facet_wrap_aggregate() {
    let twin = plain_twin_scene(
        |ctx| async move {
            let compiled = facet_wrap_aggregate(&ctx).await;
            (ctx, compiled)
        },
        Some(params(&[("min", 0.5)])),
    )
    .await;
    let (ctx, cache) = cached_ctx();
    let compiled = facet_wrap_aggregate(&ctx).await;
    probe(
        "facet_wrap_aggregate",
        &ctx,
        &cache,
        &compiled,
        Some(params(&[("min", 0.5)])),
        Some(&twin),
    )
    .await;
}

#[tokio::test]
async fn census_join_aggregate_faceted() {
    let twin = plain_twin_scene(
        |ctx| async move {
            let compiled = join_aggregate_faceted(&ctx).await;
            (ctx, compiled)
        },
        None,
    )
    .await;
    let (ctx, cache) = cached_ctx();
    let compiled = join_aggregate_faceted(&ctx).await;
    probe(
        "join_aggregate_faceted",
        &ctx,
        &cache,
        &compiled,
        None,
        Some(&twin),
    )
    .await;
}

#[tokio::test]
async fn census_temporal_scatter() {
    let twin = plain_twin_scene(
        |ctx| async move {
            let compiled = temporal_scatter(&ctx).await;
            (ctx, compiled)
        },
        None,
    )
    .await;
    let (ctx, cache) = cached_ctx();
    let compiled = temporal_scatter(&ctx).await;
    probe(
        "temporal_scatter",
        &ctx,
        &cache,
        &compiled,
        None,
        Some(&twin),
    )
    .await;
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

    // Twin: the same artifact on a fresh PLAIN context.
    let plain_ctx = SessionContext::new();
    let plain_eval = decoded
        .evaluate(&plain_ctx, Some(params(&[("min", 2.5)])))
        .await
        .expect("plain baked evaluation");
    let twin = bincode::serialize(&plain_eval.scene_graph).expect("scene bytes");

    // Evaluate the self-contained artifact on a fresh CACHED context.
    let (ctx, cache) = cached_ctx();
    probe(
        "baked_chart",
        &ctx,
        &cache,
        &decoded,
        Some(params(&[("min", 2.5)])),
        Some(&twin),
    )
    .await;
}

// ---------------------------------------------------------------------------
// Phase 1: helper wiring + kill switch
// ---------------------------------------------------------------------------

#[tokio::test]
async fn helper_context_serves_hits_and_is_discoverable() {
    let (ctx, cache) = cached_session_context(EvaluationCacheConfig::default());
    let found = physical_cache_from_ctx(&ctx).expect("cache discoverable from ctx");
    assert!(
        Arc::ptr_eq(&found, &cache),
        "extension returns the same cache"
    );

    let compiled = colored_scatter_legend(&ctx).await;
    compiled.evaluate(&ctx, None).await.expect("eval 1");
    compiled.evaluate(&ctx, None).await.expect("eval 2");
    compiled.evaluate(&ctx, None).await.expect("eval 3");
    assert!(
        cache.metrics().hits > 0,
        "helper-built context must serve hits: {:?}",
        cache.metrics()
    );
}

#[tokio::test]
async fn install_composes_with_existing_builder() {
    // Hosts with their own builder use the primitive; everything still works.
    let (builder, cache) = install_physical_cache(
        SessionStateBuilder::new().with_default_features(),
        EvaluationCacheConfig::default(),
    );
    let ctx = SessionContext::new_with_state(builder.build());
    let compiled = colored_scatter_legend(&ctx).await;
    for _ in 0..3 {
        compiled.evaluate(&ctx, None).await.expect("evaluate");
    }
    assert!(cache.metrics().hits > 0, "{:?}", cache.metrics());
}

#[tokio::test]
async fn disabled_cache_records_nothing_and_is_undiscoverable() {
    // The env kill switch cannot be safely toggled in-process (tests run
    // threaded); this exercises the same disabled path the switch takes.
    let ctx = SessionContext::new();
    assert!(
        physical_cache_from_ctx(&ctx).is_none(),
        "plain context has no cache extension"
    );
    let compiled = colored_scatter_legend(&ctx).await;
    compiled.evaluate(&ctx, None).await.expect("evaluate");
}

// ---------------------------------------------------------------------------
// Phase 2: behavior suite (param reuse, store invalidation, preview)
// ---------------------------------------------------------------------------

/// Param sweep: repeated and novel values, every scene checked against an
/// uncached twin, reuse strictly growing across the sweep.
#[tokio::test]
async fn param_change_reuses_upstream() {
    let (cached_ctx, cache) = cached_ctx();
    let plain_ctx = SessionContext::new();
    let cached = scatter_param_filter(&cached_ctx).await;
    let plain = scatter_param_filter(&plain_ctx).await;

    // a, a (warm), b, c, b (repeat)
    let sweep = [2.5, 2.5, 3.5, 4.5, 3.5];
    let mut hits_at = Vec::new();
    for min in sweep {
        let p = Some(params(&[("min", min)]));
        let cached_eval = cached
            .evaluate(&cached_ctx, p.clone())
            .await
            .expect("cached evaluation");
        let plain_eval = plain
            .evaluate(&plain_ctx, p)
            .await
            .expect("plain evaluation");
        assert_eq!(
            bincode::serialize(&cached_eval.scene_graph).expect("scene"),
            bincode::serialize(&plain_eval.scene_graph).expect("scene"),
            "param sweep: cached and uncached scenes diverge at min={min}"
        );
        hits_at.push(cache.metrics().hits);
    }
    let m = cache.metrics();
    println!("param sweep hits: {hits_at:?} | {m:?}");

    assert!(
        hits_at[2] > hits_at[1],
        "first NOVEL literal must reuse param-independent upstream subtrees: {hits_at:?}"
    );
    assert!(
        hits_at[4] > hits_at[3],
        "repeated literal must reuse its own subtrees too: {hits_at:?}"
    );
    assert_eq!(
        m.admitted_writes,
        m.committed_writes + m.discarded_writes,
        "all admitted writes resolved: {m:?}"
    );
    assert!(m.entries > 0);
}

fn threshold_store_batch() -> RecordBatch {
    record_batch(
        vec![
            Field::new("id", DataType::Utf8, false),
            Field::new("lo", DataType::Float64, false),
            Field::new("hi", DataType::Float64, false),
        ],
        vec![
            Arc::new(StringArray::from(vec!["active"])) as ArrayRef,
            Arc::new(Float64Array::from(vec![3.0])) as ArrayRef,
            Arc::new(Float64Array::from(vec![6.0])) as ArrayRef,
        ],
    )
}

/// Store-bearing chart: two mark groups, one reading a Shared store. A
/// store patch must invalidate store-dependent results (visible scene
/// change, no false hit) while the store-independent work keeps reusing.
#[tokio::test]
async fn store_change_invalidates_without_false_hits() {
    async fn store_chart(ctx: &SessionContext) -> CompiledPlot {
        ctx.register_batch("sales", sales_batch())
            .expect("register sales");
        let data = ctx
            .sql(
                "SELECT * FROM (SELECT region, SUM(value) AS total \
                 FROM sales GROUP BY region) q ORDER BY region",
            )
            .await
            .expect("sales query");
        Plot::<Cartesian>::new()
            .data(data)
            .add_store(
                Store::from_record_batch("threshold_band", threshold_store_batch())
                    .primary_key(["id"])
                    .sharing(CoordinationScope::Shared),
            )
            .mark(MarkGroup::new().mark(Symbol::new().x(col("total")).y(col("total")).size(64.0)))
            .mark(
                MarkGroup::new()
                    .data_store(StoreData::new("threshold_band"))
                    .mark(
                        Rect::new()
                            .x(lit(2.0))
                            .x2(lit(4.0))
                            .y(col("lo"))
                            .y2(col("hi"))
                            .fill("rgba(37, 99, 235, 0.18)"),
                    ),
            )
            .compile(ctx)
            .await
            .expect("compile store chart")
    }

    fn band_row(lo: f64, hi: f64) -> IndexMap<String, ScalarValue> {
        let mut row = IndexMap::new();
        row.insert(
            "id".to_string(),
            ScalarValue::Utf8(Some("active".to_string())),
        );
        row.insert("lo".to_string(), ScalarValue::Float64(Some(lo)));
        row.insert("hi".to_string(), ScalarValue::Float64(Some(hi)));
        row
    }

    let (cached_ctx, cache) = cached_ctx();
    let cached_compiled = Arc::new(store_chart(&cached_ctx).await);
    let mut cached_session = cached_compiled.instantiate(Arc::new(cached_ctx));
    let plain_ctx = SessionContext::new();
    let plain_compiled = Arc::new(store_chart(&plain_ctx).await);
    let mut plain_session = plain_compiled.instantiate(Arc::new(plain_ctx));

    async fn scene(session: &mut PlotSession) -> Vec<u8> {
        let evaluated = session
            .evaluate(EvaluationRequest::default())
            .await
            .expect("session evaluation");
        bincode::serialize(&evaluated.scene_graph).expect("scene bytes")
    }

    // Warm: two evaluations, scenes equal to the uncached twin throughout.
    let scene_a_plain = scene(&mut plain_session).await;
    let scene_a1 = scene(&mut cached_session).await;
    let scene_a2 = scene(&mut cached_session).await;
    assert_eq!(
        scene_a1, scene_a_plain,
        "pre-patch scene equal to uncached twin"
    );
    assert_eq!(scene_a1, scene_a2, "repeated evaluation stable");
    let hits_before_patch = cache.metrics().hits;

    // Patch the store on BOTH sessions (move the band, visibly).
    let owner_path = cached_session
        .store_owner_path_for_diagnostics("threshold_band", &std::collections::HashMap::new())
        .expect("store owner path");
    let patch = |lo: f64, hi: f64| {
        vec![avenger_chart::plot::ScopedStoreAssignment {
            store_name: "threshold_band".to_string(),
            owner_path: owner_path.clone(),
            replace_scoped_values: true,
            update: avenger_chart::plot::StoreStateUpdate::UpsertRows {
                rows: vec![band_row(lo, hi)],
            },
        }]
    };
    assert!(
        cached_session
            .apply_scoped_store_patch(patch(1.0, 8.0))
            .expect("patch")
    );
    assert!(
        plain_session
            .apply_scoped_store_patch(patch(1.0, 8.0))
            .expect("patch")
    );

    let scene_b_plain = scene(&mut plain_session).await;
    let scene_b = scene(&mut cached_session).await;
    assert_eq!(
        scene_b, scene_b_plain,
        "post-patch scene equal to uncached twin (no false hit on store data)"
    );
    assert_ne!(
        scene_b, scene_a1,
        "the store change is visible in the scene"
    );
    let m = cache.metrics();
    assert!(
        m.hits > hits_before_patch,
        "store-independent subtrees keep reusing after the patch: {m:?}"
    );
}

/// Preview-mode evaluations against a warmed session: pins today's metric
/// interplay ahead of the Phase 4 observe-only policy, and checks exact
/// evaluations stay correct around a preview.
#[tokio::test]
async fn preview_then_full_evaluation() {
    let (cached_ctx, cache) = cached_ctx();
    let session_ctx = Arc::new(cached_ctx);
    let compiled = Arc::new(colored_scatter_legend(session_ctx.as_ref()).await);
    let mut session = compiled.instantiate(Arc::clone(&session_ctx));

    let plain_ctx = SessionContext::new();
    let plain_compiled = Arc::new(colored_scatter_legend(&plain_ctx).await);
    let mut plain_session = plain_compiled.instantiate(Arc::new(plain_ctx));

    async fn exact_scene(session: &mut PlotSession) -> Vec<u8> {
        let evaluated = session
            .evaluate(EvaluationRequest::default())
            .await
            .expect("exact evaluation");
        bincode::serialize(&evaluated.scene_graph).expect("scene bytes")
    }

    // Warm with two exact evaluations (populates the layout profile).
    let plain_scene = exact_scene(&mut plain_session).await;
    let warm1 = exact_scene(&mut session).await;
    let warm2 = exact_scene(&mut session).await;
    assert_eq!(warm1, plain_scene);
    assert_eq!(warm1, warm2);
    let before_preview = cache.metrics();

    // A preview-mode evaluation (approximate by design: no scene assert).
    session
        .evaluate(EvaluationRequest::default().preview())
        .await
        .expect("preview evaluation");
    let after_preview = cache.metrics();
    println!(
        "preview delta: hits +{} misses +{} admitted +{} committed +{}",
        after_preview.hits - before_preview.hits,
        after_preview.misses - before_preview.misses,
        after_preview.admitted_writes - before_preview.admitted_writes,
        after_preview.committed_writes - before_preview.committed_writes,
    );

    assert_eq!(
        after_preview.admitted_writes, before_preview.admitted_writes,
        "preview evaluations must not admit writes (observe-only policy)"
    );

    // Exact evaluation after the preview: still correct, and admission is
    // enabled again (the guard restored the setting).
    let after = exact_scene(&mut session).await;
    assert_eq!(
        after, plain_scene,
        "exact evaluation correct after a preview"
    );

    // A NOVEL chart shape after the preview proves writes re-enabled: it
    // must be able to warm up and admit again.
    let novel_before = cache.metrics().admitted_writes;
    let novel = scatter_param_filter(session_ctx.as_ref()).await;
    for _ in 0..2 {
        novel
            .evaluate(session_ctx.as_ref(), Some(params(&[("min", 2.5)])))
            .await
            .expect("novel evaluation");
    }
    assert!(
        cache.metrics().admitted_writes > novel_before,
        "admission works again after the preview guard dropped"
    );
}

/// The per-evaluation cache delta lands in `EvaluationMetrics`.
#[tokio::test]
async fn metrics_delta_reported_per_evaluation() {
    let (cached_ctx, _cache) = cached_ctx();
    let session_ctx = Arc::new(cached_ctx);
    let compiled = Arc::new(colored_scatter_legend(session_ctx.as_ref()).await);
    let mut session = compiled.instantiate(Arc::clone(&session_ctx));

    let (_evaluated, first) = session
        .evaluate_with_metrics(EvaluationRequest::default())
        .await
        .expect("evaluation 1");
    let delta1 = first
        .physical_cache
        .expect("delta present on cached context");
    assert!(delta1.misses > 0, "cold evaluation misses: {delta1:?}");
    assert_eq!(delta1.hits, 0, "cold evaluation has no hits: {delta1:?}");

    session
        .evaluate(EvaluationRequest::default())
        .await
        .expect("evaluation 2");
    let (_evaluated, third) = session
        .evaluate_with_metrics(EvaluationRequest::default())
        .await
        .expect("evaluation 3");
    let delta3 = third.physical_cache.expect("delta present");
    assert!(delta3.hits > 0, "warmed evaluation serves hits: {delta3:?}");

    // A cacheless session reports no delta.
    let plain_ctx = Arc::new(SessionContext::new());
    let plain_compiled = Arc::new(colored_scatter_legend(plain_ctx.as_ref()).await);
    let mut plain_session = plain_compiled.instantiate(plain_ctx);
    let (_evaluated, metrics) = plain_session
        .evaluate_with_metrics(EvaluationRequest::default())
        .await
        .expect("plain evaluation");
    assert!(metrics.physical_cache.is_none(), "no cache, no delta");
}
