//! Regression tests for the PartitionSlotCache / ScaleDomainCache
//! dissolution campaign (`scratch/slot-domain-cache-dissolution-plan.md`):
//! both memos are per-evaluation, so host-side data mutation between
//! session evaluations must be reflected by the next evaluation.
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
    prelude::{SessionContext, col},
};

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
    Chart::<FacetColumn>::new()
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

/// A new facet value arriving via INSERT INTO must produce a new facet
/// cell on the next session evaluation.
///
/// Reproduced stale before dissolution Phase 2: the session-lifetime
/// PartitionSlotCache keyed on plan-Debug + params (no data identity), so
/// the second session evaluation served the pre-insert slot list. The
/// slot memo is now per-evaluation.
#[tokio::test]
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

/// The same INSERT shifts the y-domain extent; a session evaluation must
/// reflect it.
///
/// Reproduced stale before dissolution Phase 3: the session-lifetime
/// ScaleDomainCache keyed on `data_ptr` pointer identity, which cannot
/// see provider-interior mutation (INSERT INTO). The domain memo is now
/// per-evaluation.
#[tokio::test]
async fn probe_c_insert_into_shifts_domain_in_session() {
    let (ctx, _cache) = cached_session_context(EvaluationCacheConfig::default());
    ctx.register_batch("events", events_batch(&["EU", "EU"], &[1.0, 2.0]))
        .expect("register events");
    let ctx = Arc::new(ctx);
    // Unfaceted: isolate the domain cache from the slot cache.
    let data = ctx.sql("SELECT * FROM events").await.expect("events query");
    let compiled = Arc::new(
        Chart::<Cartesian>::new()
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

/// Same-name table RE-REGISTRATION. Compiled plots re-resolve table
/// references against the live context at evaluation time (they are
/// serializable, so they cannot hold provider `Arc`s) — a FRESH evaluation
/// sees the new data, and with per-evaluation memos so does the session.
///
/// Reproduced stale before dissolution Phases 2-3.
#[tokio::test]
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

// The Phase 1 measurement matrix that priced each memo (per-config knobs on
// PlotSessionOptions) was retired with the knobs once the decision landed;
// its methodology and numbers live in the plan's Phase 1 progress-log entry.
