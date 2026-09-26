# avenger-datafusion-cache

In-memory query and result caching for DataFusion, independent of the other
Avenger crates. The two cache APIs can be used separately or together.

## Prepared queries

`PreparedPlanCache::collect` accepts a `DataFrame` and optional placeholder
values. It reuses three kinds of work:

- Exact outputs when the query, input snapshots, and referenced values match.
- Optimized logical plans when referenced values change.
- Physical plan prototypes for eligible operators and parameter types.

Only parameters referenced by the query participate in the output key.
Physical prototype hits still execute the query. A typed, execution-local
parameter table supplies the current values without rebuilding the physical
plan.

Enable DataFusion's `sql` feature to run this example:

```rust
use avenger_datafusion_cache::{PreparedPlanCache, PreparedPlanCacheConfig};
use datafusion::{common::ScalarValue, prelude::SessionContext};

# async fn example() -> datafusion::error::Result<()> {
let ctx = SessionContext::new();
let cache = PreparedPlanCache::new(PreparedPlanCacheConfig::default());
let query = ctx.sql("SELECT CAST($value AS BIGINT) + 1 AS next_value").await?;
let output = cache.collect(
    query,
    Some(vec![("value", ScalarValue::Int64(Some(4)))].into()),
).await?;
assert_eq!(output.batches[0].num_rows(), 1);
# Ok(())
# }
# tokio::runtime::Runtime::new().unwrap().block_on(example()).unwrap();
```

Prepared reuse captures `MemTable` contents as immutable snapshots. Empty
sources are also supported. Other providers, non-query operations, and queries
containing stable or volatile functions use ordinary DataFusion collection.
Keys include the session, planning configuration, registered functions, and
input identities. Parameter coercions use ordinary collection. Parameter field
metadata uses literal-bound physical planning because DataFusion 54's scalar
subquery expressions omit that metadata.

DataFusion 54's reset API shares scalar-subquery results containers between
plan clones. Executions sharing one prototype therefore run serially, while
different prototypes can run concurrently. Failed or canceled executions
invalidate their prototype before it can be used again.

## Shared subtree results

`EvaluationCachePlanner` replaces previously computed physical subtrees with
cached batches. Register it after DataFusion's other physical optimizer rules:

```rust
use std::sync::Arc;
use avenger_datafusion_cache::{EvaluationCache, EvaluationCacheConfig, EvaluationCachePlanner};
use datafusion::{execution::session_state::SessionStateBuilder, prelude::SessionContext};

let cache = EvaluationCache::new(EvaluationCacheConfig::default());
let state = SessionStateBuilder::new()
    .with_default_features()
    .with_physical_optimizer_rule(Arc::new(EvaluationCachePlanner::new(Arc::clone(&cache))))
    .build();
let ctx = SessionContext::new_with_state(state);
```

Admission requires repeated observations within a configurable time window.
A write stages batches during normal execution and commits only after every
partition finishes. Errors, cancellation, repeated partition execution, and
oversized results discard the write. `set_observe_only(true)` continues to
serve hits while suspending new admissions, which can help during gestures.

Fingerprints cover physical structure, source versions, output properties,
and execution configuration. Dynamic filters, scalar-subquery references,
volatile expressions, unbounded streams, writes, and unsupported operators
exclude the affected subtree and its ancestors. Matching is structural and
process-local.

Use an evaluation cache within one stable function and object-store
configuration. Create a new cache when those registrations change. File
sources must remain consistent with their serialized metadata. A
`CacheVersionProvider` can supply a source identity and revision when metadata
is insufficient. Equal versions must identify equal partition contents.
Memory sources without a provider use memoized content hashes. Custom
fingerprinters must account for every input that can change results.

## Bounds and lifecycle

Both APIs expose configuration and metrics. Result stores have entry and byte
limits, including an entry limit for empty results. Prepared logical and
physical plans have entry limits. Least recently used entries are evicted.
Byte accounting estimates Arrow allocations and can count shared buffers more
than once. Pending subtree writes each have a per-entry limit.

`clear()` removes cached entries and observations. Work already in progress
can finish for its caller but cannot repopulate the cleared cache. The caches
retain no disk state. Source hashing runs on the calling thread and is
memoized for unchanged Arrow arrays.

The crate supports `wasm32-unknown-unknown` and uses a browser-compatible
clock. DataFusion's Arrow IPC dependency requires a WASM-capable C compiler
for zstd. The workspace compiler wrappers detect LLVM, including Homebrew
LLVM on macOS. Set browser-appropriate cache budgets before loading large
datasets. The browser target is compile-checked in CI.
