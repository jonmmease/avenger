# avenger-datafusion-cache

Automatic physical-plan result caching for [DataFusion](https://datafusion.apache.org/).

After DataFusion's physical optimization, an `EvaluationCachePlanner` walks
the plan bottom-up computing a safety-gated fingerprint per subtree, replaces
subtrees whose fingerprints have committed cache entries with `CacheReadExec`,
and wraps admitted cache misses with `CacheWriteExec` — a tee that stages
record batches while the query runs normally and commits the entry atomically
when every partition completes. Repeated physical planning requests on a
long-lived `SessionContext` (interactive parameter changes, hot reload,
dashboards, subtrees shared across queries) reuse executed results instead of
re-executing them.

## What it is not

- **Not a materialized-view matcher**: only structurally identical subtrees
  (by fingerprint) are reused; no semantic query rewriting.
- **Not a planning cache**: a hit still pays logical/physical planning and
  the fingerprint walk (measured at roughly 10% of plan+optimize time on
  mid-size plans).
- **Not persistent**: entries are in-memory, byte-budgeted with LRU
  eviction, and fingerprints are process-lifetime only.

## Safety model

Caching is fail-safe by construction. A subtree is excluded from both the
read and the write frontier when it contains:

- a runtime-mutated expression (dynamic filter pushdown from TopK, join, or
  aggregate consumers — detected by recursive snapshot generation *before*
  proto serialization, which would otherwise silently never-hit or, for
  join hash-table lookups, normalize into a false hit);
- a volatile expression (`random()` and friends);
- an unbounded source;
- a node the proto codec cannot encode.

Exclusion propagates to every ancestor. Memory-source leaves are never
proto-serialized (their proto encoding embeds the data); they are versioned
by a registered `CacheVersionProvider` chain or a memoized content hash.

## Example

```rust,no_run
use std::sync::Arc;
use avenger_datafusion_cache::{EvaluationCache, EvaluationCacheConfig, EvaluationCachePlanner};
use datafusion::execution::session_state::SessionStateBuilder;
use datafusion::prelude::SessionContext;

let cache = EvaluationCache::new(EvaluationCacheConfig::default());
let planner = EvaluationCachePlanner::new(Arc::clone(&cache));
// Register LAST so every built-in physical optimizer rule runs first.
let state = SessionStateBuilder::new()
    .with_default_features()
    .with_physical_optimizer_rule(Arc::new(planner))
    .build();
let ctx = SessionContext::new_with_state(state);
// ... register tables, run queries; repeated subtrees hit after a warm-up.
// cache.metrics() reports hits, misses, writes, evictions, and timings.
```

Admission is observation-based: a subtree is written only after its
fingerprint has been seen repeatedly within a recency window, subject to
per-entry and total byte budgets. `set_observe_only(true)` is a gesture hint
for interactive hosts: hits keep being served, no new writes are admitted.

## Using from avenger-chart

Chart hosts should not wire this crate by hand:
`avenger_chart::physical_cache` provides `cached_session_context()` /
`install_physical_cache()` (rule + `SessionConfig`-extension discovery),
the `AVENGER_PHYSICAL_CACHE=0` kill switch, preview-mode observe-only
policy, and per-evaluation metric deltas on `EvaluationMetrics`. The
visual regression suite can run entirely through the cache with
`AVENGER_PHYSICAL_CACHE_CENSUS=1` (see `avenger-chart/docs/DEBUGGING.md`).

## Design

The full design document lives at
`avenger-chart/docs/future-work/physical-plan-evaluation-cache.md`; the
implementation plan of record is `scratch/datafusion-cache-crate-plan.md`.
This crate is v1 of that design: memory-only, exact property compatibility,
conservative dynamic-filter exclusion, and a concrete cache type. Spill,
property adapters, cost-aware admission, and wasm32 builds are deliberate
follow-ups.

This crate depends only on DataFusion, Arrow, and `futures` — no `avenger-*`
crates.
