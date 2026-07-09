# Logical-Plan Partial Evaluation (Server-Side Plot Baking)

## Status

Direction note. The generic evaluator crate can start at any time; the Avenger
integration has a prerequisite (compile-time lowering of transforms into
DataFusion plans, see below). The motivating use case:

- a plot is constructed and compiled on a server that has access to the data
  files;
- some of its data work is param-free (fixed filters, joins, aggregations,
  cleanup SQL) and some is param-dependent (interactive filters, view domains,
  selection joins);
- we want to execute as much as possible on the server at build time, then ship
  a chart that is fully parameterized for client-side interactivity and needs
  no server at runtime.

The mechanism is partial evaluation of DataFusion **logical plans**: execute
the maximal param-independent subtrees once, and splice the results back into
the plan as in-memory table scans. Params, stores, selections, and
view-dependent work stay symbolic. Avenger's involvement is a thin
orchestration pass over the compiled plot; all of the actual machinery is
Avenger-agnostic and lives in a new `avenger-datafusion-partial-eval` workspace
crate with DataFusion-only dependencies, sibling to `avenger-datafusion-cache`
(see "Relationship To avenger-datafusion-cache").

## Non-Goals

This does not specialize the plot to specific param values. By default the
output is exactly as parameterized as the input: only subtrees that are
independent of *all* params fold. (An explicit opt-in lets the caller declare
certain params constant to fold more; see "Optional Param Specialization".)

This is not a materialized-view matching system. Folded subtrees are replaced
exactly where they stand; nothing tries to answer a different query from a
baked result.

This is not runtime federation. Executing param-dependent work on a server
during interaction is the client/server pushdown design
([client-server-architecture.md](client-server-architecture.md)); this design
removes the runtime server requirement for the param-free portion instead.

This is not freshness management. A baked plot is an as-of snapshot by design;
nothing here watches sources or re-bakes automatically.

## Prior Art

- DataFusion's `SimplifyExpressions`/const evaluator folds constant
  *expressions* during logical optimization. This design is the plan-level
  analog: folding constant *subtrees* into materialized tables.
- DataFusion `DataFrame::cache()` materializes an entire `DataFrame` into a
  `MemTable`. This design is the subtree-granular, placeholder-aware version.
- Spark `Dataset.checkpoint()` truncates a plan's lineage by materializing it;
  same shape, different motivation.
- dbt-style build pipelines materialize some models and leave others as views;
  the fold/keep-symbolic split here is the same workflow decision at plot
  granularity.
- Materialized views precompute at the table level with semantic matching;
  this design is narrower (exact subtree substitution at build time).
- In this repository: the chart DSL's data catalogs define `table sql` views
  with opt-in materialization ([chart-dsl.md](chart-dsl.md)) — that is
  catalog-level baking; this design bakes at plot-subtree level below or above
  any catalog tables. The physical-plan cache
  ([physical-plan-evaluation-cache.md](physical-plan-evaluation-cache.md))
  is the runtime complement (see the relationship section).

## The Core Primitive

```rust
struct PartialEvalPolicy {
    max_baked_bytes_per_subtree: usize,
    max_baked_bytes_total: usize,
    /// Params the caller declares constant; their placeholders are bound
    /// before folding. Empty by default (fully parameterized output).
    fixed_params: Vec<(String, ScalarValue)>,
    /// Supplying an epoch makes epoch-stamped "now"-style expressions
    /// foldable; absent, they block folding of their subtree.
    evaluation_epoch: Option<EvaluationEpoch>,
}

struct BakeReport {
    baked: Vec<BakedSubtree>,     // fingerprint, rows, bytes, folded sources
    skipped: Vec<SkippedSubtree>, // reason: params, volatile, over budget, ...
    source_versions: Vec<(SourceId, CacheVersion)>,
    as_of: EvaluationEpoch,
}

async fn partial_evaluate(
    plan: LogicalPlan,
    ctx: &SessionContext,
    policy: &PartialEvalPolicy,
) -> Result<(LogicalPlan, BakeReport)>;
```

The rewrite operates at the **logical** level, deliberately opposite to the
physical-plan cache:

- compiled plots store logical plans (proto `LogicalPlanNode` via
  `SerializableDataFrame`), so a logical rewrite slots into the existing
  artifact format;
- the client re-plans physically for its own hardware and configuration, which
  a physically-baked artifact would fight;
- a logical splice preserves downstream optimizer freedom — projection pushdown
  into the baked table scan still works, and param-dependent operators above
  the fold plan normally.

## Foldability

A subtree is foldable when all of the following hold:

- it contains no `Expr::Placeholder` (params stay symbolic);
- it scans no store or selection tables — those are revisioned runtime sources
  and are the interactivity being preserved;
- it calls no volatile functions (`random()`-style) and no now-family
  functions (`now()`, `current_date`, `current_time`, `current_timestamp`).
  The now-family is `Stable`, not `Volatile`: DataFusion deliberately folds
  it to the query start time during simplification, which at bake time would
  freeze bake time into baked results and the residual alike. The bake
  therefore runs its optimizer **without a query start time**
  (`OptimizerConfig::query_execution_start_time` is an `Option`, and `now()`
  simplifies only when it is `Some`), so now-family calls survive
  symbolically: subtrees containing them do not fold, their predicate
  conjuncts are held high like param predicates, and the residual
  re-evaluates them at the consumer's own runtime. An explicit
  `evaluation_epoch` (future work) folds them to a chosen instant instead;
- it is bounded;
- every source it references is resident where the bake runs;
- its estimated output fits the size budget (see the next section).

Placeholder detection is a plan walk that already exists in simple form in the
facet slot code (`collect_logical_plan_placeholders` /
`collect_expr_placeholders`); the crate should own a proper public version.

One hazard from the physical cache does *not* apply here. The cache must guard
against runtime-mutated plan state (dynamic filter pushdown) because it
captures output below consumers it does not control. Partial evaluation always
executes the **complete** logical subtree as its own query root: any dynamic
filters arise and resolve inside that execution, and the captured output is the
full, correct result of the subtree. At client runtime, downstream operators
may push dynamic filters into the baked table scan; that only prunes reads of
complete data. The volatility and boundedness rules are shared with the cache;
the dynamic-filter exclusion is not needed.

## Optimization Ordering

The fold input is the analyzer- and logical-optimizer-processed plan **with
placeholders still unbound**. This is DataFusion's prepared-statement path:
`Expr::Placeholder` passes through the analyzer (types are inferred from
context — `x < $p` types `$p` from `x`'s column), and the optimizer rules
treat placeholder-bearing expressions structurally like any others.

Running the optimizer first is what makes baked tables minimal:

- `OptimizeProjections` computes column usage over the *whole* plan,
  param-dependent operators included — a filter `x < $p` demands column `x`
  whether or not `$p` is bound. By the time the fold frontier is chosen, every
  foldable subtree has already been narrowed to exactly the columns the full
  residual plan needs.
- Param-free filter and limit pushdown have likewise already reduced the rows
  entering the fold.
- After the splice, the client re-optimizes the plan per query, and `MemTable`
  scans accept projection pushdown, so a consumer that needs fewer columns
  than were baked still reads only what it needs. (Filters do not push into
  `MemTable` scans; they execute above, which is fine for budget-sized baked
  tables.)

Predicate pushdown needs the opposite treatment for placeholder-bearing
predicates. Pushing `x < $p` below param-free joins or aggregations re-parents
that work *above* the predicate, shrinking the foldable region — in the worst
case from "the aggregated table" down to "the raw scan", which the budget
rightly rejects, and the bake produces nothing. The bake pass therefore runs
its own optimizer configuration in which placeholder-bearing conjuncts are
non-pushable — exactly the treatment `PushDownFilter` already gives volatile
predicates. Conjunctions still split first, so in
`x < $p AND region = 'EU'` the param-free conjunct pushes into the fold and
reduces baked rows while the param conjunct holds.

Holding a predicate at its authored position is trivially correct (pushdown is
only ever a performance transformation), and it costs nothing at runtime
because of a two-phase asymmetry: the bake analysis is the only phase that
ever sees placeholders. Clients bind params to literal values *before*
optimizing (`with_param_values` rewrites the plan eagerly), so the runtime
optimizer pushes the bound predicates as far as they go regardless of where
the bake held them. The held-high position shapes the fold boundary and
nothing else.

The corollary is inherent to interactivity rather than a cost of this rule:
the baked table must serve *every* param value, so it is by definition the
result unreduced by `$p`. When that all-params table exceeds the budget, the
fold correctly aborts, and the subtree is a case for runtime pushdown
([client-server-architecture.md](client-server-architecture.md)) rather than
baking.

More generally, the bake pass owns its optimizer run because its objective
differs from execution: maximize the param-free region, not minimize one
query's cost. This is the same tension the physical cache resolves with
boundary hints at runtime; baking resolves it more cleanly because it
controls the optimizer configuration outright.

One practical requirement falls out: every placeholder must be type-inferable
from its context, or analysis fails. Avenger constructs typed plans, so this
holds in practice; the bake pass should surface the failure clearly if it
ever does not.

## Fold Frontier And Economics

Folding chooses maximal foldable subtrees, but "maximal" needs an economic
guard: the correctness rules alone would happily fold a raw scan, embedding an
entire source file into the artifact. The degenerate cases:

- baking *below* a param-dependent filter that would have been pushed into the
  scan ships an intermediate far larger than any client query needs;
- baking a wide table whose downstream consumers project two columns ships
  dead weight (the optimization ordering above makes this rare for single
  consumers; union-baked shared tables can still be wider than any one
  consumer needs).

The useful fold points are after selective param-free reductions (filters,
joins, aggregations) and before the first param-dependent stage. The policy
enforces this with per-subtree and total byte budgets. The v1 evaluator
enforces budgets from actual output size while collecting batches (abort a fold
that exceeds its budget; report it in `BakeReport.skipped`).
Statistics-informed pre-estimates are future work. The budget also protects the
client: every baked table is resident memory in the consuming session,
including 32-bit wasm.

Identical foldable subtrees can appear in several data contexts (shared chains
are duplicated into each consumer at compile time). The v1 evaluator
deduplicates exactly equal logical subtrees. Because each context's plan is
optimized independently, future versions should also deduplicate by
logical-subtree fingerprint **modulo the topmost projection**: bake the column
union once, and splice a per-consumer projection over the shared baked table.
That future extension is the bake-time analog of the physical cache's
projection adapter over `CacheReadExec`, and it is affordable here precisely
because baking is offline. This is also the one place logical-plan
fingerprints — a non-goal for the physical cache — earn their keep, and they
stay internal to the bake pass.

Explicit boundary hints compose naturally: a compile-time boundary marker that
pins a stable projection and blocks pushdown (introduced for the physical
cache's shared-chain problem) is also a declared "good fold point" for baking.
Both consumers should honor the same marker.

## Embedding Results

Serialization support already exists: `AvengerCoreExtensionCodec` encodes
unnamed `MemTable` providers inline as Arrow IPC stream bytes, and named
`MemTable`s as references that re-register on decode
(`avenger-chart-core/src/serialization/dataframe.rs`). That gives two shipping
modes:

- **inline** (unnamed): the baked bytes travel inside the serialized plan —
  fully self-contained, right for singleton tables;
- **manifest** (named): the plan carries a reference and the baked table ships
  once in a sidecar manifest — required for deduplicated tables spliced into
  multiple plans, otherwise the same IPC bytes embed once per scan site.

The bake pass should pick per table: inline for single-use tables under a size
threshold, manifest for shared or large ones. IPC compression is an open
question below.

Baked tables carry exact row and byte counts; recording them as table
statistics improves client-side planning over the baked scans. Each baked
table's content hash is also a ready-made source version: if the physical
cache runs in the consuming session, baked scans arrive with clean
`CacheVersionProvider` identities for free.

## As-Of Semantics

A baked plot is a snapshot. That is the point — the artifact is reproducible
and self-contained — but it must be explicit rather than silent:

- `BakeReport.source_versions` records the version of every source that was
  folded away (snapshot IDs, ETags, file mtime+size, content hashes);
- the compiled plot carries the report's `as_of` stamp and source versions so
  staleness is inspectable at any distance from the server;
- baking is non-destructive: the server keeps the original compiled plot, and
  re-baking against fresh data is "run the pass again", not an inverse
  operation. The artifact may optionally carry a provenance pointer to the
  unbaked plot's identity.

## Optional Param Specialization

By default nothing param-dependent folds, keeping the chart fully interactive.
Some deployments want more: a dashboard variant pinned to `region = 'EU'`
should fold the region filter and everything below it. `fixed_params` supports
this explicitly — the named placeholders are bound to the given values first,
then folding proceeds normally. The trade (those params stop being
interactive) is the caller's explicit decision, never inferred from initial
param values.

The report makes specialization self-describing: it echoes the fixed params
that were applied, lists fixed names that matched no placeholder (not an
error — in a multi-plan bake a param may bind in only some plans), and
inventories the placeholders remaining in the residual — the exact set of
params the baked artifact still binds at runtime.

## Prerequisite: Compile-Time Lowering Of Transforms

The evaluator can only fold what it can see, and today it cannot see the whole
pipeline. `CompiledDataContext` stores a base logical plan plus a
`Vec<DataTransformStage>` applied Avenger-side during evaluation
(`avenger-chart-core/src/compiled_data_context.rs`). Run against today's
artifacts, partial evaluation would fold only the base plan — correct but
weak — or need to understand Avenger transform semantics, which is the wrong
layer.

The fix is the direction the transform system is already pointed: user-defined
transforms become SQL over a primitive Rust core
([chart-dsl.md](chart-dsl.md); [transform-system.md](transform-system.md)
"Database Pushdown" already steers transforms toward plan-expressible
constructs for exactly this class of reason). Once the static prefix of each
data context lowers into a single logical plan with placeholders at compile
time, the generic evaluator sees the whole foldable pipeline. The primitives
sort themselves out:

- `Rasterize2D` is a UDAF — plan-expressible — and view-param-dependent, so it
  stays live regardless;
- param-free `ScalarAggregate` results can fold to literals (a v2 extension);
  param-dependent ones stay symbolic;
- store and selection scans stay symbolic by rule;
- facet cell filters and selection clauses are injected above the shared chain
  at evaluation time, so they sit on top of the baked scan naturally and one
  baked chain serves every facet cell.

The server-bake use case is therefore an independent argument for scheduling
the lowering migration ahead of the DSL surface syntax that also wants it.
[transform-lowering.md](transform-lowering.md) makes this prerequisite
concrete: it defines the lowering pipeline, the scope-to-grouping rule, and
the plan-break contract under which transforms that still execute their input
remain supported. Expansion and pre-evaluation apply to lowered class-1
pipelines and stop at plan breaks; folding across a param-free break by
executing it at bake time is a possible later extension, not v1 scope.

## The Avenger Pass

The user-facing verb is **bake** — baking partially evaluates the plot's
data pipelines with respect to its live params. "Partial evaluation"
remains the mechanism term (this document, the crate); `bake` is the API
vocabulary at the chart level. With lowering in place, the Avenger side is
thin orchestration:

```rust
impl CompiledPlot {
    async fn bake(
        &self,
        ctx: &SessionContext,
        policy: &PartialEvalPolicy,
    ) -> Result<(CompiledPlot, BakeReport)>;
}
```

It iterates the compiled data contexts, runs `partial_evaluate` on each lowered
plan (deduplicating across contexts), chooses inline versus manifest embedding
per baked table, and stamps the as-of report. Its output is chart-level, not
plan-level: a folded data context's base data becomes the baked table, and the
remainder is unparsed back into a `sql` transform stage with placeholders
intact (already scope-lowered and marked so recompilation does not re-inject
facet keys), so the baked plot is itself an ordinary, printable chart
definition (see [transform-lowering.md](transform-lowering.md)). Parse and
unparse sit at the pass boundary; the fold itself still runs on optimized
logical plans.
Policy, not mechanism: size budgets, per-context opt-outs, fixed params. Store
data, channels, scales, and everything above the data layer pass through
untouched.

## Relationship To avenger-datafusion-cache

The two designs are complements along every axis:

| | Partial evaluation | Physical-plan cache |
| --- | --- | --- |
| When | build time | run time |
| Plan layer | logical | physical |
| Effect | rewrites the artifact | transparent per-session reuse |
| Data change | freezes an explicit as-of snapshot | invalidates via source versions |
| Travels | across machines, inside the artifact | never leaves the session |
| Dynamic filters | immune (executes complete subtrees) | must exclude runtime-mutated subtrees |

They also cooperate at runtime: a baked plot's residual param-dependent queries
run over small `MemTable` scans whose content hashes give the cache clean
source versions, and subtrees folded at build time simply never reach the cache
at all.

### Should It Live In The Same Crate?

No — sibling crate, same layer. `avenger-datafusion-partial-eval` alongside
`avenger-datafusion-cache`, both with DataFusion-only dependencies, for these
reasons:

- **Different lifecycle.** The evaluator is a stateless async function; the
  cache is a stateful runtime component with memory budgets, eviction, and
  entry lifecycles. Nothing is simplified by co-locating them.
- **Different consumers.** Every interactive client wants the cache; only
  server-side authoring tooling wants the evaluator. Wasm builds should not
  carry the evaluator at all, and crate granularity expresses that better than
  feature flags.
- **Different correctness contracts.** The cache promises transparent,
  invalidation-correct reuse; the bake promises an explicit, stamped freeze.
  Keeping the crates separate keeps those stories separately auditable.
- **Separate graduation paths.** A result cache and a plan-folding pass are
  different `datafusion-contrib` conversations.

What they share is small and should be shared deliberately, not by merging:
the foldability/cacheability predicates (placeholder-free, volatile-free,
bounded, epoch handling) and eventually the boundary marker node. Start by
duplicating the predicates — they are on the order of a few hundred lines —
and extract a small shared foundation crate only when both exist and the
duplication demonstrably drifts. The boundary marker, which both crates want
to honor, is the natural first tenant of that shared home when it happens.

## Relationship To Client-Server Architecture

[client-server-architecture.md](client-server-architecture.md) describes two
authoring modes that both assume a live server during interaction. Baking adds
the third deployment mode: **server at build time only**. The server compiles,
bakes, and publishes a self-contained artifact; the client (including static
hosting, exported notebooks, or emailed reports) interacts with no backend.

The modes compose along a data-size gradient: bake the param-free reductions
into the artifact; push down param-dependent work that is too heavy for the
client when a server is available; let the physical cache absorb whatever
executes client-side. A plot over moderate data can graduate from
"federated" to "fully baked" without changing its definition — the bake pass
decides, per subtree, what travels as data versus what stays as plan.

## Implementation Phases

1. Generic crate (`avenger-datafusion-partial-eval`, DataFusion-only
   dependencies):
   - foldability predicates (public, tested independently);
   - frontier walk with per-subtree and total byte budgets;
   - execute-and-splice with actual-size abort;
   - exact dedup by logical-subtree fingerprint (modulo-projection dedup is a
     future extension);
   - `BakeReport` with folded source versions;
   - pure-DataFusion test suite (no chart machinery).
2. Lowering alignment (prerequisite, tracked with the transform-system work):
   - static transform prefixes lowered into data-context plans at compile
     time;
   - primitives audited for plan-expressibility.
3. Avenger pass:
   - `CompiledPlot::bake` orchestration;
   - inline versus manifest embedding selection;
   - as-of stamping on the compiled plot;
   - an end-to-end example: server bakes a plot over local Parquet, client
     wasm session renders and interacts with zero backend calls.
4. Extensions:
   - param-free `ScalarAggregate` folding to literals;
   - param-free scale-domain precompute into fixed domains;
   - boundary-marker honoring shared with the cache;
   - Arrow IPC compression for embedded tables.

## Open Questions

- Where does the shared boundary marker live once both crates want it — the
  cache crate, this crate, or the shared foundation crate that extraction
  would create?
- Inline versus manifest thresholds, and whether embedded IPC should be
  compressed (zstd) given wasm decode cost.
- Should the artifact carry a provenance pointer to the unbaked plot identity,
  and should re-baking be exposed as a server API or stay a build step?
- How does plot-level baking interact with catalog-level `table sql`
  materialization from the DSL data catalogs — bake below, above, or through
  catalog views?
- What DSL surface, if any, controls bake policy (per-transform opt-out, size
  budgets, fixed-param variants)?
