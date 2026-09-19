# avenger-selection

Immutable selection state and consumer-specific DataFusion predicates. Several
views can contribute points or intervals to one named selection. Cross-filter
uses exclude contributions from the consuming view's own instance. Reusable query
families bind these predicates into native plans or prepared dataflows.

## Generate a cross-filter predicate

```rust
use std::ops::Bound;
use avenger_selection::*;
use datafusion::logical_expr::{col, Expr};

let selection = SelectionId::new("filters")?;
let delay_view = ViewAddress::root(ViewId::new("delay_histogram")?);
let delay_projection = ProjectionId::new("delay")?;
let brush = ProducerDefinition::new(
    ProducerAddress {
        selection: selection.clone(),
        producer: ProducerId::new("delay_brush")?,
        origin: delay_view.clone(),
    },
    SelectionKind::Interval,
    vec![Projection::new(delay_projection.clone(), col("delay"))?],
)?;

let before = SelectionSet::new([SelectionSnapshot::new(
    SelectionDefinition::new(selection.clone(), Resolution::Intersect),
)?])?;
let after = before.apply(&selection, SelectionUpdate::set(
    &brush,
    SelectionValue::Tuples(vec![SelectionTuple {
        terms: vec![SelectionTerm {
            projection: delay_projection,
            test: ValueTest::Range {
                lower: Bound::Included(10_i64.into()),
                upper: Bound::Excluded(30_i64.into()),
            },
        }],
    }]),
))?;

let compiler = SelectionCompiler::new();
let consumer = SelectionConsumer::new(
    ViewAddress::root(ViewId::new("distance_histogram")?),
);
let filter = compiler.filter(
    &consumer,
    SelectionFilter::cross_filter([&selection]),
)?;
let predicate: Expr = filter.predicate(&after)?;
// Bind predicate to a dataflow Boolean Expr input, or filter a DataFusion plan.
assert_eq!(before.get(&selection)?.contributions().count(), 0);
assert_eq!(after.get(&selection)?.contributions().count(), 1);
# Ok::<(), avenger_selection::Error>(())
```

Keep the `ConsumerFilter` and bind new immutable `SelectionSet` values after
updates. `filter.resolve(&state)` exposes the named-use tree, retained
contributions, effective projection mappings, excluded producers, and empty
handling. `resolved.predicate()` uses the same lowering as `filter.predicate()`.

The runnable example builds two histograms and categorical counts in a dataflow,
applies three producers under one name, changes the delay brush, and verifies
reuse of unchanged results:

```sh
cargo run -p avenger-selection --example direct_crossfilter
```

## Selection semantics

- `SelectionId` identifies a shared name. Each declaration has a separate
  `ProducerId`, and each contribution retains its full `ViewAddress`.
- `Resolution::Intersect` ANDs active producer predicates. `Union` ORs them.
  A global `set` replaces all contributions. Global point toggles can retain
  tuples from multiple origins and compare projected meanings across producers.
- Terms inside a tuple combine with AND. Tuples inside one producer combine
  with OR, preserving correlations across dimensions. Interaction kind does
  not determine comparison: a point may select bin bounds, and an interval may
  select a categorical set.
- `clear(address)` removes only that producer in every mode. `clear_all()`
  removes all contributions. An absent contribution is inactive. An explicitly
  empty `set` remains active and matches no rows. Toggling the last tuple off
  removes that contribution.
- Membership retains all producers. Cross-filtering excludes every producer
  with the consuming view's exact address. Layers can share that address.
  Sibling and nested facet instances have different addresses. An origin does
  not implicitly add facet-key comparisons to the predicate.
- Each use specifies behavior for an inactive selection. If active producers
  exist but all are excluded, that leaf is unrestricted even with
  `EmptySelection::MatchNone`. `All`, `Any`, and `Not` compose named uses without
  silently changing those rules. Missing names are errors in every branch.

Every update validates before returning a new snapshot. `apply_all` applies
ordered updates atomically. Tuple order, duplicate tuples, and set order are
canonicalized so equivalent updates can produce equal dataflow bindings.
Changing a producer definition requires a replacement `set` before toggling.

## Values and consumer mappings

Values preserve Arrow scalar types, including integer widths, decimal precision
and scale, and timestamp units and zones. Comparisons use DataFusion 54.1 type
semantics at the consuming plan, without JavaScript value coercion.

Equality and sets are null-safe. Floating point selection treats all NaN
payloads as one selectable value and both signs of zero as equal. Ranges reject
null, NaN, reversed, or differently typed bounds. Use `Bound::Unbounded`
for an open end. Exact ranges exclude null and non-finite rows, even with both ends
unbounded. Included/excluded endpoints remain explicit. Final membership is a
non-null Boolean, including under `Not`.

Selection values support flat Boolean, numeric, string, binary, date, time,
timestamp, and duration scalars. Nested/list values are outside this model.
Facet keys follow dataflow's supported partition types and retain composite key
order and the complete ordered parent path. Create them with `FacetKey::new`.

`SelectionConsumer::new(view)` uses producer projections directly for a shared
row relation. Use `with_projection(address, projection_id, expr)` for a
compiler-verified mapping to renamed or transformed fields. Mapping keys include
the producer address so local projection names cannot collide. The compiler
owns the proof that relations and projections have the same meaning.

For row-ID selection, create `RowIdentity::new(data_type)`, declare a producer
with `ProducerDefinition::row_ids`, and supply `RowIdSelection::new`. The
consumer must supply `with_row_identity` using that same identity descriptor.
Clones retain the lineage token. A fresh descriptor has a different lineage,
even with the same type. Filtering can preserve identity. Joins, aggregates,
and regenerated IDs require explicit lineage reasoning by the compiler.
Row-ID updates use `set` and `clear`. Tuple toggles apply to projected points.

Projection and mapping expressions must be immutable and row-local. Aggregates,
windows, subqueries, placeholders, and stable/volatile functions are rejected.
Columns and comparison types are validated at the consuming DataFusion plan or
dataflow Expr-input usage site, so definitions need no separate row schema.

## Pixel interval membership

Capture a solved scale and logical pixel grid with `PixelGrid`. Linear and UTC
time mappings are supported. The grid uses the existing scale UDF's input
coercion and Float32 output, then computes
`floor((Float64(coordinate) - origin) / size)` as a checked Int64 cell.

```rust
use std::{collections::HashMap, sync::Arc};
use avenger_selection::{PixelGrid, IntervalPrecision};
use avenger_scales_datafusion::BuiltinScale;
use datafusion::{arrow::array::Float32Array, logical_expr::col};

let grid = PixelGrid::new(
    BuiltinScale::Linear,
    Arc::new(Float32Array::from(vec![0.0, 200.0])),
    Arc::new(Float32Array::from(vec![0.0, 600.0])),
    HashMap::new(),
    0.0, // Grid origin in chart-local logical pixels.
    2.0, // Logical pixels per cell.
)?;
assert_eq!(grid.cell(&10.0_f64.into())?, Some(15));
let cell_expression = grid.cell_expr(col("delay"));
# let _ = (cell_expression, IntervalPrecision::Pixels { size: 2.0 });
# Ok::<(), avenger_selection::Error>(())
```

Call `brush.with_pixel_grids([(projection_id, grid)])` to create an interval
producer with `IntervalPrecision::Pixels { size }`. Grids must identify declared
projections, use a common positive size, and have finite nondegenerate domains
and ranges in the scale kernel's precision. Every range term needs a grid.
Unmapped dimensions can use categorical equality or sets. Point producers keep
exact membership.

Pixel precision changes selected membership consistently. It is not an
optimization hint. Included/excluded bounds apply to entire cells. On the grid
above, `[10, 30)` selects cells `[15, 45)`. An included upper endpoint includes
cell 45, including other values that map to it. An excluded endpoint removes its
entire boundary cell. Two distinct endpoints can map to one cell and yield an
empty interval. Decreasing scales swap endpoint positions and flags. Unbounded
ends preserve their data-space direction.

`Contribution::value()` retains the original typed bounds.
`Contribution::effective_value()` exposes checked cell bounds, and resolved
projections expose both `raw_expr()` and the cell-mapped `expr()`. The predicate
resolver uses the mapped expressions and effective bounds without evaluating
brush endpoints again.

A null, non-finite mapped coordinate, or unrepresentable cell becomes a null
Int64 cell. Active intervals exclude null cells. The cell expression itself
preserves those rows for later grouping and selection clear. Non-finite or
unrepresentable brush bounds are errors. Linear clamping follows the scale
kernel, including mapping out-of-domain values into endpoint cells. Without
clamping, offscreen values keep their out-of-range cells.

Supported configuration:

- Linear: numeric domain/range, optional Boolean `clamp`, finite numeric
  `range_offset`, and `round: false`.
- Time: Date32, Date64, or zone-free/UTC timestamp domain, numeric range, and
  optional `timezone: "UTC"` (case insensitive). Row and bound types must match
  the domain. The existing kernel reduces temporal coordinates to milliseconds.
  Inputs that would overflow its time arithmetic become null cells.
- Domain normalization, output rounding, custom/nonlinear scales, alternate
  precision modes, and other options are rejected. Supply an already resolved
  domain. Time does not support clamp or range offset in the current kernel.

Changing a scale, range, origin, or cell size creates a new grid. Build a new
producer with `with_pixel_grids` and apply `SelectionUpdate::set` with the old
contribution's retained raw values. Old snapshots preserve the old membership.
Consumer mappings are applied before pixel mapping. Device pixel ratio is not
an input: all coordinates are chart-local logical pixels.

The three-view example supports pixel precision for both brushes:

```sh
cargo run -p avenger-selection --example direct_crossfilter -- --pixels
```

## Query families and aggregate states

Construct a query once with a callback over its selection-filtered relation:

```rust
use avenger_selection::{ConsumerFilter, ProducerDefinition, QueryPolicy, SelectionSet};
use datafusion::{
    functions_aggregate::expr_fn::count,
    logical_expr::{col, lit, LogicalPlan, LogicalPlanBuilder},
};
# fn example(filter: &ConsumerFilter, source: LogicalPlan, selections: &SelectionSet,
#     delay: &ProducerDefinition) -> avenger_selection::Result<()> {
let query = filter.query(source, |rows| {
    LogicalPlanBuilder::from(rows)
        .aggregate(vec![col("carrier")], vec![count(lit(1_i64)).alias("count")])?
        .build()
})?;
let family = query.plan(selections).focus(delay).build()?;
let automatic = family.bind(selections)?;
let forced = family.bind_with_policy(selections, QueryPolicy::ForceDirect)?;
println!("{}", automatic.explain());
# let _ = forced;
# Ok(())
# }
```

The callback runs once. Its returned plan must consume the supplied relation.
The query owns a filter-site identity, so binding does not replace unrelated
filters or depend on a string name. Repeated uses and scalar-subquery uses of the
relation are supported. The recipe retains no callback or current snapshot.
Planning and binding read no data and invoke no projection functions.

`query.logical_plan(selections)` returns the ordinary native plan with the full
consumer predicate. `family.bind(selections)` returns a `BoundQuery` and strategy
diagnostics. Supported aggregates return `BoundQuery::Preaggregated`:

- `materialization` applies fixed selections and groups by the target's display
  keys plus the focused producer's interaction keys. Each aggregate uses one typed
  state column from `avenger-datafusion-aggregate-state`.
- `aggregate` is an `AggregateStep`. Its `over(materialized_relation)` method
  filters those cells with the current focused selection, merges their states,
  restores the original result schema, and applies the query's original suffix.
  `schema()` describes the required materialization fields. Relation qualifiers
  may differ. Field names, order, types, nullability, and metadata must match.

Execute the materialization once and supply a scan of its result to `over`, or
use the dataflow installation below to manage both stages. Passing the
materialization plan itself to `over` produces one nested query without a reuse
boundary. An empty ungrouped count returns zero. Grouped queries retain
only observed groups, including groups with a zero `COUNT(column)` value.

The rewrite supports the pinned DataFusion built-ins and their aliases:

| Aggregate | Materialization | Final aggregate |
|---|---|---|
| `COUNT(*)`, `COUNT(column)` | `countState` | `countMerge` |
| `SUM` | `sumState` | `sumMerge` |
| `MIN`, `MAX` | `minState`, `maxState` | `minMerge`, `maxMerge` |
| `AVG` | `avgState` | `avgMerge` |
| Sample/population variance | `varSampState`, `varPopState` | `varSampMerge`, `varPopMerge` |
| Sample/population stddev | `stddevSampState`, `stddevPopState` | `stddevSampMerge`, `stddevPopMerge` |

Several measures can share one query. Arguments can include checked computed
expressions, with native coercion and return types. Numeric families cover supported integers,
floats, and decimals. Extrema also support flat ordered types such as strings
and dates. Unsupported signatures use the direct path. The matcher checks
built-in implementations, so a custom UDAF with the same name remains direct.

The planner rewrites the nearest aggregate above the selection site. Fixed row
filters and aliases may precede that aggregate. Projection, `HAVING`, aliases,
sorting, top-k/offset, windows, and further aggregates can follow it in their
original order. The target's complete schema is restored before those finishing
operators run. Source transformations can remain upstream of the selection site.

Each target aggregate can have its own `FILTER`, applied only during state
construction. Groups remain present when all measure filters reject their rows.
The filter does not need its columns retained as interaction keys. Its argument
still requires an independent safety check because native grouped execution can
evaluate arguments before applying aggregate filters.

Each state is an opaque Arrow struct, including count. Average and moment states
retain their weights and other components inside that column. Selection uses
`Merge` to produce final values and adds no separate finalizer. Obtain the
intermediate schema from the materialization plan or `AggregateStep::schema()`.
Preserve its nested field metadata when supplying batches or a relation to
`over`. The final query retains its original field names, order, types,
nullability, and metadata. Programmatic expressions carry their function
implementations, so neither native nor installed execution needs caller UDF
registration.

The consumer predicate must factor into fixed and changing conjunctions after
self-exclusion. An intersecting shared selection supports this split. Other
producers remain fixed dependencies, including producers under the same name.
The first rewrite conservatively leaves focused union/global resolution and
focused OR/NOT expressions direct. Arbitrary Boolean branches that do not use
the focus remain fixed.

Interaction keys retain all observed values, including nulls and values outside
the current brush. Pixel producers retain cells from the same captured grid used
by direct predicates. Multiple keys preserve tuple correlation. Without a pixel
grid, exact values are retained. High-cardinality keys may provide little size
reduction. There is no cardinality-based cost decision yet.

Grouping expressions and measure arguments are checked because materialization
also evaluates unselected rows. Supported forms include columns, literals,
aliases, checked casts, floating arithmetic, independently safe `TRY_CAST` and
`CASE` children, captured pixel cells, and division by a positive literal. Unknown
UDFs, unproved integral/decimal overflow risks, and fallible children use direct
execution even when immutable. Selection supplies trusted properties for its
pixel and numeric-classification UDFs.

Target DISTINCT/ORDER BY, explicit null-treatment modifiers, unsupported
aggregates/query shapes, unsafe moved expressions, and stable/volatile
computations use the direct path with a specific reason. Native-valid outer
aggregates do not require the target's State/Merge recipe.
If one measure is unsupported, the whole target uses its direct query. Other
targets can still optimize. There is no retry that hides backend execution
failures. Invalid state and mappings remain errors.

`QueryPolicy::Auto` is the default. `ForceDirect` reports
`DirectReason::Forced` and applies the original complete predicate. Use
`.policy(QueryPolicy::ForceDirect)` on the family builder to set that default,
or `bind_with_policy` for one request. Policy overrides work in either direction
within the same family. Neither option disables ordinary dataflow caching or
changes exact/pixel membership. A focus is optional and can name an inactive
producer. A changed producer definition or grid selects direct execution until
a compatible family is prepared. Changing bounds or fixed-selection values
does not require a new family.

### Numeric behavior

The generic [pre-aggregation planner](../avenger-datafusion-preaggregate) owns
query recognition, predicate rewriting, and State/Merge plan construction. The
aggregate utility owns accumulation, state types, and merging through native
DataFusion accumulators. Count returns zero for empty global input. Other
aggregates return null for empty/all-null input. Observed groups remain present
even when every measure value is null. For finite singleton input, population
variance/stddev is zero and the sample versions are null.

Regrouping can change floating rounding, overflow, and non-finite-value results.
Automatic execution does not promise bitwise equality or identical overflow
behavior with the original physical query shape. Tests require exact schemas,
groups, ordinary null behavior, and exact-type results without overflow. Finite
floating measures use absolute/relative tolerances. Floating differences can
also affect HAVING, ranks, and top-k membership near a numeric boundary.
Known special-value cases
are tested separately.

DataFusion 54.1.0's grouped floating extrema start from finite bounds. For
example, materializing an all-positive-infinity minimum cell can retain the
largest finite value, while a direct global minimum can return infinity.
Grouped and global population statistics can also differ for non-finite
singletons. See the utility's [numeric contract](../avenger-datafusion-aggregate-state/README.md#numerical-behavior)
for details. `ForceDirect` preserves the original query shape when those
differences matter to the caller.

Materialization evaluates all retained cells, including currently unselected
ones. Aggregate failures propagate as query errors without an automatic retry.
The conservative grouping and argument checks still protect row-expression
evaluation. This is separate from the native aggregate's numeric behavior.

### Install once and bind through dataflow

Enable the optional `dataflow` feature for `family.install`. A family can use a
root source directly or a permitted import in a prepared extension:

```rust
# #[cfg(feature = "dataflow")]
# async fn example(base: &avenger_datafusion_dataflow::PreparedDataflow,
#     base_inputs: &avenger_datafusion_dataflow::Inputs,
#     source_output: &avenger_datafusion_dataflow::TableOutput,
#     filter: &avenger_selection::ConsumerFilter,
#     selections: &avenger_selection::SelectionSet,
#     delay: &avenger_selection::ProducerDefinition,
# ) -> Result<(), Box<dyn std::error::Error>> {
use avenger_datafusion_dataflow::DataflowBuilder;
use avenger_selection::QueryPolicy;
use datafusion::{functions_aggregate::expr_fn::count, logical_expr::{col, lit, LogicalPlanBuilder}};

let mut additional = DataflowBuilder::with_base(&base.interface());
let source = additional.import_table("flights", source_output)?;
let family = filter.query(source.plan_ref(), |rows| {
    LogicalPlanBuilder::from(rows)
        .aggregate(vec![col("carrier")], vec![count(lit(1_i64)).alias("count")])?
        .build()
})?.plan(selections).focus(delay).build()?;
let installed = family.install(&mut additional, "airline_counts")?;
let extension = base.prepare_extension(&additional.finish()?).await?;

let binding = installed.bind(selections)?;
let inputs = binding.apply(extension.inputs())?.finish()?;
let output = binding.output();
let result = extension.query(&[output], &[], base_inputs, &inputs).await?;
let table = result.table(&output)?;
# let _ = table;
# Ok(())
# }
```

Keep the same installed family and prepared extension across updates. Each
binding owns its current predicate, supplies its generated inputs through
`apply`, and chooses the output through `output`. Chart rendering does not
branch on strategy. Several installations can apply bindings to one input
builder and request their outputs together. `apply` also accepts `inputs.edit()`
and preserves unrelated bindings. The caller must still supply every unrelated
required root input.

Both `installed.bind` and `installed.bind_with_policy` use this path. A policy
override does not require another preparation. Compatible installations have
three expression inputs: full, fixed, and changing predicates. They expose a
direct output, a materialization output, and a final aggregate output. Direct-only
families have just the full predicate and direct output. `apply` supplies every
owned input, including neutral predicates on unused optimized branches.

`preaggregate_output()` returns the compatible materialization output, or `None`
for direct bindings. On plot entry, bind an inactive focus and query those
outputs to warm the same nodes that final queries depend on. This does not
change selections or render a chart. Overlapping queries share in-progress
work through dataflow. Dropping a query detaches that consumer's interest.

The materialization depends on the source, fixed predicate, and concrete plan.
Focused bounds and suffix-only parameters affect the final node. Inputs used by
measure filters or arguments remain materialization dependencies. Unknown deferred
expression inputs in moved expressions use the direct path. Ordinary scalar
inputs can be resolved in an upstream source node and exposed as typed columns.
Each binding checks the fixed selection predicate's safety before allowing
warm-up, and validates its focused predicate through the generic planner.
A new source `TableSnapshot` or a
changed fixed predicate creates a new cache dependency. Externally mutable
sources still need the dataflow's explicit refresh/version policy. Retention
is subject to the runtime's LRU budget: eviction, `clear_results`, and disabled
caching can require rebuilding after warm-up. Keeping a preparation does not
guarantee keeping its results. The controller separately bounds the number of
preparations it keeps when switching focus.

The installed three-view example starts with inactive selections, applies a
delay brush `[10, 40)`, drags its lower edge to `[11, 40)`, and repeats that last
request. It first warms compatible materializations with the inactive snapshot.
The default output follows these steps, states which outputs each step requests,
and prints pretty result tables in a stable order. Each step reports the actual
executed-node names, cache hits, and shared in-progress computations from the
runtime's `EvaluationReport`. A legend explains the materialization, aggregate,
and direct node names.

```sh
cargo run -p avenger-selection --features dataflow --example query_families
cargo run -p avenger-selection --features dataflow --example query_families -- --force-direct
cargo run -p avenger-selection --features dataflow --example query_families -- --sql
cargo run -p avenger-selection --features dataflow --example query_families -- --aggregates --sql
```

The count-only default keeps the tables small. `--aggregates` adds fare measures
to both receiver charts, with null values and unequal cell populations. It
supports `--force-direct` and `--sql`. After the displayed request sequence, the
example checks all results against the opposite policy. These validation queries
do not contribute to the displayed cache reports.

The example retains each `QueryFamily` beside its installation.
`family.bind(&state)` exposes either a direct plan or both pre-aggregation stages.
The optional `--sql` appendix prints the bound SQL separately from the execution
trace. It prints each materialization as its own query and gives the final
aggregate a schema-only reference to that named relation for inspection. This
keeps the reuse boundary visible. Later steps print only changed SQL text.
Printing a plan does not execute it or report a cache lookup. `--sql` can also
be combined with `--force-direct`.
`query.logical_plan(&state)` is also available
when inspecting the ordinary direct query without a family. The dataflow crate's
`additional.sql().plan(&plan)` formats that plan using DataFusion's unparser and
renders its source reference as `nodes.flights`. Generated selection UDFs remain
visible. This requires the dataflow crate's `sql` feature, enabled for this
example. The displayed query describes the computation at that named boundary.
See [SQL diagnostics](../avenger-datafusion-dataflow/README.md#inspect-plans-and-expressions-as-sql)
for formatting stored table nodes, scalar nodes, and native expressions.

## Integration boundary

The chart compiler owns name resolution, producer declarations, consumer/view
associations, and lineage mappings. The controller owns events and immutable
state updates. Dataflow owns execution, caching, shared in-progress work, and
cancellation. Selection state contains no event queues or result cache.

The core crate depends on DataFusion, the Avenger scale adapter, and the
standalone pre-aggregation planner. It has no chart controller dependency. The optional
`dataflow` feature adds the runtime adapter for root installations. Root and
scoped dataflow Expr inputs also accept the predicates directly. Dataflow is a development dependency for integration
tests and the direct example.

The implementation covers typed state, direct predicates, pixel membership,
aggregate query families, and root/extension installation with optional warm-up.
Selection owns focus, factorization, and interaction eligibility. The generic
planner owns relational rewriting and State/Merge expression construction. The
aggregate-state utility owns state encoding and accumulator behavior. Dataflow owns execution
and result reuse. Selection-state serialization and complete-dataflow UDF codecs
are deferred. Arrow state transport alone does not serialize the functions.
