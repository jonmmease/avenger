//! Staleness probes and measurements for the PartitionSlotCache /
//! ScaleDomainCache dissolution campaign
//! (`scratch/slot-domain-cache-dissolution-plan.md`).
//!
//! The mutable-data-behind-a-stable-plan path probed here is `INSERT INTO`
//! a registered `MemTable` between PlotSession evaluations: compiled plots
//! capture provider `Arc`s at compile time, so inserts are visible to
//! executions, invisible to plan-`Debug` cache keys, and invisible to
//! `data_ptr` pointer identities. The physical result cache handles this
//! correctly by construction (content-hashed memory leaves); these session
//! memos sit ABOVE it and can serve stale artifacts.

use std::sync::Arc;

use avenger_chart::{
    physical_cache::{EvaluationCacheConfig, cached_session_context},
    plot::{CompiledPlot, EvaluationRequest},
    prelude::*,
};
use datafusion::{
    arrow::{
        array::{ArrayRef, Float64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    common::ScalarValue,
    prelude::{SessionContext, col},
};
use indexmap::IndexMap;

fn events_batch(regions: &[&str], values: &[f64]) -> RecordBatch {
    RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("region", DataType::Utf8, false),
            Field::new("value", DataType::Float64, false),
        ])),
        vec![
            Arc::new(StringArray::from(regions.to_vec())) as ArrayRef,
            Arc::new(Float64Array::from(values.to_vec())) as ArrayRef,
        ],
    )
    .expect("events batch")
}

async fn faceted_chart(ctx: &SessionContext) -> CompiledPlot {
    let data = ctx.sql("SELECT * FROM events").await.expect("events query");
    let leaf =
        Plot::<Cartesian>::new().mark(Symbol::new().x(col("value")).y(col("value")).size(48.0));
    Plot::<FacetColumn>::new()
        .canvas_size(640.0, 240.0)
        .data(data)
        .mark(Subplot::new(leaf).column(col("region")))
        .compile(ctx)
        .await
        .expect("compile faceted chart")
}

async fn scene_via_session(session: &mut PlotSession) -> Vec<u8> {
    let evaluated = session
        .evaluate(EvaluationRequest::default())
        .await
        .expect("session evaluation");
    bincode::serialize(&evaluated.scene_graph).expect("scene bytes")
}

/// Fresh-cache truth: a direct evaluation on the same context (per-build
/// slot cache, no session artifacts).
async fn scene_fresh(compiled: &CompiledPlot, ctx: &SessionContext) -> Vec<u8> {
    let evaluated = compiled
        .evaluate(ctx, None)
        .await
        .expect("fresh evaluation");
    bincode::serialize(&evaluated.scene_graph).expect("scene bytes")
}

/// Probe A: a new facet value arriving via INSERT INTO must produce a new
/// facet cell on the next session evaluation.
///
/// KNOWN-STALE until dissolution Phase 2: the session-lifetime
/// PartitionSlotCache keys on plan-Debug + params (no data identity), so
/// the second session evaluation serves the pre-insert slot list.
#[tokio::test]
#[ignore = "known stale-slot bug: session PartitionSlotCache misses INSERTed facet values; enabled by dissolution Phase 2"]
async fn probe_a_insert_into_adds_facet_cell_in_session() {
    let (ctx, _cache) = cached_session_context(EvaluationCacheConfig::default());
    ctx.register_batch(
        "events",
        events_batch(&["EU", "EU", "NA"], &[1.0, 2.0, 3.0]),
    )
    .expect("register events");
    let ctx = Arc::new(ctx);
    let compiled = Arc::new(faceted_chart(ctx.as_ref()).await);
    let mut session = Arc::clone(&compiled).instantiate(Arc::clone(&ctx));

    let before = scene_via_session(&mut session).await;

    // A NEW region arrives (streaming-dashboard shape).
    ctx.sql("INSERT INTO events VALUES ('APAC', 9.0)")
        .await
        .expect("insert plan")
        .collect()
        .await
        .expect("insert exec");

    let after_session = scene_via_session(&mut session).await;
    let after_fresh = scene_fresh(compiled.as_ref(), ctx.as_ref()).await;

    assert_ne!(
        before, after_fresh,
        "fixture sanity: the new region must change the scene (fresh evaluation)"
    );
    assert_eq!(
        after_session, after_fresh,
        "session evaluation after INSERT must match a fresh evaluation \
         (a mismatch means the session slot cache served stale facet slots)"
    );
}

/// Probe C: the same INSERT shifts the y-domain extent; a session
/// evaluation must reflect it.
///
/// Verifies the ScaleDomainCache `data_ptr` identity contract for
/// provider-interior mutation (pointer identity CANNOT see INSERT INTO).
#[tokio::test]
#[ignore = "probe: run explicitly; expected stale until Phase 3 settles the data_ptr contract"]
async fn probe_c_insert_into_shifts_domain_in_session() {
    let (ctx, _cache) = cached_session_context(EvaluationCacheConfig::default());
    ctx.register_batch("events", events_batch(&["EU", "EU"], &[1.0, 2.0]))
        .expect("register events");
    let ctx = Arc::new(ctx);
    // Unfaceted: isolate the domain cache from the slot cache.
    let data = ctx.sql("SELECT * FROM events").await.expect("events query");
    let compiled = Arc::new(
        Plot::<Cartesian>::new()
            .data(data)
            .mark(Symbol::new().x(col("value")).y(col("value")).size(48.0))
            .compile(ctx.as_ref())
            .await
            .expect("compile plot"),
    );
    let mut session = Arc::clone(&compiled).instantiate(Arc::clone(&ctx));

    let before = scene_via_session(&mut session).await;

    // New max value: the inferred y-domain must stretch.
    ctx.sql("INSERT INTO events VALUES ('EU', 50.0)")
        .await
        .expect("insert plan")
        .collect()
        .await
        .expect("insert exec");

    let after_session = scene_via_session(&mut session).await;
    let after_fresh = scene_fresh(compiled.as_ref(), ctx.as_ref()).await;

    assert_ne!(
        before, after_fresh,
        "fixture sanity: the new extreme must change the scene (fresh evaluation)"
    );
    assert_eq!(
        after_session, after_fresh,
        "session evaluation after INSERT must match a fresh evaluation \
         (a mismatch means ScaleDomainCache served a stale domain)"
    );
}

/// Probe B: same-name table RE-REGISTRATION. Compiled plots re-resolve
/// table references against the live context at evaluation time (they are
/// serializable, so they cannot hold provider `Arc`s) — a FRESH evaluation
/// sees the new data. The session, however, serves the pre-swap scene:
/// a second live staleness vector for the session memos.
///
/// KNOWN-STALE until dissolution Phase 2/3.
#[tokio::test]
#[ignore = "known stale-session bug: re-registered tables invisible to session memos; enabled by dissolution Phases 2-3"]
async fn probe_b_reregistration_reaches_session_evaluations() {
    let (ctx, _cache) = cached_session_context(EvaluationCacheConfig::default());
    ctx.register_batch("events", events_batch(&["EU", "NA"], &[1.0, 2.0]))
        .expect("register events");
    let ctx = Arc::new(ctx);
    let compiled = Arc::new(faceted_chart(ctx.as_ref()).await);
    let mut session = Arc::clone(&compiled).instantiate(Arc::clone(&ctx));

    let before = scene_via_session(&mut session).await;

    // Replace the registration wholesale (new provider, new data).
    ctx.deregister_table("events").expect("deregister");
    ctx.register_batch(
        "events",
        events_batch(&["EU", "NA", "APAC", "LATAM"], &[1.0, 2.0, 3.0, 4.0]),
    )
    .expect("re-register events");

    let after_session = scene_via_session(&mut session).await;
    let after_fresh = scene_fresh(compiled.as_ref(), ctx.as_ref()).await;

    assert_ne!(
        before, after_fresh,
        "fixture sanity: fresh evaluations resolve tables by name and see the swap"
    );
    assert_eq!(
        after_session, after_fresh,
        "session evaluation after a table swap must match a fresh evaluation \
         (a mismatch means session memos served pre-swap artifacts)"
    );
}

// ---------------------------------------------------------------------------
// Phase 1: measurement — what is each session memo worth with the physical
// cache installed? (run explicitly: `-- --ignored measure_memo`)
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "measurement: run explicitly and record numbers in the plan's progress log"]
async fn measure_memo_value_matrix() {
    use avenger_chart::plot::PlotSessionOptions;
    use std::time::Instant;

    // A moderately expensive faceted chart: 6 cells over 3k rows with an
    // aggregate chain, plus a style param ($size) that does NOT touch data
    // — re-evaluating with a new $size exercises the same key behavior as
    // the pinned responsive-width scenario (dependency-pruned cache keys).
    fn big_events() -> RecordBatch {
        let n = 3000;
        let regions: Vec<String> = (0..n).map(|i| format!("r{}", i % 6)).collect();
        let values: Vec<f64> = (0..n).map(|i| ((i * 37) % 501) as f64 * 0.1).collect();
        RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("region", DataType::Utf8, false),
                Field::new("value", DataType::Float64, false),
            ])),
            vec![
                Arc::new(StringArray::from(regions)) as ArrayRef,
                Arc::new(Float64Array::from(values)) as ArrayRef,
            ],
        )
        .expect("events batch")
    }

    async fn wrap_chart(ctx: &SessionContext) -> CompiledPlot {
        let width = Param::new("width", ScalarValue::Float64(Some(720.0)));
        let data = ctx.sql("SELECT * FROM events").await.expect("query");
        let leaf = Plot::<Cartesian>::new().mark(
            MarkGroup::new().transform(
                Aggregate::new()
                    .sum("total", col("value"))
                    .mean("avg", col("value")),
                |group, aggregate| {
                    group.mark(
                        Symbol::new()
                            .x(aggregate.output("total"))
                            .y(aggregate.output("avg"))
                            .size(40.0),
                    )
                },
            ),
        );
        Plot::<FacetWrap>::new()
            .add_param(width.clone())
            .canvas_constraint(CanvasConstraint::width(width.expr()))
            .plot_constraint(PlotConstraint::height(140.0))
            .data(data)
            .mark(Subplot::new(leaf).wrap_with(col("region"), |c| c.columns(3)))
            .compile(ctx)
            .await
            .expect("compile wrap chart")
    }

    let (ctx, physical_cache) = cached_session_context(EvaluationCacheConfig::default());
    ctx.register_batch("events", big_events())
        .expect("register");
    let ctx = Arc::new(ctx);
    let compiled = Arc::new(wrap_chart(ctx.as_ref()).await);

    // Process warm-up (fs caches, allocator, lazy statics): one throwaway
    // session evaluated twice, then the physical cache cleared.
    {
        let mut warmup = Arc::clone(&compiled).instantiate(Arc::clone(&ctx));
        warmup
            .evaluate(EvaluationRequest::new().exact())
            .await
            .expect("warmup");
        warmup
            .evaluate(EvaluationRequest::new().exact())
            .await
            .expect("warmup");
    }

    let configs = [
        ("both memos ON ", false, false),
        ("slot OFF      ", true, false),
        ("domain OFF    ", false, true),
        ("both OFF      ", true, true),
    ];

    println!(
        "\n{:<15} {:>9} {:>9} {:>9} {:>10} {:>10} {:>9} {:>9}",
        "config", "cold ms", "warm ms", "param ms", "prev ms", "plan/eval", "slot m/h", "dom m/h"
    );
    for (label, slot_off, domain_off) in configs {
        physical_cache.clear();
        let mut session = Arc::clone(&compiled)
            .instantiate(Arc::clone(&ctx))
            .with_options(PlotSessionOptions {
                disable_facet_semantic_cache: slot_off,
                disable_scale_domain_cache: domain_off,
            });

        let time_eval = |label: &'static str| label; // readability no-op
        let _ = time_eval;

        // Cold.
        let start = Instant::now();
        let (_e, cold_metrics) = session
            .evaluate_with_metrics(EvaluationRequest::new().exact())
            .await
            .expect("cold");
        let cold_ms = start.elapsed().as_secs_f64() * 1e3;

        // Warm (same request) — median of 5.
        let mut warm = Vec::new();
        for _ in 0..5 {
            let start = Instant::now();
            session
                .evaluate(EvaluationRequest::new().exact())
                .await
                .expect("warm");
            warm.push(start.elapsed().as_secs_f64() * 1e3);
        }
        warm.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let warm_median = warm[2];

        // Width-only change (dependency-pruned from cache keys): the
        // pinned responsive-resize scenario.
        let mut param_times = Vec::new();
        let mut last_metrics = None;
        for (i, size) in [780.0f64, 660.0, 900.0, 840.0, 700.0].iter().enumerate() {
            let mut patch = IndexMap::new();
            patch.insert("width".to_string(), ScalarValue::Float64(Some(*size)));
            let start = Instant::now();
            let (_e, m) = session
                .evaluate_with_metrics(EvaluationRequest::new().exact().param_patch(patch))
                .await
                .expect("param eval");
            param_times.push(start.elapsed().as_secs_f64() * 1e3);
            if i == 4 {
                last_metrics = Some(m);
            }
        }
        param_times.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let param_median = param_times[2];
        let pm = last_metrics.expect("metrics");

        // Preview (observe-only) after warm.
        let mut prev_times = Vec::new();
        for _ in 0..3 {
            let start = Instant::now();
            session
                .evaluate(EvaluationRequest::new().preview())
                .await
                .expect("preview");
            prev_times.push(start.elapsed().as_secs_f64() * 1e3);
        }
        prev_times.sort_by(|a, b| a.partial_cmp(b).unwrap());

        let plannings = pm
            .physical_cache
            .as_ref()
            .map(|d| d.hits + d.misses)
            .unwrap_or_default();
        println!(
            "{:<15} {:>9.1} {:>9.1} {:>9.1} {:>10.1} {:>10} {:>6}/{:<3} {:>6}/{:<3}",
            label,
            cold_ms,
            warm_median,
            param_median,
            prev_times[1],
            plannings,
            pm.pipeline.facet_semantic_cache_misses,
            pm.pipeline.facet_semantic_cache_hits,
            pm.pipeline.scale_domain_cache_misses,
            pm.pipeline.scale_domain_cache_hits,
        );
        let _ = cold_metrics;
    }
}
