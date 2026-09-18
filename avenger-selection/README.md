# avenger-selection

Immutable selection state and consumer-specific DataFusion predicates. Several
views can contribute points or intervals to one named selection. Cross-filter
uses exclude contributions from the consuming view's own instance.

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
for an open end. Ranges exclude null and non-finite rows, even with both ends
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

## Integration boundary

The chart compiler owns name resolution, producer declarations, consumer/view
associations, and lineage mappings. The controller owns events and immutable
state updates. Dataflow owns execution, caching, shared in-progress work, and
cancellation. Selection state contains no event queues or result cache.

The core crate depends on DataFusion, not on dataflow, rendering, or chart
components. Root and scoped dataflow Expr inputs already accept the predicates.
Dataflow is a development dependency for integration tests and the example.

The current implementation covers exact typed state and direct predicates
(phases 1–2). Pixel membership, query-family planning, and pre-aggregation
installation remain later phases. Selection-state serialization is deferred.
