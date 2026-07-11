//! Consumer-perspective integration tests: everything here uses only the
//! public API, the way a host application would.
//!
//! The census harness runs each SQL sequence on two identically provisioned
//! contexts — one with the cache rule installed, one without — and compares
//! results after every statement: rendered-order comparison for `ORDER BY`
//! queries, sorted-row comparison otherwise (replay pins one outcome of
//! order-nondeterministic operators, so on/off byte-identity is only
//! guaranteed for order-stable plans).

use std::sync::Arc;
use std::time::Instant;

use arrow::array::{Float64Array, Int64Array, RecordBatch, StringArray, TimestampNanosecondArray};
use arrow::datatypes::{DataType, Field, Schema, TimeUnit};
use arrow::util::pretty::pretty_format_batches;
use datafusion::datasource::MemTable;
use datafusion::execution::session_state::SessionStateBuilder;
use datafusion::physical_plan::ExecutionPlan;
use datafusion::prelude::SessionContext;

use avenger_datafusion_cache::{
    CacheVersion, CacheVersionProvider, EvaluationCache, EvaluationCacheConfig,
    EvaluationCachePlanner,
};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn sales_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("region", DataType::Utf8, true),
        Field::new("amount", DataType::Float64, true),
        Field::new("ts", DataType::Timestamp(TimeUnit::Nanosecond, None), false),
    ]))
}

fn sales_batch() -> RecordBatch {
    let n = 500i64;
    let ids: Vec<i64> = (0..n).collect();
    let regions: Vec<Option<String>> = (0..n)
        .map(|i| {
            if i % 17 == 0 {
                None // nulls in the group key
            } else {
                Some(format!("r{}", i % 5))
            }
        })
        .collect();
    let amounts: Vec<Option<f64>> = (0..n)
        .map(|i| {
            if i % 23 == 0 {
                None
            } else {
                Some((i as f64) * 1.5 - 100.0)
            }
        })
        .collect();
    let timestamps: Vec<i64> = (0..n)
        .map(|i| 1_700_000_000_000_000_000 + i * 60_000_000_000)
        .collect();
    RecordBatch::try_new(
        sales_schema(),
        vec![
            Arc::new(Int64Array::from(ids)),
            Arc::new(StringArray::from(regions)),
            Arc::new(Float64Array::from(amounts)),
            Arc::new(TimestampNanosecondArray::from(timestamps)),
        ],
    )
    .unwrap()
}

fn regions_batch() -> (Arc<Schema>, RecordBatch) {
    let schema = Arc::new(Schema::new(vec![
        Field::new("region", DataType::Utf8, false),
        Field::new("priority", DataType::Int64, false),
    ]));
    let batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![
            Arc::new(StringArray::from(vec!["r0", "r1", "r2", "r3", "r4"])),
            Arc::new(Int64Array::from(vec![3, 1, 4, 1, 5])),
        ],
    )
    .unwrap();
    (schema, batch)
}

fn register_fixtures(ctx: &SessionContext) {
    let sales = MemTable::try_new(sales_schema(), vec![vec![sales_batch()]]).unwrap();
    ctx.register_table("sales", Arc::new(sales)).unwrap();
    let (schema, batch) = regions_batch();
    let regions = MemTable::try_new(schema, vec![vec![batch]]).unwrap();
    ctx.register_table("regions", Arc::new(regions)).unwrap();
}

fn cached_context(config: EvaluationCacheConfig) -> (SessionContext, Arc<EvaluationCache>) {
    let cache = EvaluationCache::new(config);
    let planner = EvaluationCachePlanner::new(Arc::clone(&cache));
    let state = SessionStateBuilder::new()
        .with_default_features()
        .with_physical_optimizer_rule(Arc::new(planner))
        .build();
    let ctx = SessionContext::new_with_state(state);
    register_fixtures(&ctx);
    (ctx, cache)
}

fn plain_context() -> SessionContext {
    let ctx = SessionContext::new();
    register_fixtures(&ctx);
    ctx
}

async fn run(ctx: &SessionContext, sql: &str) -> Vec<RecordBatch> {
    ctx.sql(sql).await.unwrap().collect().await.unwrap()
}

// ---------------------------------------------------------------------------
// Census harness
// ---------------------------------------------------------------------------

fn rendered(batches: &[RecordBatch]) -> String {
    pretty_format_batches(batches).unwrap().to_string()
}

fn sorted_rows(batches: &[RecordBatch]) -> Vec<String> {
    let text = rendered(batches);
    let mut rows: Vec<String> = text.lines().map(|line| line.to_string()).collect();
    rows.sort();
    rows
}

/// Run `statements` on a cached and an uncached context, comparing results
/// after every statement. `ordered` marks statements whose output order is
/// pinned by an `ORDER BY` (compared in rendered order); everything else is
/// compared as sorted rows.
async fn assert_cache_equivalence(
    cached: &SessionContext,
    plain: &SessionContext,
    statements: &[(&str, bool)],
) {
    for (sql, ordered) in statements {
        let cached_result = run(cached, sql).await;
        let plain_result = run(plain, sql).await;
        if *ordered {
            assert_eq!(
                rendered(&cached_result),
                rendered(&plain_result),
                "ordered output diverged for: {sql}"
            );
        } else {
            assert_eq!(
                sorted_rows(&cached_result),
                sorted_rows(&plain_result),
                "row set diverged for: {sql}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Consumer smoke tests (public API shape)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn consumer_smoke_repeated_query_hits_and_matches() {
    let (ctx, cache) = cached_context(EvaluationCacheConfig::default());
    let sql =
        "SELECT region, sum(amount) AS s FROM sales GROUP BY region ORDER BY region NULLS FIRST";

    let mut outputs = Vec::new();
    for _ in 0..3 {
        outputs.push(rendered(&run(&ctx, sql).await));
    }
    let metrics = cache.metrics();
    assert!(metrics.hits >= 1, "post-warm-up runs must hit: {metrics:?}");
    assert!(
        outputs.windows(2).all(|w| w[0] == w[1]),
        "output identical across runs"
    );
}

#[tokio::test]
async fn version_provider_invalidates_without_content_change() {
    use std::sync::atomic::{AtomicU64, Ordering};

    #[derive(Debug, Default)]
    struct BumpableVersion(AtomicU64);
    impl CacheVersionProvider for BumpableVersion {
        fn source_version(&self, _plan: &dyn ExecutionPlan) -> Option<CacheVersion> {
            Some(CacheVersion(self.0.load(Ordering::Relaxed) as u128))
        }
    }

    let provider = Arc::new(BumpableVersion::default());
    let (ctx, cache) = cached_context(EvaluationCacheConfig::default());
    cache.register_version_provider(Arc::clone(&provider) as Arc<dyn CacheVersionProvider>);

    let sql = "SELECT count(*) AS c FROM sales";
    run(&ctx, sql).await;
    run(&ctx, sql).await;
    run(&ctx, sql).await;
    let warm = cache.metrics();
    assert!(warm.hits >= 1, "warm-up must hit: {warm:?}");

    // Bump the version: same data, new identity → no further hits until
    // re-committed under the new fingerprint.
    provider.0.store(1, Ordering::Relaxed);
    let hits_before = cache.metrics().hits;
    run(&ctx, sql).await;
    assert_eq!(
        cache.metrics().hits,
        hits_before,
        "bumped source version must invalidate (no hit on the old entry)"
    );

    run(&ctx, sql).await;
    run(&ctx, sql).await;
    assert!(
        cache.metrics().hits > hits_before,
        "new-version entry commits and hits again"
    );
}

// ---------------------------------------------------------------------------
// Census cases
// ---------------------------------------------------------------------------

#[tokio::test]
async fn census_repeated_identical_aggregate() {
    let (cached, cache) = cached_context(EvaluationCacheConfig::default());
    let plain = plain_context();
    let sql = "SELECT region, sum(amount) AS s, count(*) AS c FROM sales GROUP BY region ORDER BY region NULLS FIRST";

    assert_cache_equivalence(&cached, &plain, &[(sql, true)]).await;
    let m1 = cache.metrics();
    assert_eq!(
        (m1.hits, m1.committed_writes),
        (0, 0),
        "run 1: observe only"
    );

    assert_cache_equivalence(&cached, &plain, &[(sql, true)]).await;
    let m2 = cache.metrics();
    assert_eq!(m2.committed_writes, 1, "run 2: admitted and committed");
    assert_eq!(m2.hits, 0);

    assert_cache_equivalence(&cached, &plain, &[(sql, true)]).await;
    let m3 = cache.metrics();
    assert_eq!(m3.hits, 1, "run 3: root hit");
    assert_eq!(m3.committed_writes, 1);
}

#[tokio::test]
async fn census_param_style_literal_changes() {
    let (cached, cache) = cached_context(EvaluationCacheConfig::default());
    let plain = plain_context();
    let q = |lit: i64| format!("SELECT id, amount FROM sales WHERE id > {lit} ORDER BY id");

    // Three literals, then the first again: the shared scan commits during
    // the second statement and serves hits from the third on; the repeated
    // first literal reuses both.
    let statements: Vec<String> = vec![q(5), q(6), q(7), q(5)];
    for (i, sql) in statements.iter().enumerate() {
        assert_cache_equivalence(&cached, &plain, &[(sql, true)]).await;
        let metrics = cache.metrics();
        match i {
            0 => assert_eq!(metrics.hits, 0),
            1 => assert_eq!(
                metrics.committed_writes, 1,
                "shared upstream scan committed during the second literal"
            ),
            2 => assert!(
                metrics.hits >= 1,
                "third literal reuses the scan: {metrics:?}"
            ),
            _ => assert!(
                metrics.hits >= 2,
                "repeat reuses upstream again: {metrics:?}"
            ),
        }
    }
}

#[tokio::test]
async fn census_shared_subtree_across_different_queries() {
    let (cached, cache) = cached_context(EvaluationCacheConfig::default());
    let plain = plain_context();
    // Identical aggregate+scan below different sorts.
    let q1 =
        "SELECT region, sum(amount) AS s FROM sales GROUP BY region ORDER BY region NULLS FIRST";
    let q2 =
        "SELECT region, sum(amount) AS s FROM sales GROUP BY region ORDER BY s, region NULLS FIRST";

    // q1 twice: q1's root commits; the shared aggregate below it is observed
    // twice.
    assert_cache_equivalence(&cached, &plain, &[(q1, true), (q1, true)]).await;
    let after_q1 = cache.metrics();
    assert_eq!(after_q1.committed_writes, 1);

    // q2 first run: its root misses, but the shared aggregate subtree is
    // admission-eligible thanks to q1's observations (fingerprint identity
    // across query shapes) and commits.
    assert_cache_equivalence(&cached, &plain, &[(q2, true)]).await;
    let after_q2 = cache.metrics();
    assert!(
        after_q2.committed_writes > after_q1.committed_writes,
        "shared subtree written under q2, warmed by q1's observations: {after_q2:?}"
    );

    // q2 second run: the shared aggregate serves a hit.
    let hits_before = cache.metrics().hits;
    assert_cache_equivalence(&cached, &plain, &[(q2, true)]).await;
    assert!(
        cache.metrics().hits > hits_before,
        "shared subtree read back under q2"
    );
}

#[tokio::test]
async fn census_join_reused_under_two_parents() {
    let (cached, cache) = cached_context(EvaluationCacheConfig::default());
    let plain = plain_context();
    // Same join inputs and columns below different aggregate expressions.
    let qa = "SELECT r.priority, sum(s.amount) AS x FROM sales s JOIN regions r ON s.region = r.region GROUP BY r.priority ORDER BY r.priority";
    let qb = "SELECT r.priority, sum(s.amount) * 2 AS x FROM sales s JOIN regions r ON s.region = r.region GROUP BY r.priority ORDER BY r.priority";

    assert_cache_equivalence(&cached, &plain, &[(qa, true), (qa, true)]).await;
    let after_qa = cache.metrics();
    assert!(after_qa.committed_writes >= 1);

    assert_cache_equivalence(&cached, &plain, &[(qb, true), (qb, true)]).await;
    let after_qb = cache.metrics();
    assert!(
        after_qb.hits >= 1 || after_qb.committed_writes > after_qa.committed_writes,
        "the shared join subtree is reused across parents: {after_qb:?}"
    );
    // Whichever way the frontier fell, a third run of each must be correct.
    assert_cache_equivalence(&cached, &plain, &[(qa, true), (qb, true)]).await;
}

#[tokio::test]
async fn census_topk_dynamic_filter_excluded_and_equivalent() {
    let (cached, cache) = cached_context(EvaluationCacheConfig::default());
    let plain = plain_context();

    // Parquet-backed copy so TopK dynamic filter pushdown (default-on) has a
    // pushdown-accepting scan under it.
    let dir = std::env::temp_dir().join(format!(
        "avenger-datafusion-cache-census-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("sales.parquet").to_str().unwrap().to_string();
    plain
        .sql("SELECT * FROM sales")
        .await
        .unwrap()
        .write_parquet(
            &path,
            datafusion::dataframe::DataFrameWriteOptions::new(),
            None,
        )
        .await
        .unwrap();
    cached
        .register_parquet("sales_pq", &path, Default::default())
        .await
        .unwrap();
    plain
        .register_parquet("sales_pq", &path, Default::default())
        .await
        .unwrap();

    let sql = "SELECT id, amount FROM sales_pq ORDER BY amount NULLS LAST LIMIT 10";
    assert_cache_equivalence(&cached, &plain, &[(sql, true), (sql, true), (sql, true)]).await;

    let metrics = cache.metrics();
    assert!(
        metrics.excluded_dynamic > 0,
        "dynamic filters excluded: {metrics:?}"
    );
    assert_eq!(metrics.entries, 0, "no entries for a fully dynamic plan");
    assert_eq!(metrics.committed_writes, 0);

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn census_volatile_query_never_false_hits() {
    let (cached, cache) = cached_context(EvaluationCacheConfig::default());
    let plain = plain_context();

    // Deterministic output through a volatile expression (`random()` ∈
    // [0, 1), so the filter keeps every row).
    let sql = "SELECT id FROM (SELECT id, random() AS r FROM sales) WHERE r >= 0 ORDER BY id";
    assert_cache_equivalence(&cached, &plain, &[(sql, true), (sql, true), (sql, true)]).await;
    let metrics = cache.metrics();
    assert!(
        metrics.excluded_volatile > 0,
        "volatile subtree excluded: {metrics:?}"
    );

    // Direct volatile output: two cached runs must differ (no false hit).
    let volatile_sql = "SELECT sum(r) AS total FROM (SELECT random() AS r FROM sales)";
    let first = rendered(&run(&cached, volatile_sql).await);
    let second = rendered(&run(&cached, volatile_sql).await);
    assert_ne!(first, second, "volatile results must not replay from cache");
}

#[tokio::test]
async fn census_limit_above_admitted_subtree_never_commits_partial() {
    let (cached, cache) = cached_context(EvaluationCacheConfig::default());
    let plain = plain_context();

    let inner = "SELECT id, amount FROM sales WHERE amount > 0 ORDER BY id";
    let limited =
        "SELECT * FROM (SELECT id, amount FROM sales WHERE amount > 0 ORDER BY id) LIMIT 3";

    // One observation of the inner shape, then the limited query: whatever
    // the frontier admits, a partial (early-terminated) result must never
    // commit, and subsequent full queries must be complete and correct.
    assert_cache_equivalence(&cached, &plain, &[(inner, true), (limited, true)]).await;
    assert_cache_equivalence(&cached, &plain, &[(limited, true), (inner, true)]).await;
    assert_cache_equivalence(&cached, &plain, &[(inner, true)]).await;

    // Every committed entry must be a complete result: replaying any entry
    // must never shrink a full query's output (verified by the inner-query
    // equivalence above). Discards from early termination are legitimate,
    // and at rest every admitted write has resolved one way or the other.
    let metrics = cache.metrics();
    assert_eq!(
        metrics.admitted_writes,
        metrics.committed_writes + metrics.discarded_writes,
        "all admitted writes resolved: {metrics:?}"
    );
}

#[tokio::test]
async fn census_sort_order_pinning_for_unordered_aggregate() {
    let (cached, cache) = cached_context(EvaluationCacheConfig::default());
    let plain = plain_context();
    let sql = "SELECT region, sum(amount) AS s FROM sales GROUP BY region";

    // Warm to a committed root entry.
    assert_cache_equivalence(&cached, &plain, &[(sql, false), (sql, false)]).await;
    assert!(cache.metrics().committed_writes >= 1);

    // Once cached, replay pins one row order: consecutive cached runs are
    // identical in rendered order, and sorted-equal to the uncached context.
    let third = run(&cached, sql).await;
    let fourth = run(&cached, sql).await;
    assert_eq!(
        rendered(&third),
        rendered(&fourth),
        "cached replays pin the committed row order"
    );
    assert_eq!(sorted_rows(&third), sorted_rows(&run(&plain, sql).await));
}

#[tokio::test]
async fn census_eviction_pressure_alternating_entries() {
    // Budget sized to hold roughly one full-table entry, so two alternating
    // cached queries evict each other.
    let (cached, cache) = cached_context(EvaluationCacheConfig {
        max_memory_bytes: 20 * 1024,
        ..Default::default()
    });
    let plain = plain_context();
    let qa = "SELECT id, region, amount, ts FROM sales ORDER BY id";
    let qb = "SELECT id, region, amount, ts FROM sales ORDER BY id DESC";

    for _ in 0..3 {
        assert_cache_equivalence(&cached, &plain, &[(qa, true), (qb, true)]).await;
    }
    let metrics = cache.metrics();
    assert!(
        metrics.committed_writes >= 2,
        "both queries were admitted: {metrics:?}"
    );
    assert!(
        metrics.evictions >= 1,
        "budget pressure evicted: {metrics:?}"
    );
    assert!(
        metrics.bytes <= 20 * 1024,
        "byte budget respected: {metrics:?}"
    );
}

#[tokio::test]
async fn census_clear_behaves_like_cold_start() {
    let (cached, cache) = cached_context(EvaluationCacheConfig::default());
    let plain = plain_context();
    let sql = "SELECT region, count(*) AS c FROM sales GROUP BY region ORDER BY region NULLS FIRST";

    assert_cache_equivalence(&cached, &plain, &[(sql, true), (sql, true), (sql, true)]).await;
    assert!(cache.metrics().hits >= 1);

    cache.clear();
    let hits_before = cache.metrics().hits;
    assert_cache_equivalence(&cached, &plain, &[(sql, true)]).await;
    assert_eq!(
        cache.metrics().hits,
        hits_before,
        "no hits right after clear()"
    );

    // The cache warms again from scratch.
    assert_cache_equivalence(&cached, &plain, &[(sql, true), (sql, true)]).await;
    assert!(
        cache.metrics().hits > hits_before,
        "recommitted after clear"
    );
}

// ---------------------------------------------------------------------------
// Benchmark (the design doc's phase-1 exit criterion; run explicitly with
// `cargo test --release -p avenger-datafusion-cache -- --ignored bench`)
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "benchmark: run explicitly and record numbers in the plan's progress log"]
async fn bench_fingerprint_overhead() {
    let mut statements: Vec<String> = vec![
        // Multi-join + aggregate shapes.
        "SELECT r.priority, sum(s.amount) AS x, count(*) AS c FROM sales s JOIN regions r ON s.region = r.region GROUP BY r.priority ORDER BY r.priority".to_string(),
        "SELECT s1.region, avg(s1.amount) AS a FROM sales s1 JOIN sales s2 ON s1.id = s2.id GROUP BY s1.region ORDER BY s1.region NULLS FIRST".to_string(),
        // Window functions.
        "SELECT id, sum(amount) OVER (PARTITION BY region ORDER BY id) AS running FROM sales ORDER BY id LIMIT 100".to_string(),
        "SELECT id, row_number() OVER (PARTITION BY region ORDER BY amount DESC NULLS LAST) AS rn FROM sales ORDER BY id LIMIT 100".to_string(),
        // Nested aggregation.
        "SELECT region, max(s) FROM (SELECT region, id % 7 AS bucket, sum(amount) AS s FROM sales GROUP BY region, id % 7) GROUP BY region ORDER BY region NULLS FIRST".to_string(),
        "SELECT count(*) FROM (SELECT DISTINCT region, id % 11 FROM sales)".to_string(),
    ];
    // Facet-like repeated shapes: the same aggregate under different filters.
    for i in 0..14 {
        statements.push(format!(
            "SELECT region, sum(amount) AS s, count(*) AS c FROM sales WHERE id % 17 <> {i} AND amount > {} GROUP BY region ORDER BY region NULLS FIRST",
            -50 + i * 10
        ));
    }

    println!();
    println!(
        "{:<4} {:>6} {:>14} {:>16} {:>14}",
        "plan", "nodes", "plan+opt (µs)", "fingerprint (µs)", "rewrite (µs)"
    );
    let mut total_plan = 0u128;
    let mut total_fp = 0u128;
    let mut total_rewrite = 0u128;
    for (i, sql) in statements.iter().enumerate() {
        // Fresh cache per plan so rewrite timing is a pure cold pass.
        let cache = EvaluationCache::new(EvaluationCacheConfig::default());
        let planner = EvaluationCachePlanner::new(Arc::clone(&cache));
        let ctx = plain_context();

        let plan_start = Instant::now();
        let df = ctx.sql(sql).await.unwrap();
        let plan = df.create_physical_plan().await.unwrap();
        let plan_micros = plan_start.elapsed().as_micros();

        fn node_count(plan: &Arc<dyn ExecutionPlan>) -> usize {
            1 + plan.children().iter().map(|c| node_count(c)).sum::<usize>()
        }
        let nodes = node_count(&plan);

        let rewrite_start = Instant::now();
        let _rewritten = planner.rewrite(Arc::clone(&plan)).unwrap();
        let rewrite_micros = rewrite_start.elapsed().as_micros();
        let fp_micros = (cache.metrics().fingerprint_nanos_total / 1_000) as u128;

        total_plan += plan_micros;
        total_fp += fp_micros;
        total_rewrite += rewrite_micros;
        println!(
            "{:<4} {:>6} {:>14} {:>16} {:>14}",
            i, nodes, plan_micros, fp_micros, rewrite_micros
        );
    }
    println!(
        "{:<4} {:>6} {:>14} {:>16} {:>14}",
        "sum", "", total_plan, total_fp, total_rewrite
    );
    println!(
        "fingerprint overhead: {:.1}% of plan+optimize time",
        (total_fp as f64 / total_plan as f64) * 100.0
    );
}

// ---------------------------------------------------------------------------
// End-to-end speedup benchmark: repeated + shared-structure queries over a
// 1M-row table (run explicitly with `-- --ignored bench_shared`)
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "benchmark: run explicitly and record numbers in the plan's progress log"]
async fn bench_shared_structure_speedup() {
    use std::time::Duration;

    // 1M rows, 50k distinct group keys, split into 8 batches.
    let n: i64 = 1_000_000;
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("k", DataType::Int64, false),
        Field::new("amount", DataType::Float64, false),
    ]));
    let batches: Vec<RecordBatch> = (0..8)
        .map(|chunk| {
            let lo = chunk * (n / 8);
            let hi = lo + n / 8;
            let ids: Vec<i64> = (lo..hi).collect();
            let keys: Vec<i64> = (lo..hi).map(|i| (i * 2_654_435_761) % 50_000).collect();
            let amounts: Vec<f64> = (lo..hi)
                .map(|i| ((i % 10_007) as f64) * 0.25 - 100.0)
                .collect();
            RecordBatch::try_new(
                Arc::clone(&schema),
                vec![
                    Arc::new(Int64Array::from(ids)),
                    Arc::new(Int64Array::from(keys)),
                    Arc::new(Float64Array::from(amounts)),
                ],
            )
            .unwrap()
        })
        .collect();

    let register_big = |ctx: &SessionContext| {
        let table = MemTable::try_new(Arc::clone(&schema), vec![batches.clone()]).unwrap();
        ctx.register_table("big", Arc::new(table)).unwrap();
    };
    let (cached, cache) = cached_context(EvaluationCacheConfig::default());
    register_big(&cached);
    let plain = plain_context();
    register_big(&plain);

    async fn timed(ctx: &SessionContext, sql: &str) -> (Duration, String) {
        let start = Instant::now();
        let batches = run(ctx, sql).await;
        (start.elapsed(), rendered(&batches))
    }
    let ms = |d: Duration| d.as_secs_f64() * 1e3;

    // Case A: the identical query repeated (heavy aggregate + sort).
    let q =
        "SELECT k, sum(amount) AS s, count(*) AS c FROM big GROUP BY k ORDER BY s DESC, k LIMIT 20";
    println!("\n=== Case A: identical query repeated (1M rows, 50k groups) ===");
    println!("{:<10} {:>12} {:>12}", "run", "plain (ms)", "cached (ms)");
    let mut reference: Option<String> = None;
    for i in 0..5 {
        let (plain_time, plain_out) = timed(&plain, q).await;
        let (cached_time, cached_out) = timed(&cached, q).await;
        assert_eq!(plain_out, cached_out, "run {i}: outputs must match");
        reference.get_or_insert(plain_out);
        let label = match i {
            0 => "1 observe",
            1 => "2 write",
            _ => "3+ hit",
        };
        println!(
            "{:<10} {:>12.2} {:>12.2}",
            label,
            ms(plain_time),
            ms(cached_time)
        );
    }
    let m = cache.metrics();
    assert!(m.hits >= 3, "case A must serve hits: {m:?}");

    // Case B: shared upstream structure under changing literals — the heavy
    // aggregate is identical across variants; only the outer filter/sort
    // changes. After the aggregate commits (variant 1), every NEW variant
    // executes only the cheap outer stage over the cached 50k-row result.
    cache.clear();
    let variant = |x: i64| {
        format!(
            "SELECT k, s FROM (SELECT k, sum(amount) AS s FROM big GROUP BY k) \
             WHERE s > {x} ORDER BY s, k LIMIT 10"
        )
    };
    println!("\n=== Case B: shared heavy aggregate under changing literals ===");
    println!(
        "{:<10} {:>12} {:>12} {:>9}",
        "variant", "plain (ms)", "cached (ms)", "speedup"
    );
    for (i, x) in [0, 50, 100, 150, 200, 250, 300, 350].iter().enumerate() {
        let sql = variant(*x);
        let (plain_time, plain_out) = timed(&plain, &sql).await;
        let (cached_time, cached_out) = timed(&cached, &sql).await;
        assert_eq!(plain_out, cached_out, "variant {i}: outputs must match");
        println!(
            "{:<10} {:>12.2} {:>12.2} {:>8.1}x",
            format!("x>{x}"),
            ms(plain_time),
            ms(cached_time),
            ms(plain_time) / ms(cached_time),
        );
    }
    let m = cache.metrics();
    println!(
        "case B metrics: hits={} committed={} entries={} bytes={}",
        m.hits, m.committed_writes, m.entries, m.bytes
    );
    assert!(
        m.hits >= 5,
        "later variants must reuse the shared aggregate: {m:?}"
    );
}
