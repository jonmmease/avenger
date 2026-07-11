# Physical-Plan Evaluation Cache

## Status

Phase 1 BUILT (2026-07-10): the `avenger-datafusion-cache` workspace crate
implements the memory-only prototype exactly as phased below — safety-gated
proto-bytes fingerprints, `CacheReadExec`/`CacheWriteExec` with single-flight
pending entries, LRU byte budget, observe-then-admit policy — with a
cache-on/off equivalence census and a measured fingerprint overhead of ~10%
of plan+optimize time (~100 µs on chart-scale plans). Implementation plan of
record: `scratch/datafusion-cache-crate-plan.md`. Phase 2 (Avenger
integration) BUILT 2026-07-10 per
`scratch/physical-cache-avenger-integration-plan.md`: blessed session
wiring (`avenger_chart::physical_cache`, `AVENGER_PHYSICAL_CACHE=0` kill
switch, `SessionConfig`-extension discovery), scene-byte equivalence suite,
store versioning resolved by revision-in-data (no provider needed),
preview-mode observe-only policy + per-evaluation metrics deltas, and the
full visual suite byte-identical WITH the cache enabled
(`AVENGER_PHYSICAL_CACHE_CENSUS=1`, 729/729) — the census caught and fixed
one real false-hit class (DF54 `ScalarSubqueryExpr` serializes as a bare
results index; now a third deep-guard exclusion). Compile-time
cache-boundary hints: measured and declined for now (see the dated NO-GO
memo in the integration plan; per-mark chain specialization is real but
worth single-digit milliseconds at chart scale). Phases 3-5 below
(cost-aware admission, spill, generalization) remain future work. This note focuses on a
physical-plan-only cache for
DataFusion execution results. The design is useful for Avenger chart sessions,
but it must not depend on Avenger-specific types: the implementation target is
an `avenger-datafusion-cache` workspace crate whose dependencies are DataFusion
crates only, with Avenger behavior plugging in through the trait seams described
below (fingerprinter/extension codec, source versions, cache-boundary
insertion, clock, admission hints). The motivating use cases are:

- regular interactive charts where params, stores, selections, or view domains
  change over time;
- repeated dashboard and notebook queries over the same sources;
- multiple marks or mark groups that compile to overlapping DataFusion work;
- online editing and hot reload, where style changes should not force
  re-execution of data queries. The cache is a necessary layer for this use
  case but not the whole answer; see "Hot Reload Scope" below.

Reviewed 2026-07-10 against the landed bake layer
(`avenger-datafusion-partial-eval` + `CompiledPlot::bake`; see
[logical-plan-partial-evaluation.md](logical-plan-partial-evaluation.md), now
built): the design holds, and the bake campaign strengthened its motivation.
Baked artifacts deliberately leave work live that only a runtime cache can
capture — faceted group chains stay live per cell even when param-free
(shared-scale domain inference evaluates chains at sharing-owner scope, so
per-cell folds are semantically unreachable at build time), and every
param-dependent residual replays on each interaction. A baked interactive
session is therefore exactly this cache's target workload: recurring physical
subtrees over stable `MemTable` leaves. As-built bake facts are folded in
below where they touch the design (memory-leaf fingerprints, source versions,
proto codec feature gates, the decode memo). The implementation plan of
record for phase 1 (the crate) is `scratch/datafusion-cache-crate-plan.md`.

The key choice is to work after physical planning. Cache hits become
`CacheReadExec` nodes. Cache writes become `CacheWriteExec` nodes that tee the
normal `RecordBatch` stream into the cache while still passing batches upstream.

## Non-Goals

This is not a materialized-view matching system. It does not try to prove that a
different query can be answered from an existing cached aggregate. It only reuses
physical subtrees that have compatible fingerprints and output properties.

This is also not a replacement for DataFusion's existing file-listing, file
metadata, or Parquet statistics caches. Those caches sit near source discovery
and pruning. This proposal caches executed physical subplan outputs.

This is also not a planning cache. A cache hit still pays chart compilation,
logical plan construction, logical optimization, physical planning, physical
optimization, and the fingerprint walk. Workloads whose latency is dominated by
planning rather than execution need different mechanisms (plan reuse, spec
diffing, or higher-level artifact caches) and should not expect much from this
design.

This is also not build-time precomputation. Folding param-free plan subtrees
into materialized tables on a server and shipping the result inside the
compiled artifact is a separate, complementary layer that has since shipped
as `avenger-datafusion-partial-eval` + `CompiledPlot::bake`; see
[logical-plan-partial-evaluation.md](logical-plan-partial-evaluation.md).
Baked `MemTable` scans appear to this cache as ordinary memory-source leaves.

Logical-plan fingerprints may be useful later for observability or broader
semantic reuse, but the first design should not require them.

## Prior Art

Several existing systems are adjacent:

- DataFusion has a [`CacheManager`](https://docs.rs/datafusion/latest/datafusion/execution/cache/cache_manager/struct.CacheManager.html)
  for file listings, file metadata, and file statistics caches.
- DataFusion `DataFrame::cache()` eagerly materializes a full `DataFrame` into a
  `MemTable`; issue [#17297](https://github.com/apache/datafusion/issues/17297)
  discusses making cache behavior more explicit and extensible.
- DataFusion issue [#12779](https://github.com/apache/datafusion/issues/12779)
  proposed `datafusion-query-cache` for repeated intermediate results.
- DataFusion issue [#22676](https://github.com/apache/datafusion/issues/22676)
  tracks physical subplan materialization work, especially for CTEs.
- DataFusion issue [#15585](https://github.com/apache/datafusion/issues/15585)
  discusses a predicate cache and points at admission policies that avoid
  caching first hits.
- [`datafusion-materialized-views`](https://github.com/datafusion-contrib/datafusion-materialized-views)
  is a stronger semantic-query-rewrite direction, complementary to this design.
- PostgreSQL [`shared_buffers`](https://www.postgresql.org/docs/current/runtime-config-resource.html)
  and [materialized views](https://www.postgresql.org/docs/current/rules-materializedviews.html)
  cover lower-level page caching and explicit persisted query results.
- DuckDB's [memory manager](https://duckdb.org/2024/07/09/memory-management.html)
  describes a buffer manager that caches persistent pages and coordinates memory
  pressure with query intermediates.

The design here is narrower than materialized views and higher level than page
caches: exact physical subtree result caching with execution-time tee writes.

## Physical Planning Flow

The normal DataFusion planning path stays intact until physical planning has
produced an executable plan:

```text
LogicalPlan
  -> DataFusion logical optimization
  -> DataFusion physical planning
  -> DataFusion physical optimization
  -> EvaluationCachePlanner
       - compute physical fingerprints
       - choose cache-read frontier
       - choose cache-write frontier
       - rewrite with CacheReadExec / CacheWriteExec
  -> optional final physical normalization
  -> execute
```

`EvaluationCachePlanner` can be implemented as a physical optimizer rule or as a
thin wrapper around physical planning. The important constraint is that cache
substitution happens before execution and after enough physical optimization has
occurred that the chosen subtrees represent real executable work.

The cache planner runs after all built-in physical optimizer rules. The first
implementation should be conservative: only substitute cached results when
`CacheReadExec` can report the same schema, partitioning, ordering, boundedness,
and equivalence properties as the original subtree, or a clearly compatible
subset of those properties. Any final normalization after substitution should be
minimal — property-compatibility adapters only, not a re-run of the optimizer
pipeline. Re-optimizing around substituted nodes can rearrange the plan and
invalidate the fingerprint-to-plan correspondence that was just computed.

## Fingerprints

The cache key is a physical subtree fingerprint, not a logical query identity.
For cache reuse, subtree identity is more useful than path identity. A parent
fingerprint can still be recorded for telemetry, but it should not be required
for the cache key because the same subtree may be reused under different parents.

```text
subtree_fingerprint = hash(
    datafusion_version_or_key_version,
    execution_config_fingerprint,
    node_kind,
    normalized_local_physical_node,
    normalized_physical_exprs,
    output_schema,
    output_partitioning,
    output_ordering,
    boundedness,
    child_subtree_fingerprints,
    source_versions,
    udf_versions_and_volatility,
)
```

The local physical node fingerprint should include operator-specific details:

- projection expressions and aliases;
- filter predicates;
- aggregate groups, aggregate expressions, and modes;
- join type, join keys, join filters, and join algorithm when relevant;
- sort expressions, limits, windows, repartitioning, coalescing, and unions;
- scan projection, pushed filters, file groups, source table identity, and scan
  options;
- scalar function, aggregate function, window function, and UDF identifiers.

It should exclude runtime-only details such as metrics, object addresses,
display names that are not semantic, and generated IDs that change between
equivalent plans.

`ExecutionPlan` implements neither `Eq` nor `Hash`, so fingerprinting requires
per-node handling one way or another. A pragmatic first fingerprinter is a hash
of `datafusion-proto` physical-plan bytes — with three validated constraints:

- **Node-local serialization is required, not optional.** `MemoryScanExecNode`
  proto-encodes the full partition data (`repeated bytes partitions`), so
  hashing whole-subtree proto bytes is O(data) per walk for any plan over a
  `MemTable`. Fingerprint each node with its children replaced by
  `EmptyExec` placeholders and combine with child fingerprints; never
  proto-serialize memory-source leaves at all (they fingerprint as
  hash(schema, projection, sort info, fetch, source version)).
- **Dynamic filters must be excluded before serialization.** The proto
  converter serializes `DynamicFilterPhysicalExpr` successfully — including
  its runtime state and a process-global expression id — producing
  silently-never-hitting fingerprints rather than errors. Intercept via a
  custom `PhysicalProtoConverterExtension` (`physical_expr_to_proto` hook)
  with a deep recursive guard, since the default converter recurses past the
  hook for child expressions.
- Physical expressions implement `DynEq`/`DynHash` and can be hashed
  directly, subject to the runtime-mutated-state caveats below.

Avenger already maintains logical-plan extension codecs and uses hashed proto
bytes for materialization keys, so the physical codec is the main new piece.
Hand-written per-node normalization can replace proto hashing later if proto
encoding shows further unstable details (prost map encoding of schema
metadata is one known miss-only instability).

Proto codec coverage is also cargo-feature-gated, and a missing arm fails
with a misleading error rather than an obviously-missing-feature one: the
bake campaign lost datafusion-proto's ListingTable/`ParquetFormat` logical
arm to a `default-features = false` dependency rewrite and spent a diagnosis
cycle on the resulting "bug in DataFusion" message (restored in `f7f52ed5f`,
target-gated for native builds). The physical codec needs the same per-target
feature audit, and the fingerprinter should treat an unserializable node as
non-cacheable (fail-safe miss), never as an error that blocks execution.

At the physical layer, chart params will usually appear as literal values or as
bound physical expressions. Store values will usually appear as a table provider,
memory table, or join input with its own revision. That is fine: changing a param
or store should invalidate the physical subtrees that actually depend on it, and
leave independent upstream or sibling subtrees available for reuse.

## Source And Context Versions

Physical fingerprints need stable invalidation inputs. The cache should require
source providers to expose version identity where possible:

```rust
trait CacheVersionProvider: Send + Sync {
    /// Resolve a version for a source node, or None if unrecognized.
    fn source_version(&self, plan: &dyn ExecutionPlan) -> Option<CacheVersion>;
}
```

The trait is a physical-layer *resolver* rather than a method on table
sources, because `TableProvider`s are not reachable from physical plans.
Providers register on the cache and are consulted in order; the built-in
fallback for memory sources is a content hash of the partitions (memoized by
column-`ArrayRef` pointer identity), which is also the version story for
baked tables produced by `avenger-datafusion-partial-eval`. File scans need
no provider — their proto bytes already carry paths, sizes, and mtimes.

The landed bake layer turns two aspects of that fallback from nice-to-haves
into requirements:

- Baked table names are non-deterministic by design: every bake generates a
  unique registration prefix (`__pe_baked_{12 hex}_`,
  `unique_bake_table_prefix()` in `avenger-chart/src/bake.rs`) so artifacts
  from different bakes — including re-bakes of the same chart — can register
  side by side in one consuming context. Memory-leaf fingerprints must
  therefore exclude the registered table name and key on content; otherwise a
  re-baked artifact with identical data never hits the previous artifact's
  entries. (Within one artifact the prefix is fixed at bake time, so replans
  of the same compiled plot see stable names either way.)
- The pointer-identity memoization is effective in practice because baked
  manifest entries decode once per compiled plot into a cached
  `Arc<MemTable>` (`BakedTableManifestEntry::mem_table`, a serde-skipped
  `Arc<OnceLock<Arc<MemTable>>>`): replans within a session see the same
  `ArrayRef`s, so the content hash is computed once, and a freshly
  deserialized artifact pays one hash per table.

Possible source versions:

- table snapshot IDs;
- object-store ETags and file sizes;
- file modification metadata plus size;
- explicit application epochs;
- content hashes for small in-memory tables or selection stores;
- catalog revision IDs supplied by a server.

The execution context also matters. Include any configuration that can change
semantics or physical compatibility, such as timezone, SQL options, extension
registry versions, UDF versions, and nondeterminism policy.

UDF identity (as built): `datafusion-proto` encodes scalar and aggregate
UDFs by NAME plus arguments and return type (its codec encode hooks are
no-ops), so a fingerprint identifies a UDF by its registered name.
Re-registering a DIFFERENT function under an unchanged name mid-session is
therefore unsupported while the cache is enabled — that is registration
discipline the host already owes the cache for tables, extended to
functions. Avenger registers its scale and transform UDFs once per process
with stable semantics per build, which satisfies the contract; hosts that
hot-swap UDFs must `clear()` the cache or version via a provider.

Plans that call nondeterministic functions should be non-cacheable by default.
If the application supplies an evaluation epoch for values such as "now", the
epoch can become part of the key and make those expressions deterministic within
the session.

## Runtime-Mutated Plan State

DataFusion mutates parts of the physical plan during execution, which breaks
the assumption that a subtree's output is a pure function of its pre-execution
structure. The main case is dynamic filter pushdown
(`datafusion.optimizer.enable_dynamic_filter_pushdown`, default `true`): TopK,
join, and aggregate operators push a `DynamicFilterPhysicalExpr` into
descendant scans and tighten it while the query runs.

Dynamic filters defeat both obvious fingerprinting strategies:

- `DynamicFilterPhysicalExpr` implements `Hash`/`PartialEq` by pointer identity
  of its shared state (DataFusion issue #19641), so a `DynEq`/`DynHash`-based
  fingerprint differs on every replan. Subtrees containing one would silently
  never hit.
- A normalizing fingerprinter (hashing the pre-execution snapshot, which is
  `lit(true)`) produces false hits. A scan pruned by a TopK `LIMIT 10` parent
  captures output that provably contains the top 10 but not necessarily the
  top 20. The `k` lives in the parent, and the cache key deliberately excludes
  parent identity, so the same fingerprint under a `LIMIT 20` parent would
  replay incomplete data. Join and aggregate dynamic filters have the same
  failure shape.

The capture side is equally affected: `CacheWriteExec` output collected below a
dynamic-filter consumer is only valid for the specific run that pruned it.

The rule: any subtree containing a physical expression with a non-zero
`snapshot_generation()` (DataFusion's marker for execution-time-mutable
expressions) is excluded from both the read frontier and the write frontier. A
subtree that contains the consumer itself (for example, the TopK) above every
dynamic filter it feeds is safe to cache, because the consumer's output is
deterministic. An alternative is disabling dynamic filter pushdown when the
cache is enabled, trading scan pruning for cacheability; that should be a
measured decision, not the default.

Volatile functions are related but simpler. `PhysicalExpr::is_volatile`
identifies `random()`-style expressions, which make their subtree
non-cacheable. `now()` is const-folded to a literal during logical
optimization, so it is safe (no false hits) but useless (a new fingerprint per
statement); the evaluation-epoch mechanism above is the fix when such
expressions should hit within a session.

## Cache Entries

```rust
struct PhysicalCacheKey {
    subtree_fingerprint: PlanFingerprint,
    schema_fingerprint: SchemaFingerprint,
    partitioning_fingerprint: PartitioningFingerprint,
    ordering_fingerprint: OrderingFingerprint,
    boundedness: Boundedness,
}

struct PhysicalCacheEntry {
    key: PhysicalCacheKey,
    schema: SchemaRef,
    statistics: Statistics,
    properties: CachedPlanProperties,
    partitions: CachedPartitions,
    actual_rows: usize,
    actual_bytes: usize,
    created_at: Instant,
    last_accessed_at: Instant,
    hits: u64,
    estimated_cost_saved: f64,
}

enum CachedPartitions {
    Memory(Vec<Vec<RecordBatch>>),
    Spill(Vec<SpillPartitionRef>),
}
```

The first implementation can store in-memory Arrow batches. A general project
should add memory-pool integration and spill to Arrow IPC or Parquet. Cache
entries should preserve partition boundaries so a `CacheReadExec` can expose the
same partition count and avoid accidental single-partition bottlenecks.
Timestamps should come from an injectable clock (`std::time::Instant` needs a
shim on `wasm32`).

Eviction should be memory bounded from the first implementation, with a hard
byte budget. LRU is a reasonable prototype policy, but an interactive cache
should eventually use an admission-aware policy such as 2Q, S3-FIFO, TinyLFU,
or Foyer-like admission to avoid filling memory with one-hit entries.

## CacheReadExec

`CacheReadExec` is an `ExecutionPlan` that reads committed cached partitions.
It should expose:

- the cached schema;
- exact `Statistics` computed from the materialized partitions (row and byte
  counts are known, strictly better than the original subtree's estimates);
- output partitioning;
- output ordering when preserved;
- boundedness;
- equivalence properties when they remain valid;
- one output stream per cached partition.

The first version should require exact property compatibility. Later versions can
add adapters:

- project fewer columns from a wider cached result;
- cast compatible types;
- repartition cached output;
- sort cached output if ordering is required but not preserved.

Adapters make reuse more flexible, but they also complicate cost modeling. They
should be explicit physical nodes above `CacheReadExec`, not hidden behavior.

## CacheWriteExec

`CacheWriteExec` wraps a child `ExecutionPlan`:

```text
child ExecutionPlan
  -> CacheWriteExec
       - yield each RecordBatch upstream immediately
       - clone or retain each RecordBatch for the cache writer
       - spill if memory pressure requires it
       - commit the cache entry only after all partitions complete
  -> parent ExecutionPlan
```

This is the main reason to work at the physical execution layer. An admitted
miss can populate the cache while the query runs normally. The subtree does not
need to be executed a second time.

The write path should be transactional:

- each output partition writes to a per-partition staging buffer;
- successful partition completion marks that partition ready in staging;
- successful completion of all partitions commits the entry atomically;
- errors, cancellation, or dropped streams discard staging data;
- cache-write errors should normally fail open and let the query continue unless
  the caller explicitly requested mandatory caching.

Partitions execute as independent streams, so the commit barrier is shared
state across partition writers. It must tolerate partitions that are never
executed (the entry simply never commits) and `execute(partition)` being called
more than once for the same partition.

Downstream early termination is important. If a parent `LIMIT` stops pulling from
the stream before the child subtree completes, `CacheWriteExec` has only a
partial result and must discard it. If a limit is part of the child subtree being
fingerprinted, then the limited result is complete and cacheable.

Backpressure should be explicit. A synchronous writer is simplest and naturally
slows the query when caching is expensive. An async writer can reduce latency,
but it needs a bounded queue tied to DataFusion memory accounting; otherwise the
cache can become an untracked memory sink.

The entry lifecycle needs a pending state between "write began" and
"committed". Without one, two concurrent plans containing the same admitted
subtree both miss and both execute — and concurrency exists today (background
materializations run in parallel) and grows in client-server deployments. A
lookup that finds a pending entry should be able to wait for the in-flight
write (single-flight) or fall back to executing independently without admitting
a second write.

## Hit And Write Frontiers

Cache lookup should choose a non-overlapping hit frontier:

1. Walk the physical plan from root to leaves.
2. Compute child fingerprints bottom-up.
3. When a subtree has a valid committed cache hit, replace that subtree with
   `CacheReadExec` and stop walking its children entirely — descendants of a
   hit are neither matched nor observed, so their seen-counts do not inflate
   while they are shadowed by the parent's entry.
4. If a subtree has no hit, continue into its children.

Write admission should choose a separate non-overlapping frontier among misses:

1. Exclude subtrees already replaced by `CacheReadExec`.
2. Exclude unbounded, nondeterministic, runtime-mutated, very small, or
   unsupported nodes.
3. Prefer candidates with repeated recent fingerprints.
4. Prefer candidates with high observed cost and manageable output size.
5. Avoid caching both an ancestor and descendant in the same run unless the
   descendant is reused by another consumer.

The root can be a valid cache entry, but regular charts often benefit from
intermediate entries because small downstream changes can still reuse upstream
work.

## Admission And Cost Model

The planner should distinguish observation from admission:

- observation records fingerprints, estimated statistics, and later runtime
  metrics;
- admission decides whether a future occurrence should be wrapped with
  `CacheWriteExec`;
- reads occur only from committed entries (or pending entries via
  single-flight).

A simple starting policy:

```text
admit when:
  recent_seen_count >= 2
  estimated_or_observed_elapsed_ms >= min_elapsed_ms
  estimated_output_bytes <= max_entry_bytes
  plan_is_bounded
  plan_is_deterministic
```

"Recent" needs a real definition (a sliding window or decayed counter), and the
observation table itself must be size-bounded, or it becomes a leak under
continuous interaction. During a continuous gesture (pan, zoom, brush), every
frame produces first-occurrence fingerprints for viewport-dependent subtrees;
none of them should be admitted mid-gesture. The planner should accept an
interaction hint (gesture-active: observe only, no writes) or an equivalent
debounce, so viewport-dependent admission waits for the interaction to settle.
Hits during a gesture still come from the stable upstream subtrees.

A richer policy can estimate:

```text
expected_value =
    expected_future_hits * observed_subtree_cost
  - write_cost
  - read_cost
  - memory_pressure_penalty
  - spill_penalty
```

Useful high-value candidates:

- scans over remote or slow storage after projection/filter pushdown;
- selective filters that reduce large inputs;
- joins reused by multiple marks;
- aggregates used by multiple scales, legends, or layers;
- sorts, windows, and repartition-heavy stages;
- expensive UDF/UDAF outputs, including rasterization.

Low-value candidates:

- cheap projections and aliases;
- tiny inputs;
- highly volatile param-dependent filters that rarely repeat;
- very wide outputs when downstream needs only a few columns;
- unbounded streams.

## DataFusion Statistics And Runtime Signals

DataFusion exposes plan information that can feed the cost model:

- [`Statistics::num_rows`](https://docs.rs/datafusion/latest/datafusion/common/struct.Statistics.html);
- `Statistics::total_byte_size`;
- `Statistics::column_statistics`;
- [`ColumnStatistics::byte_size`](https://docs.rs/datafusion/latest/datafusion/common/struct.ColumnStatistics.html);
- column null counts, min/max, sum, and distinct counts when available;
- [`ExecutionPlan::partition_statistics`](https://docs.rs/datafusion/latest/datafusion/physical_plan/trait.ExecutionPlan.html);
- `ExecutionPlan::metrics()` after execution;
- [`ExecutionPlanProperties::output_partitioning`](https://docs.rs/datafusion/latest/datafusion/physical_plan/trait.ExecutionPlanProperties.html);
- `ExecutionPlanProperties::output_ordering`;
- `ExecutionPlanProperties::boundedness`;
- `ExecutionPlanProperties::pipeline_behavior`;
- `ExecutionPlanProperties::equivalence_properties`;
- physical pushdown behavior for limits, projections, and filters.

These statistics may be exact, inexact, or absent. They should be treated as
hints until runtime metrics and actual `RecordBatch` memory sizes are available.
DataFusion's [configuration](https://datafusion.apache.org/user-guide/configs.html)
also includes statistics collection and display options, plus a pluggable
statistics registry that may improve estimates.

## Cache Placement And Lifetime

The cache must be owned by the long-lived execution context, not by per-chart
session objects. In Avenger terms: a chart spec edit recompiles the plot and
recreates the `PlotSession` along with every session-scoped cache, while the
DataFusion `SessionContext` survives. A physical cache attached to the
`PlotSession` would lose exactly the reuse the hot-reload use case needs, so
the handle belongs on the `SessionContext` or an `Arc` shared above it.

Consequences:

- One context hosting several charts shares one cache. That is desirable for
  dashboards (shared scans and aggregates hit across charts) but makes the
  memory budget a cross-chart resource.
- Cache lifetime exceeds compiled-plot lifetime, which is why fingerprints and
  source versions must be self-contained: no object identity from the compiled
  plot may leak into keys.
- Reuse across replans requires registration discipline from the host: the
  same logical tables must be re-registered with the same identities and
  version behavior after a reload, or scans fingerprint differently and
  nothing hits.

## Params, Stores, And Interactive Charts

Interactive Avenger charts should look like ordinary repeated physical planning
requests against a context-scoped cache.

Baked charts sharpen this picture. After `CompiledPlot::bake`, everything
param-free that can fold has already folded into embedded tables, so the
remaining runtime work is by construction the recurring kind: param-dependent
residuals above baked scans, plus faceted per-cell chains, which stay live
even when param-free — the bake system cannot fold them, because shared-scale
domain inference evaluates group chains at sharing-owner scope, making
per-cell folds semantically unreachable at build time (see
[logical-plan-partial-evaluation.md](logical-plan-partial-evaluation.md)). A
faceted baked chart re-executes N per-cell subtrees over the same baked leaf
on every interaction and every hot-reload replan; subtrees untouched by the
changed param are exact repeats. No build-time layer can capture that
recurrence — this cache is the layer that does.

Params should become physical constants or bound physical expressions. When a
param changes, only subtrees containing that param and their ancestors should get
new fingerprints. Unrelated scans, joins, or aggregates can still hit.

Stores and selections should have revisioned table identities. A lasso selection
store, for example, may be represented as a small table whose content hash or
revision is part of the store scan fingerprint. Changing the store invalidates
joins or filters that consume it, but not independent source work. Avenger's
scoped store tables already maintain per-instance revision counters, which are
ready-made `CacheVersionProvider` inputs.

View-domain changes are just param changes. For view-dependent rasterization,
each viewport may produce a different physical fingerprint. That is acceptable:
the cache can still help when users return to a recent viewport, when several
marks share the same viewport-dependent subplan, or when upstream static work is
independent of the view params.

Filter pushdown deserves care. If a parameterized filter is pushed into a scan,
the scan fingerprint changes with the param. That is usually semantically correct.
If we later need reuse of the unfiltered source scan, that is a different cache
boundary: either a lower-level source/page cache, or an explicit cache-boundary
node in the plan (see "Column Width, Projection, And Cache Boundaries").

## Hot Reload Scope

Hot reload is where this cache is structurally unique and also where it is
least sufficient on its own. Both halves deserve stating.

Unique: every Avenger session cache dies on a spec edit because keys are object
pointers into the compiled plot (`marks_ptr`, `program_ptr`, `data_ptr`), which
cannot survive recompilation by construction. A physical cache keyed on plan
structure plus source versions survives recompilation for free. It is the only
layer in this design space that does.

Not sufficient:

- A hit still pays compilation, logical planning and optimization, physical
  planning and optimization, and fingerprinting — per query, and per
  measurement refinement pass. For execution-heavy charts (large scatter,
  rasterization) that cost is noise; for a modest faceted chart issuing dozens
  of small domain and slot queries, planning can dominate the reload. The
  design spike should measure plan+optimize+fingerprint latency on a
  representative mid-size chart before treating hot reload as served.
- Derived and presentation artifacts still rebuild: scale builders, partition
  slot values, guide overflow profiles, legend measurements, and layout.
  Making those survive a reload is a separate workstream — moving session
  cache handles to a reload-surviving owner and replacing pointer keys with
  structural keys (the serialization infrastructure already exists and is
  already used in materialization keys). The text measurement cache key is
  already fully structural and only needs the longer-lived home.

Style-only edits then decompose cleanly: the physical cache makes the data work
free minus planning, and the survivable-artifact workstream makes the derived
work free. Channel, transform, param, and store edits still replan, and the
physical cache lets unchanged subtrees survive those replans.

## Avenger Caches To Re-Evaluate

Once a physical DataFusion result cache exists, Avenger should audit its current
ad-hoc cache layers. The goal is not to delete every cache. The goal is to stop
each chart subsystem from inventing its own data-query key when the physical
cache can provide the same reuse with better invalidation.

| Current cache | Current role | Re-evaluate after physical cache |
| --- | --- | --- |
| `mark_group_data_cache` on `EvaluationContext` | Per-evaluation memo of prepared mark-group base data: a lazy `DataFrame` plus derived scalars and facet scope. Each consumer's collect re-executes the chain today; the memo avoids rebuilding the plan and re-running derived-scalar seeding, not re-executing results. | Keep. There is no result reuse here for the physical cache to take over — the physical cache adds cross-collect result reuse that does not exist today. The memo stays valuable as cheap plan construction and scalar seeding carrying semantics the physical cache cannot. |
| `group_view_data_cache` on `EvaluationContext` | Per-evaluation memo for group view-local transform chains shared by children (lazy `DataFrame`, derived scalars, view params). | Keep, for the same reasons. Cross-child and cross-evaluation result reuse becomes a physical-cache job, but only with an explicit cache boundary (see below): children project different columns, so after projection pushdown the "shared" subtrees are not structurally identical. Keep view-param resolution, derived-scalar seeding, and scheduling semantics outside the cache. |
| `PartitionSlotCache` | Session cache of facet observed/domain slot values, string-keyed on the `Debug` format of the full logical plan plus filters and params. | Replace the query reuse with physical cache entries. The current key embeds no source versions, so it can serve stale slots if a registered table's content changes mid-session; physical fingerprints with source versions fix that class. Keep a thin derived-value memo (`Vec<ScalarValue>` per fingerprint) so the common path stays a lookup with no planning. |
| `ScaleDomainCache` | Session cache of `ScaleBuilder` domain artifacts keyed by compiled-object pointers, scope, and stringified params. | Split derived scale-artifact reuse from DataFusion query reuse. The physical cache owns extent/distinct/aggregate query results; the assembled `ScaleBuilder` stays an Avenger cache. Its pointer keys cannot survive recompiles, so if scale artifacts should survive hot reload they need structural keys regardless of this design. |
| `MaterializationCache` ready payloads | Stores queued/running/ready/error materialization entries and ready `MaterializationResult` payloads. Keys are already versioned structural hashes of the proto-serialized plan, transform, and params — computable without planning. | Keep scheduling, debounce/throttle, latest-ready identity, stale fallback, and invalidation, and keep the logical key for schedule-time identity (it must remain computable without planning). Ready payloads can move into the physical cache only once entries are pinnable: stale fallback requires the payload to still exist when policy reaches for it, so eviction must not be able to break scheduling semantics. |
| `BakedTableManifestEntry` decode memo | Per-manifest-entry `OnceLock` that decodes embedded Arrow IPC bytes into an `Arc<MemTable>` once per compiled plot. | Keep. This is deserialization memoization, not query-result caching — nothing executes. Its stable `Arc`/`ArrayRef` identity is also what makes memory-leaf content-hash versioning cheap for this cache (see "Source And Context Versions"). |
| Future owner-scope transform caches | Proposed future cache for transformed owner-scope tables and derived scalar maps. | Prefer physical subplan caching before adding another bespoke transform-table cache. |

Likely caches to keep because they are not DataFusion result caches:

- guide overflow cache;
- legend measurement cache;
- text measurement cache;
- layout profile and rendered component preview reuse;
- facet scale-builder precompute artifacts;
- image resource cache, map tile cache, GPU texture cache, and text raster cache.

Those caches answer presentation, layout, resource-loading, or renderer questions.
The physical DataFusion cache should feed them, not replace them. Note that the
guide overflow, legend measurement, and facet scale-builder precompute caches
are pointer-keyed and session-bound: if hot reload is a goal, they need the same
survival treatment as the data layer (structural keys plus a reload-surviving
owner). The text measurement cache key is already structural and only needs the
longer-lived home.

## Column Width, Projection, And Cache Boundaries

Caching a wide subtree can waste memory if downstream consumers only need a few
columns. The physical plan already reflects DataFusion projection pushdown, so
the best first rule is to cache only after normal projection optimization.

Further improvements:

- prefer cache candidates whose output schema is already narrow;
- inspect parent column requirements before admitting a wide candidate;
- use DataFusion projection-swapping helpers where legal;
- allow `CacheReadExec` plus a projection adapter for reading a subset of a
  wider cached result;
- record per-column byte sizes to predict memory pressure.

Projection pushdown also works against cross-consumer reuse. Marks that share a
transform chain rarely need the same columns, so after pushdown their "shared"
subtrees are no longer structurally identical and never hit each other. Relying
on the optimizer to leave identical subtrees behind makes sharing a
coincidence.

The chart compiler knows the sharing statically — shared chains are lowered
onto children at compile time — so it should say so in the plan: an explicit
cache-boundary hint node at the shared-chain root pins a stable projection,
blocks pushdown through it, and turns the chain into a guaranteed cache unit.
The cost is a wider scan than any single consumer needs; the planner can weigh
that against the fan-out. The same mechanism covers the explicit pre-filter
scan boundary mentioned above. This moves the design from a purely transparent
cache toward a cooperative one, and it is the highest-leverage Avenger-specific
decision in this project.

## Failure And Replanning Semantics

Cache reads should be validated before execution. If an entry is missing,
evicted, corrupted, or incompatible, the planner should treat it as a miss and
produce the normal uncached subtree.

If a cache read fails during execution, the safest behavior depends on where this
is implemented:

- in a wrapper outside DataFusion, replan without the cache and retry;
- inside a physical optimizer, prefer pre-execution validation so execution-time
  read failures are rare;
- for interactive charts, fail open is preferable when possible because the
  cache is an optimization, not the source of truth.

Cache writes should never expose partial results as committed entries.

## WebAssembly Constraints

Avenger runs DataFusion in the browser, so the cache must work on
`wasm32-unknown-unknown`:

- no filesystem spill: `spill_dir` is native-only and the wasm cache is
  memory-only at every phase;
- the 32-bit address space makes the hard byte budget with eviction a phase-1
  requirement, not a later refinement;
- timestamps need an injectable clock (`std::time::Instant` requires a shim);
- async surfaces (single-flight waits, the write path) need a `?Send` variant,
  following the workspace's existing `async_trait(?Send)` split; this is a
  phase-1 API decision, not a retrofit;
- execution is single-threaded, which makes partition replay trivial but puts
  fingerprinting and cache maintenance on the interaction thread, so their
  per-plan cost must be budgeted.

## Possible API Shape

```rust
struct EvaluationCacheConfig {
    max_memory_bytes: usize,
    max_entry_bytes: usize,
    spill_dir: Option<PathBuf>,
    min_elapsed_for_admission: Duration,
    admission_policy: AdmissionPolicy,
}

trait PhysicalPlanFingerprinter {
    fn fingerprint(
        &self,
        plan: Arc<dyn ExecutionPlan>,
        children: &[PlanFingerprint],
        ctx: &FingerprintContext,
    ) -> Result<PlanFingerprint>;
}

enum CacheLookup {
    Miss,
    Pending(CacheWaitHandle),
    Ready(PhysicalCacheEntryRef),
}

trait EvaluationCache {
    fn lookup(&self, key: &PhysicalCacheKey) -> CacheLookup;
    fn observe(&self, observation: PlanObservation);
    fn should_admit(&self, candidate: &CacheCandidate) -> AdmissionDecision;
    fn begin_write(&self, candidate: CacheCandidate) -> CacheWriteHandle;
}

struct EvaluationCachePlanner {
    cache: Arc<dyn EvaluationCache>,
    fingerprinter: Arc<dyn PhysicalPlanFingerprinter>,
    config: EvaluationCacheConfig,
}
```

The prototype can keep the fingerprinter small and explicitly support the
physical nodes Avenger emits most often. A general DataFusion project would need
a registry (or proto codec coverage) so extension `ExecutionPlan` nodes and UDFs
can participate without being serialized through brittle display strings.

As built (v1, `avenger-datafusion-cache`): `EvaluationCache` shipped as a
CONCRETE type — the trait above is deferred until a second implementation
exists — with `CacheVersionProvider` in the resolver shape, exact property
compatibility only, the conservative dynamic-filter rule (consumer-above-
filters not special-cased), memory-only storage, and no wasm build yet; all
per this document's phase 1. The proto-bytes fingerprinter is the default;
`PhysicalPlanFingerprinter` is the extension seam.

## Implementation Phases

1. Memory-only prototype (new `avenger-datafusion-cache` crate, DataFusion-only
   dependencies):
   - exact physical subtree fingerprints;
   - exclusion of runtime-mutated (dynamic filter) and volatile subtrees;
   - exact property compatibility;
   - `CacheReadExec`;
   - `CacheWriteExec` with pending-entry single-flight;
   - hard memory budget with LRU eviction;
   - no spill;
   - conservative admission after a second recent occurrence;
   - exit criterion: measured plan + optimize + fingerprint latency on a
     representative mid-size chart plan set, to bound the hit-path cost.
2. Avenger integration:
   - context-scoped cache handle that survives `PlotSession` recreation;
   - param/store/source revision fingerprints (store tables already carry
     per-instance revisions; materialization specs already carry versions);
   - compile-time cache-boundary hints for shared mark and group-view chains;
   - metrics for hits, misses, admitted writes, evictions, bytes, and latency;
   - examples covering repeated params, selection stores, and shared mark work;
   - re-key or retire existing Avenger data-result caches per the table above.
3. Cost-aware admission:
   - use DataFusion statistics and runtime metrics;
   - avoid wide/cheap candidates;
   - interaction-aware admission (observe-only during gestures);
   - pinned entries and hinted chart subtrees (pinning is a prerequisite for
     moving materialization payloads into the cache).
4. Memory management and spill:
   - integrate with DataFusion memory accounting;
   - bounded async write queue;
   - Arrow IPC or Parquet spill (native only);
   - eviction by bytes and expected value.
5. Generalization:
   - extension-node fingerprint registry;
   - source version traits;
   - compatibility with DataFusion's existing `CacheManager` or a sibling
     result-cache manager;
   - possible donation to `datafusion-contrib`, renaming
     `avenger-datafusion-cache` to a neutral `datafusion-*` name at that point
     (defer crates.io publication until then to avoid a deprecation dance).

## Testing And Rollout

- An environment kill switch (`AVENGER_PHYSICAL_CACHE=0`) from the first
  integrated build.
- A census mode that runs the visual baseline suite with the cache on and off
  and reports hits, misses, admitted writes, and any byte differences. Cache
  on/off should be byte-identical for order-stable plans.
- Replaying cached partitions pins one outcome of order-nondeterministic
  operators (round-robin repartitioning, hash-aggregate emission order). This
  improves frame-to-frame stability, but it means on/off byte-identity is only
  guaranteed where plans are order-stable; where rendering depends on the row
  order of an unordered query, the census surfaces a pre-existing issue rather
  than one the cache introduces.
- Hit/miss/eviction/bytes/latency metrics from day one, surfaced per
  evaluation so chart sessions can report cache effectiveness.

## Open Questions

- How should extension `ExecutionPlan` nodes expose stable fingerprints?
- Should source version identity live on `TableProvider`, object-store layers, or
  a separate registry?
- Can `CacheReadExec` preserve enough ordering and partitioning metadata for all
  downstream operators, or should v1 drop to conservative unknown properties?
- What is the right default admission policy for interactive chart workloads?
- Beyond compiler-inserted cache boundaries, should Avenger expose user-level
  cache hints, or keep this entirely automatic?
- Should spilled cache entries use Arrow IPC for speed or Parquet for size and
  column pruning?
