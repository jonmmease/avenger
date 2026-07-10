# Transform Lowering And Plan Breaks

## Status

Direction note, partially built. This is the prerequisite named by
[logical-plan-partial-evaluation.md](logical-plan-partial-evaluation.md)
("compile-time lowering of transforms") made concrete, and the compile-side
counterpart of the `transform sql` direction in [chart-dsl.md](chart-dsl.md).
It defines two supported execution shapes for data transforms, a lowering
pipeline of deterministic compiler passes, and the contract that keeps the
non-lowered shape correct everywhere it appears.

**As-built (2026-07):** implementation steps 1 and 2 landed (the `Sql`
transform, `ExecutionShape` declarations, `expand_stage` derived expansion,
and the round-trip census; `join_aggregate` was refactored to window
functions, leaving only `lump` native-only). Step 3 (the scope injector) was
built and then **retired** — see the as-built note under "Scope Lowering
Rule". Step 4 (the chart-level expansion pass) was **descoped** by owner
decision on 2026-07-09 together with the printable/sql-stage bake emit;
`expand_stage` survives as the editor action's engine.

Design stance up front: **plan breaks are a permanently supported shape.**
Transforms that must execute their input are always possible and always
correct; they are allowed to be less efficient. The system's job at a break is
graceful degradation, not break elimination.

## Two Execution Shapes

A data transform executes in one of two shapes:

- **Lowered**: a plan-to-plan rewrite. The transform contributes operators and
  expressions to the data context's single DataFusion logical plan
  (placeholders intact). Everything downstream — partial evaluation, the
  physical cache, client/server pushdown, whole-plan optimization — sees
  through it.
- **Plan break**: the transform executes its input and produces a new table
  (today: `collect()` + `read_batch` inside `apply`). The pipeline becomes a
  sequence of plan *segments* separated by breaks. Workstreams operate on
  segments; the break itself participates through the contract below.

## Where Today's Transforms Stand

From an audit of `avenger-chart-transforms` apply paths:

- **Already lowered (pure plan rewrites)**: `aggregate`, `calculate`,
  `filter`, `fold`, `impute`, `join_aggregate`, `lump`, `select`, `stack`,
  `time_levels`, `window`.
- **Plan breaks today**: `kde` (bandwidth/extent evaluated eagerly, kernel
  computed in Rust per group), `time_fill` (gap rows generated from executed
  distincts/extents), `bin` (edge computation), `time_unit` (lookup
  construction), `scalar_aggregate` (executes to seed derived scalars),
  `rasterize_2d` (materialization point, break by design).

Notably, none of today's breaks are *forced* to be breaks by their semantics:
every non-rasterize break exists to compute data-dependent constants (extents,
edges, distincts, scalars), and the rasterize break is the async
materialization pattern working as intended.

The taxonomy therefore classifies implementations, not semantics: class 2 is
class 1 in waiting. Scalar subqueries (plus the occasional UDAF) are the
bridge, crossed by refactoring a class-2 transform's `apply` to build the
subquery-bearing plan directly — one implementation, no dual paths. Classes 3
and 4 are the semantic boundaries that no subquery rewrite can cross —
data-dependent output schemas, and breaks whose asynchrony is the feature.

## Taxonomy

1. **Pure plan rewrites** — lowered by construction.
2. **Data-dependent constants** — the transform needs scalars computed from
   its input (KDE bandwidth via variance, auto bin edges, time-fill extents,
   time-unit lookups). These lower fully: the constants become scalar
   subqueries (or small joins) embedded in the same plan, and the remaining
   logic is ordinary relational work or a UDAF. Output schemas are fixed, and
   expansion needs only the input *schema*, which plans carry without
   executing. KDE, the canonical worry, is in this class: bandwidth and extent
   are scalar subqueries, the kernel is a bounded `generate_series` grid
   cross-joined with the data and aggregated (or a KDE UDAF if the SQL shape
   is awkward — still in-plan, like `Rasterize2D`).
3. **Schema-data-dependent** — the *output schema* depends on data values
   (`Pivot`: columns from distinct values). These are necessarily breaks, or
   require an expansion-time schema evaluation with explicit as-of semantics.
   They should be rare and explicit; none exist today.
4. **Materialization points** — `Rasterize2D`: plan-expressible (UDAF) but
   wrapped in async scheduling, previews, and debounce. Breaks with identity
   discipline done right; the model for what the contract below asks of
   everything else.

## Lowering Examples

Expansion is a chart-to-chart rewrite: plan-pure transform *stages* are
replaced with equivalent derived `sql` stages. Both charts in each pair below
are valid definitions producing identical output; the second is what the
expansion pass produces from the first, and being able to print it is the
debugging story. The `Kde`, `Bin`, and `ScalarAggregate` pairs assume those
transforms' class-1 refactors have landed — until then, their stages are plan
breaks and do not expand.

**Fully specified KDE.** When bandwidth, steps, and extent are literals (as in
the existing `kde_density_area` visual test), there are no data-dependent
constants at all and the generated query is static:

```rust
// before
Area::new().transform(
    Kde::new(col("value"))
        .bandwidth(0.38)
        .steps(160)
        .extent(-3.2, 3.8)
        .as_fields("sample", "density"),
    |mark, kde| mark.x_with(kde.value(), /* … */).y_with(kde.density(), /* … */),
)
```

```rust
// after expansion
Area::new().transform(
    Sql::new(r#"
        WITH grid AS (
            SELECT -3.2 + (3.8 - -3.2) * g.i / 159.0 AS sample
            FROM generate_series(0, 159) AS g(i)
        )
        SELECT grid.sample,
               sum(exp(-0.5 * pow((grid.sample - d."value") / 0.38, 2)))
                 / (count(*) * 0.38 * sqrt(2 * pi())) AS density
        FROM grid CROSS JOIN input d
        GROUP BY grid.sample
    "#),
    |mark, t| mark.x_with(t.field("sample"), /* … */).y_with(t.field("density"), /* … */),
)
```

The channel bindings survive the rewrite because the stage's output contract
(fields `sample`, `density`) is preserved.

**Grouped KDE with automatic extent** (the `kde_grouped_density_lines` test).
Data-dependent constants become subqueries and `group_by` becomes `GROUP BY`:

```rust
// before
Line::new().transform(
    Kde::new(col("value"))
        .group_by([col("series")])
        .bandwidth(0.42)
        .steps(150)
        .resolve(KdeResolve::Shared)
        .as_fields("sample", "density"),
    |mark, kde| /* … */,
)
```

```sql
-- after expansion (the Sql stage's query)
WITH ext AS (SELECT min("value") AS lo, max("value") AS hi FROM input),
grid AS (
    SELECT ext.lo + (ext.hi - ext.lo) * g.i / 149.0 AS sample
    FROM ext CROSS JOIN generate_series(0, 149) AS g(i)
),
counts AS (SELECT "series", count(*) AS n FROM input GROUP BY "series")
SELECT d."series", grid.sample,
       sum(exp(-0.5 * pow((grid.sample - d."value") / 0.42, 2)))
         / (max(c.n) * 0.42 * sqrt(2 * pi())) AS density
FROM grid CROSS JOIN input d JOIN counts c ON c."series" = d."series"
GROUP BY d."series", grid.sample
```

`KdeResolve::Shared` is visible in the SQL: one global extent feeds one shared
grid. `Independent` would compute `ext` per series and join it back. Automatic
bandwidth would add a Silverman expression over `stddev`/`approx_percentile_cont`
to a per-group stats CTE. If the numerics warrant it, the kernel sum becomes a
`kde` UDAF — still one plan. And if this mark lived in a facet with per-cell
scope, expansion appends the facet partition columns to `counts` and the final
`GROUP BY`: the facet *columns* are known at compile time even though the cell
values are not.

**Bin with automatic extent.** `Bin::new(col("value")).maxbins(7)` expands to
an extent subquery plus pure-math edge expressions (`bin_step` is a scalar UDF
on `(lo, hi, maxbins)` with no data access):

```sql
WITH ext AS (SELECT min("value") AS lo, max("value") AS hi FROM input)
SELECT input.*,
       e.lo + floor(("value" - e.lo) / bin_step(e.lo, e.hi, 7))
            * bin_step(e.lo, e.hi, 7)                     AS bin_start,
       bin_start + bin_step(e.lo, e.hi, 7)                AS bin_end
FROM input CROSS JOIN ext e
```

**ScalarAggregate consumed inside the pipeline** expands to a scalar subquery
(`WHERE "price" >= 0.5 * (SELECT max("price") FROM input)`); scalars consumed
by encodings, scales, or styling outside the data plan keep the eager seeding
path.

**Pipelines of non-breaking transforms collapse into a `sql` stage too.**
Class-1 stages already build plans, so their expansion is *derived* rather
than hand-written: run the plan construction they already implement against a
symbolic `input` scan, then unparse the result to SQL (DataFusion's
plan-to-SQL unparser). No per-transform expansion code is required, and the
lowered chart is still a chart. An interactive stacked bar with a param
threshold over aggregated totals:

```rust
// authored
Rect::new().transform(
    Aggregate::new()
        .group_by([col("month"), col("series")])
        .sum("total", col("value")),
    |mark, agg| {
        mark.transform(
            Filter::new(agg.output("total").gt_eq(min_total.expr())),
            |mark, _| {
                mark.transform(
                    Stack::new(col("total"))
                        .group_by([col("month")])
                        .sort_by_exprs([col("series")])
                        .name("total_stack"),
                    |mark, stack| {
                        mark.x(col("month"))
                            .y_with(stack.start(), /* … */)
                            .y2(stack.end())
                            .fill(col("series"))
                    },
                )
            },
        )
    },
)
```

```rust
// lowered: the run of class-1 stages is one derived Sql stage
Rect::new().transform(
    Sql::new(r#"
        WITH agg AS (
            SELECT "month", "series", sum("value") AS total
            FROM input GROUP BY "month", "series"
        ),
        kept AS (SELECT * FROM agg WHERE total >= $min_total)
        SELECT *,
               sum(total) OVER (
                   PARTITION BY "month" ORDER BY "series"
                   ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW
               ) AS total_stack_end,
               total_stack_end - total AS total_stack_start
        FROM kept
    "#),
    |mark, t| /* same channel bindings via t.field(...) */,
)
```

Pre-evaluation is then chart-to-chart as well: parse the stage's query,
optimize under the bake configuration (placeholder predicates held high), fold
the frontier — here, everything below the `$min_total` filter — execute the
folded portion, and emit a chart whose *data* is the baked table and whose
stage is the residual query:

```rust
// baked: data replaced, residual sql keeps the params
plot.data(baked_month_series_totals)   // MemTable in the artifact: agg's result
    .mark(Rect::new().transform(
        Sql::new(r#"
            WITH kept AS (SELECT * FROM input WHERE total >= $min_total)
            SELECT *,
                   sum(total) OVER (
                       PARTITION BY "month" ORDER BY "series"
                       ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW
                   ) AS total_stack_end,
                   total_stack_end - total AS total_stack_start
            FROM kept
        "#),
        |mark, t| /* unchanged bindings */,
    ))
```

Every `$min_total` change re-runs only the filter and the window restack over
the baked aggregate. Notes on the mechanics:

- the residual is the *optimized remainder unparsed back to SQL*, not a
  textual slice of the authored query;
- contiguous class-1 runs collapse into one stage; collapse boundaries are
  plan breaks and sharing-scope changes (until scope has been lowered to
  grouping inside the generated query);
- baked side-inputs beyond the pipeline's own `input` (join branches, for
  example) become named tables in the artifact manifest, referenced by name
  from the residual query.

Equivalence is directly testable: a derived expansion must round-trip — the
unparsed query, reparsed against the same schema, must plan equivalently to
what the stage builds directly — and running both forms of a chart must
produce identical output. Unparser fidelity is thus a hard, tested
requirement; a stage whose plan cannot be unparsed faithfully stays native —
an accepted exception in the same spirit as plan breaks. For class-2
refactors, the oracle is transitional: the visual baseline suite plus
targeted old-versus-new output comparisons during the rewrite.

## The `sql` Transform Primitive

The DSL already commits to `transform sql` as the primitive escape hatch: one
query statement over the reserved relation `input`, query-only
(`SELECT`/`VALUES`), pipelined positionally
([chart-dsl.md](chart-dsl.md)). The lowering work adds its Rust-API twin — a
`Sql` transform stage — because the two are the same object at two surfaces,
and it earns its place three ways:

- **Lowered by construction.** Its `lower()` parses the query and rewrites
  the `input` table reference to the upstream subplan; there is no `apply`
  path that executes anything. A `sql` transform can never be a plan break,
  which makes it the escape hatch users should reach *before* writing a Rust
  transform.
- **The expansion target.** Derived expansions emit `Sql` stages, and
  user-defined transforms (`define transform` pipelines over `transform sql`)
  are born as them, so lowered charts speak one language regardless of
  provenance.
- **The single assembly seam.** Plan assembly needs exactly one lowering
  mechanism: parse each `sql` stage and splice the upstream plan in for
  `input`. Expansions target `Sql` stages, so assembly never needs
  per-transform knowledge.

Placeholders (`$param`) in the query text parse to `Expr::Placeholder` and
flow through the standard param machinery, so `sql` stages are exactly as
bake-able, cache-able, and pushdown-able as their content allows.

## The Lowering Pipeline

Compiler passes over the compiled representation, in order:

1. **Expansion**: plan-pure transform *stages* rewrite to `sql` stages at
   the chart representation level — a chart-to-chart rewrite whose output is
   itself a printable chart definition. Expansion is derived-only: build the
   plan the stage already builds, unparse it to SQL, with contiguous class-1
   runs collapsing into one stage. Class-2 stages participate by being
   refactored to class 1 (their data-dependent constants becoming scalar
   subqueries in the refactored plan construction); until then they are plan
   breaks and pass through unexpanded.
2. **Scope lowering**: transform sharing scope becomes grouping (see next
   section).
3. **Plan assembly**: each data context's base plan plus its lowered stages
   compose into one logical plan with placeholders intact — the input the
   partial evaluator and the bake pass require.
4. **Policy passes**: boundary markers, bake budgets, and other annotations
   consume the assembled plans.

This is deliberately a *lowering* pipeline, not an optimizer. Every pass is a
deterministic semantic rewrite. Cost-based search stays in DataFusion;
Avenger-side passes that need cost awareness (the bake's fold frontier) are
explicit policy with budgets, not plan-equivalence exploration.

## Scope Lowering Rule

**Scope lowers to grouping columns, never to plan multiplicity.** Per-cell
scope means the expanded plan groups or partitions by the facet keys up to the
owning depth; owner scope means no grouping; per-cell filters apply above the
shared result, as they do today.

The KDE implementation already validates this rule Rust-side: it appends the
facet partition columns to its `group_by` and computes every cell's density in
one grouped pass. The lowering pass generalizes that from "each transform
handles `facet_context` itself" to a single shared rewrite — and lifts KDE's
v1 restriction (partition expressions must be simple columns) by projecting
computed partition expressions into named columns before grouping.

The rule is what preserves the downstream story. One grouped plan serves every
facet cell, so a param-free per-cell density over param-free input folds into
a single baked table with facet-key columns; the same plan is one cache
subtree and one pushdown fragment. Per-cell plan multiplicity would also be a
chicken-and-egg error: cells are data-dependent and unknown at compile time.
Plan breaks must follow the same rule internally — one grouped execution, not
one execution per cell.

> **As-built (2026-07-09, the owner-scope domain law):** the injector was
> implemented and then removed, because the rule's premise fails at the
> CHART level today: unbaked shared-scale domain inference evaluates a mark
> group's transform chain at the *sharing-owner* scope (a per-cell `SUM`
> contributes the GLOBAL total to a shared domain), and a table pre-grouped
> to leaf cells cannot reproduce owner-scope chain output. No correct
> one-table lowering exists while domain inference has this shape. Bake v1
> instead serves faceted charts through plot-data baking: the plot's data
> plan folds into ONE table of raw rows (facet-key columns included), and
> per-cell chains stay live above it — which satisfies this section's
> "per-cell filters apply above the shared result" clause with the shared
> result being the un-aggregated base. Scope lowering as written remains the
> right target if and when domain inference is redesigned to consume grouped
> chain output; the retired injector is recoverable from branch history
> (`jonmmease/facet-fresh-start`, pre-`b34fa9477`).

### Scope And The `sql` Stage

The authored query string cannot capture facet scope, for two reasons: a
reusable definition (`define transform`) does not know the facet columns of
the charts that will use it, and per-cell versus shared is a semantic choice
invisible in the text — `SELECT avg("v") FROM input` is a global mean or a
per-cell mean depending on intent, not syntax.

So the `sql` stage carries the same stage-level scope every other transform
stage already carries (`CoordinationScope` on the stage — not a Sql-specific
option), and the scope-lowering pass makes the *lowered* query capture it:
parse the query, track `input` lineage, and inject the facet partition columns
into every aggregate's `GROUP BY`, every window's `PARTITION BY`, `DISTINCT`
column sets, key-equality conditions on input-lineage self-joins, and
per-group `LIMIT`/`ORDER` via `row_number()`. Authored SQL declares intent;
lowered SQL embodies it. This also centralizes what is currently
per-transform Rust: today each facet-aware transform consumes
`facet_context` itself (`kde`, `rasterize_2d`); after lowering, the injection
is one pass applied uniformly to every `sql` stage.

Two boundary rules keep this sound:

- Scope is injected exactly once, by this pass, at the plan level. Derived
  expansion emits the portable form (the transform's own `group_by`, no facet
  keys), and every stage carries its scope declaration. (An earlier revision
  also defined an "already-scoped" marking for baked-artifact residual `sql`
  stages; that emit form was descoped 2026-07-09 — baked artifacts are
  proto-only — so no already-scoped queries exist in any document.)
- A per-cell `sql` stage whose query the rewriter cannot handle (v1 supports
  a declared subset; input-lineage self-joins are the canonical hard case)
  falls back to per-cell eager execution — a plan break under the contract:
  always possible, less efficient, never wrong.

Row-preserving queries are scope-invariant and need no injection.

This whole subsection inherits the as-built caveat above: with the injector
retired, per-cell `sql` stages evaluate per cell like every other stage
(correct, unshared), and this design reactivates only alongside a
domain-inference redesign.

## The Plan Break Contract

"Possible but less efficient" holds only if breaks carry these obligations:

- **Determinism declaration and version.** A break declares whether its
  output is a pure function of its input plus configuration, and carries a
  version (the `Rasterize2DMaterializationSpec { version, .. }` pattern).
  Volatile breaks are excluded from folding and memoization.
- **Stable output identity.** The break derives its output table's identity
  from `fingerprint(input segment plan, transform config, params)` plus its
  version — generalizing `view_materialization_identity` and the rasterize
  key. This is the single most important obligation. Today's anonymous
  `read_batch` output churns identity every evaluation, which blinds the
  physical cache to *everything downstream of the break* and defeats
  hot-reload reuse. With stable identity, the break memoizes exactly like a
  materialization, and downstream segments cache and survive recompiles
  normally.
- **Declared dependencies.** A break declares which params, stores, and
  placeholders it depends on. These feed its output identity, and they are
  what a future across-break bake extension would use to classify a break as
  param-free.
- **Scope via grouping.** Per the rule above.
- **Optional: required input columns.** Without a declaration, the break
  consumes its input at full width and blocks projection pruning across
  itself; declaring requirements recovers most of that. Acceptable to defer.

What each workstream does at a break:

- **Partial evaluation**: the fold operates within lowered class-1 plans and
  stops at breaks; segments upstream of a break fold normally. Folding
  *across* a param-free break by executing it at bake time is a natural
  later extension (baking is eager evaluation anyway), but it is out of v1
  scope, which is class-1-only by design.
- **Physical cache**: upstream segments cache normally; the break memoizes by
  its identity (the existing materialization machinery); downstream segments
  cache because the break's output identity is stable.
- **Client/server pushdown**: segments push down; the break executes where
  its Rust runs — client-side over fetched upstream fragments, or inside the
  baked prefix. Not pushable; accepted.
- **Hot reload**: identity-keyed break outputs survive recompiles once the
  cache homes survive sessions.

What is genuinely lost at a break, and accepted: whole-plan optimization
across it (no projection/filter pushdown through the break beyond the optional
declaration), pushdown-ability of the break itself, and streaming (a break
materializes its input fully).

## Expansion Model

(As-built note: the derived expansion mechanism below exists —
`expand_stage` in `avenger-chart-transforms` — and passed its round-trip
census. Its chart-level consumer, the expansion PASS of implementation step
4, was descoped 2026-07-09; the surviving consumers are tests and the future
editor action.)

Expansion and pre-evaluation apply to class-1 (plan-pure) stages only, by
design. There are no hand-written expansions: the single expansion mechanism
is the derived one — run the plan construction the stage already implements,
unparse it — so every transform has exactly one implementation at all times.

A class-2 transform joins the model by being *refactored into class 1*: its
`apply` is rewritten to build the plan directly, with data-dependent
constants as scalar subqueries and heavy math as UDAFs where numerics warrant
it. The refactor is validated the way any behavior-preserving rewrite here
is: the visual baseline suite, plus targeted old-versus-new output
comparisons during development. Until (and unless) a class-2 transform is
refactored, it is simply a plan break under the contract — supported,
correct, unexpanded, and opaque to the bake.

Refactor priority by value:

1. `kde` — the blocker for baking/caching density charts;
2. `bin` edges and `time_fill` — common in static pipelines;
3. `time_unit` lookups — likely joins or pure expressions;
4. `scalar_aggregate` — dual-natured: scalars consumed *inside* the data
   pipeline become embedded scalar subqueries; scalars consumed by encodings,
   scales, or styling outside the data plan keep the eager seeding path.

`rasterize_2d` intentionally stays a break (class 4). `Pivot`, when it
arrives, is the first honest class-3 break.

What is settled about the mechanism: expansion produces chart stages, not
plans, so the expanded chart is inspectable and the assembly seam stays
singular (`sql` stage in, plan out).

## Editor Expansion (The LSP Action)

The DSL language server should offer "expand to `transform sql`" as a code
action on any expandable stage, so a user or an agent can see and edit the
definition a `stack` or `kde` stands for. This is the same expansion the
compiler pass runs, surfaced in the editor — one mechanism, two consumers —
and it settles how expansions must be exposed in Rust:

- **Expansion is a spec-level entry point, not a compile-pass internal**:
  `expand(spec, input_schema, function_registry) -> Result<SqlStageSpec, NotExpandable>`.
  It consumes a schema and a function registry, never table data, so the
  editor can run it at any document position where the upstream schema is
  resolvable (catalog manifests provide base schemas; planning the upstream
  stages symbolically provides the rest). The DSL toolchain reaches it
  through the same native bindings the compiler uses, and the result prints
  through the DSL's canonical printer.
- **Execution shape becomes a declaration, not an inference.** The derived
  class-1 expansion works by running the stage's existing plan construction
  against a symbolic `input` scan — safe only if `apply` is plan-pure. That
  property is currently implicit in each `apply` body. It must become a
  declared property of the transform (plan-rewrite versus break), both so
  the derived expansion can never accidentally execute a break's `apply`
  inside an editor action, and to satisfy the plan-break contract's
  declared-dependencies obligation without inspecting Rust.
- **Document expansion emits the portable form.** The generated query keeps
  the transform's own semantics (its `group_by`, sort keys, offsets) but not
  the facet keys: stage-level scope is preserved as a declaration. (The
  scope-injection and baked-artifact clauses that used to complete this
  bullet are inoperative as-built: the injector is retired and baked
  artifacts are proto-only.)
- **Readability is a requirement internal lowering does not have.** All
  expansions are derived and pass through the unparser, whose output needs a
  formatting and naming pass before it is worth editing — doubly so for
  refactored class-2 transforms like `kde`, whose generated queries are the
  ones users will most want to read. The round-trip check doubles as the
  verification affordance: the action can verify that the expanded (or
  subsequently edited) SQL produces output identical to the original stage
  on request.

Nothing else about transform definitions changes: `apply` bodies, typetag
serialization, and the materialization hooks stay as they are.

## Implementation Steps

The lowering work decomposes into four stages. The first three are
independent of pre-evaluation, the cache, and client/server work, and are
worth landing now: they carry standalone value, and stage 2 retires the
design's biggest unknown (unparser fidelity) before anything is built on top
of it.

1. **The `Sql` transform primitive.** LANDED (`fb6f66313`). Parse-and-splice
   `apply` (one query over the reserved `input` relation, `SELECT`/`VALUES`
   only), handle-based channel binding, placeholder flow, typetag
   serialization. No dependencies; immediate user value as the
   custom-transform escape hatch, and the DSL's `transform sql` lands on it
   later.
2. **Execution-shape declaration, derived expansion, round-trip census.**
   LANDED (`fb6f66313`; `join_aggregate` window rewrite `9cff96a16` leaves
   only `lump` native-only). Declare plan-pure versus break on every
   transform; implement `expand(spec, input_schema, registry)` as
   symbolic-scan + `apply` + unparse; then run the census: for every class-1
   stage in the visual test corpus, the expanded chart must render
   byte-identical to the original. This is the unparser-fidelity gate — gaps
   get fixed upstream or the affected stage is marked native-only — and it
   is the cheapest possible de-risking of the entire direction. It also
   yields the editor action's engine, minus LSP plumbing.
3. **Scope lowering.** BUILT AND RETIRED (2026-07-09) — blocked by the
   owner-scope domain law; see the as-built note under "Scope Lowering
   Rule". Reactivates only with a domain-inference redesign.
4. **The expansion pass.** DESCOPED (owner decision 2026-07-09, together
   with the printable bake emit). The derived expansion engine remains for
   the editor action; no chart-level collapse pass is planned.

In parallel, at any time: class-2 refactors (`kde` first), validated by the
baseline suite; their payoff amplifies once stage 4 exists. Dependent and
deferred: `avenger-datafusion-partial-eval` consumes stages 1–4 (its own
phases live in
[logical-plan-partial-evaluation.md](logical-plan-partial-evaluation.md));
the physical cache and session-succession tracks are orthogonal and
independently sequenced.

## Relationship To Other Documents

- [logical-plan-partial-evaluation.md](logical-plan-partial-evaluation.md):
  this doc is its lowering prerequisite made concrete; the fold operates
  within lowered class-1 plans and stops at plan breaks (folding across a
  param-free break by executing it at bake time is a noted later extension).
- [physical-plan-evaluation-cache.md](physical-plan-evaluation-cache.md):
  segments are cache territory; break output identities are ready-made source
  versions for downstream subtrees.
- [client-server-architecture.md](client-server-architecture.md): segments
  are the pushdown unit; breaks pin execution to wherever their Rust runs.
- [chart-dsl.md](chart-dsl.md) / [transform-system.md](transform-system.md):
  expansion is the compile-side of `transform sql` + `define transform`, and
  the "Database Pushdown" guidance in the transform-system note (prefer
  plan-expressible constructs, isolate unavoidable UDFs) is this document's
  direction stated from the other side.

## Open Questions

- Does the compile pipeline always collapse class-1 runs into `sql` stages,
  or only when producing artifacts (bake output, the editor action), keeping
  native stages for direct local evaluation?
- Identity plumbing: generalize `MaterializationIdentity` for break outputs,
  or introduce a break-specific identity shared with the partial evaluator's
  logical fingerprints?
- Should v1 require the input-columns declaration from breaks, or accept
  full-width inputs until it matters?
- Do `bin`/`time_unit` lookups lower as joins against generated tables, or
  stay as small identity-carrying breaks?
- How does the derived-scalar split for `scalar_aggregate` (subquery inside
  the pipeline, eager for encoding/scale consumers) interact with the
  existing derived-scalar placeholder machinery?
- What query subset does the per-cell scope rewriter support in v1, and is
  the per-cell plan-break fallback automatic or an explicit opt-in when a
  query falls outside it?
- How much formatting/naming work does unparser output need before derived
  expansions are pleasant to hand-edit in the editor action?
