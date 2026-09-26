# avenger-selection

Immutable selection state and consumer-specific DataFusion predicates. Several
views can contribute points or intervals to one named selection. Cross-filter
uses exclude contributions from the consuming view's own instance. Charts can
pass the generated expressions to DataFusion, dataflow, and the independent
[preaggregate planner](../avenger-datafusion-preaggregate/README.md).

## Generate a cross-filter predicate

```rust
use avenger_selection::*;
use datafusion::logical_expr::{col, Expr};

let selection = SelectionId::new("filters")?;
let delay_view = ViewId::new("delay_histogram")?;
let delay_projection = ProjectionId::new("delay")?;
let brush = ProducerDefinition::new(
    selection.clone(),
    ProducerId::new("delay_brush")?,
    delay_view,
    [Projection::new(delay_projection.clone(), col("delay"))?],
)?;

let before = SelectionSet::new([(selection.clone(), Resolution::Intersect)])?;
let after = before.set(
    &brush,
    SelectionValue::tuple([(delay_projection, ValueTest::range(10_i64..30_i64))]),
)?;

let filter = ConsumerFilter::new(
    ViewId::new("distance_histogram")?,
    SelectionFilter::cross_filter([&selection]),
);
let predicate: Expr = filter.predicate(&after)?;
// Bind predicate to a dataflow Boolean Expr input, or filter a DataFusion plan.
assert_eq!(before.contributions(&selection)?.count(), 0);
assert_eq!(after.contributions(&selection)?.count(), 1);
# Ok::<(), avenger_selection::Error>(())
```

Keep the `ConsumerFilter` and pass a new immutable `SelectionSet` after each
update. Use `state.contributions(&selection_id)?` to inspect producers and
selected values for brush overlays. `Contribution::value()` exposes the original
typed bounds, and `effective_value()` exposes the comparisons used by predicates.

`set(&producer, value)`, `toggle(&producer, value)`, and `clear(&producer)` route
to the selection named by the producer. `clear_all(&selection_id)` clears that
named selection. For event queues or atomic updates across producers and names,
use `apply(update)` or `apply_all([update, ...])` with `SelectionUpdate` values.
The updates carry their own destination. `SelectionUpdate::clear` takes a
producer, just like `SelectionSet::clear`.

The runnable example builds two histograms and categorical counts in a dataflow,
applies three producers under one name, changes the delay brush, and verifies
reuse of unchanged results:

```sh
cargo run -p avenger-selection --example direct_crossfilter
```

## Selection semantics

- `SelectionId` identifies a shared name. Each declaration has a separate
  `ProducerId`, and each contribution retains its opaque `ViewId`.
  A producer is identified by its selection, declaration, and view together.
  Inspect them with `selection()`, `id()`, and `view()`.
- `Resolution::Intersect` ANDs active producer predicates. `Union` ORs them.
  A global `set` replaces all contributions. Global toggles can retain
  tuples from multiple origins and compare projected meanings across producers.
- Terms inside a tuple combine with AND. Tuples inside one producer combine
  with OR, preserving correlations across dimensions. Values define the
  comparisons: equality, sets, or ranges. The chart owns the interaction kind.
- `clear(&producer)` removes only that producer in every mode.
  `clear_all(&selection_id)` removes all contributions to that name. An absent contribution is inactive. An explicitly
  empty `set` remains active and matches no rows. Toggling the last tuple off
  removes that contribution.
- Membership retains all producers. Cross-filtering excludes every producer
  with the consuming view's exact ID. The chart assigns distinct IDs to
  distinct facet instances, including nested facets, and shares an ID across
  layers that should exclude the same producers. Selection treats IDs as opaque.
  The chart also supplies any required facet-key predicates.
- Each use specifies behavior for an inactive selection. If active producers
  exist but all are excluded, that leaf is unrestricted even with
  `EmptySelection::MatchNone`. `All`, `Any`, and `Not` compose named uses without
  silently changing those rules. Missing names are errors in every branch.

Every update validates before returning a new snapshot. `apply_all` applies
ordered updates atomically. Tuple order, duplicate tuples, and set order are
canonicalized so equivalent updates can produce equal dataflow bindings.
Changing a producer definition requires a replacement `set` before toggling.

## Values and consumer mappings

Construct a single tuple from `(ProjectionId, ValueTest)` pairs. Use
`ValueTest::equal(value)`, `one_of(values)`, or `range(bounds)` for comparisons.
For correlated alternatives, use `SelectionValue::tuples`:

```rust
# use avenger_selection::*;
let carrier = ProjectionId::new("carrier")?;
let delay = ProjectionId::new("delay")?;
let value = SelectionValue::tuples([
    [
        (carrier.clone(), ValueTest::equal("AA")),
        (delay.clone(), ValueTest::range(10_i64..30_i64)),
    ],
    [
        (carrier, ValueTest::one_of(["DL", "UA"])),
        (delay, ValueTest::range(40_i64..60_i64)),
    ],
]);
// AA with delay [10, 30), OR DL/UA with delay [40, 60).
# let _ = value;
# Ok::<(), avenger_selection::Error>(())
```

Each tuple must include every projection declared by its producer exactly once.
`value.as_tuples()` exposes these same nested pairs for inspection.
`SelectionValue::default()` contains no tuples and matches no rows when set.

Values preserve Arrow scalar types, including integer widths, decimal precision
and scale, and timestamp units and zones. Comparisons use DataFusion 54.1 type
semantics at the consuming plan, without JavaScript value coercion.

Equality and sets are null-safe. Floating point selection treats all NaN
payloads as one selectable value and both signs of zero as equal. Ranges reject
null, NaN, reversed, or differently typed bounds. Use Rust ranges such as `10_i64..30_i64`, `10_i64..=30_i64`,
`10_i64..`, or `..30_i64` with `ValueTest::range`. For both ends open, use
`ValueTest::range::<i64>(..)`. Exact ranges exclude null and non-finite rows, even with both ends
unbounded. Included/excluded endpoints remain explicit. Final membership is a
non-null Boolean, including under `Not`.

Selection values support flat Boolean, numeric, string, binary, date, time,
timestamp, and duration scalars. Nested/list values are outside this model.

`ConsumerFilter::new(view, filter)` uses producer projections directly for a
shared row relation. Use `with_projection(&producer, &projection_id, expr)` for a
compiler-verified mapping to renamed or transformed fields. Mapping keys include
the full producer identity so local projection names cannot collide. The compiler
owns the proof that relations and projections have the same meaning.

Row IDs use ordinary projections and values:

```rust
# use avenger_selection::*;
# use datafusion::logical_expr::col;
let selection = SelectionId::new("picked_rows")?;
let row_id = ProjectionId::new("row_id")?;
let producer = ProducerDefinition::new(
    selection.clone(),
    ProducerId::new("points")?,
    ViewId::new("scatter")?,
    [Projection::new(row_id.clone(), col("id"))?],
)?;
let state = SelectionSet::new([(selection, Resolution::Union)])?.set(
    &producer,
    SelectionValue::tuples([2_i64, 5_i64].map(|id| [
        (row_id.clone(), ValueTest::equal(id)),
    ])),
)?;
let state = state.toggle(
    &producer,
    SelectionValue::tuple([(row_id, ValueTest::equal(5_i64))]),
)?;
// Only row ID 2 remains selected.
# assert_eq!(state.contributions(producer.selection())?.next().unwrap().value().as_tuples().len(), 1);
# Ok::<(), avenger_selection::Error>(())
```

The chart must verify that a row-ID projection identifies the same rows in every
consumer. Use ordinary projection mappings when the ID column is renamed.
Filtering can preserve identity. Joins, aggregates, and regenerated IDs require
the chart to verify lineage before reusing a selection.

`toggle` adds or removes whole normalized tuples, including ranges and sets.
It compares original typed values and bounds before pixel mapping. Overlapping
ranges remain separate tuples, and two different ranges can select the same
pixel cells. Toggling one removes only that tuple. To toggle individual row IDs,
use one tuple per ID, as above. A `one_of` comparison toggles as one whole set.

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
use avenger_selection::PixelGrid;
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
# let _ = cell_expression;
# Ok::<(), avenger_selection::Error>(())
```

Call `brush.with_pixel_grids([(projection_id, grid)])` to configure pixel
membership for a projection. Each grid must identify a declared projection,
use a positive size, and have finite nondegenerate domains and ranges in the
scale kernel's precision. Different projections can use different cell sizes.
Gridded projections require range comparisons. Ungridded projections retain
exact equality, set, or range comparisons. `with_pixel_grids([])` removes all
grids from the returned definition.

Pixel precision changes selected membership consistently. It is not an
optimization hint. Included/excluded bounds apply to entire cells. On the grid
above, `[10, 30)` selects cells `[15, 45)`. An included upper endpoint includes
cell 45, including other values that map to it. An excluded endpoint removes its
entire boundary cell. Two distinct endpoints can map to one cell and yield an
empty interval. Decreasing scales swap endpoint positions and flags. Unbounded
ends preserve their data-space direction.

`Contribution::value()` retains the original typed bounds.
`Contribution::effective_value()` exposes checked cell bounds. The predicate
resolver uses mapped expressions and effective bounds without evaluating brush
endpoints again. `predicates(...).split()` exposes the mapped interaction
dimensions when a fixed/changing split is supported.

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
producer with `with_pixel_grids` and call `state.set` with the old contribution's
retained raw values. Old snapshots preserve the old membership.
Consumer mappings are applied before pixel mapping. Device pixel ratio is not
an input: all coordinates are chart-local logical pixels.

The three-view example supports pixel precision for both brushes:

```sh
cargo run -p avenger-selection --example direct_crossfilter -- --pixels
```

## Predicate information for preaggregation

A chart can identify a focused producer before it has an active selection.
`predicates` resolves the state once, returning the full predicate and an optional
fixed/changing split. Both predicates and the interaction dimensions use the
consumer's source-row expressions.

```rust
use avenger_selection::*;
use datafusion::logical_expr::Expr;
# fn example(filter: &ConsumerFilter, state: &SelectionSet,
#            focus: &ProducerDefinition) -> avenger_selection::Result<()> {
let predicates = filter.predicates(state, focus)?;
let full: &Expr = predicates.full();
match predicates.split() {
    Ok(split) => {
        let fixed: &Expr = split.fixed();
        let changing: &Expr = split.changing();
        let dimensions: &[Expr] = split.dimensions();
        // The chart gives these expressions to its preaggregate planner.
#       let _ = (fixed, changing, dimensions);
    }
    Err(reason) => {
        // The chart's direct query always uses complete current membership.
        println!("Selection split unavailable: {reason}");
#       let _ = full;
    }
}
# Ok(())
# }
```

`fixed AND changing` preserves the membership of `full`. The preaggregate planner
checks query eligibility against the actual relation and target query. The caller
is responsible for expressions being valid over the entire warm-up dataset.

Intersecting producers and enclosing `All` branches support splitting. Focused
union/global resolution, `Any`, and `Not` remain direct initially. Branches that
do not use the focus can stay intact in `fixed`. An unused or self-excluded focus
has no split. Row-ID projections supply interaction dimensions just like other
projections. Invalid names and mappings remain errors.

An inactive focus still supplies dimensions for warm-up. Its changing predicate
respects the empty policy, including match-none. A current focused contribution
with a different definition or grid returns `SplitReason::IncompatibleFocus`.
The full predicate always uses the actual current state.

For an exact delay brush, the dimension is the mapped delay expression. For a
pixel brush, it is the captured grid's cell expression. A receiving distance
histogram still supplies its own distance display bins in its query. The generic
planner groups materialized states by both the original display groups and the
explicit interaction dimensions. It rewrites the changing predicate into those
stored columns when binding.

### Native DataFusion queries

Start with the self-contained [native-query example](examples/preaggregate_queries.rs).
It uses `SessionContext`, native plans, and ordinary `MemTable` values to show
two histograms and categorical counts. It executes each receiver's warm-up once,
stores the resulting batches, and reuses that table across brush, drag, and clear
queries. The focused histogram uses its full direct predicate.

```sh
cargo run -p avenger-selection --example preaggregate_queries
cargo run -p avenger-selection --example preaggregate_queries -- --sql
```

This example owns materialization explicitly. Each rollup executes again, with
no automatic result cache. Only the focused bounds change in its state sequence.
Changed fixed predicates or dimensions require a new preparation and stored table.
It prints pretty tables and optionally SQL, then runs extra direct queries to
verify every result. The example uses no dataflow API or support module.

### Add dataflow caching

The [preaggregate example](examples/preaggregate_crossfilter.rs) builds three
ordinary direct queries in a base dataflow. It then builds an optimized extension
for the focused producer and concrete fixed predicates using
`avenger-datafusion-preaggregate::FilterQuery` and `PreaggregatePlanner`.

The example reuses the extension through brush, drag, and unchanged requests.
A changed fixed predicate prepares a replacement while retaining the base and
its source cache. A changed pixel grid uses the full direct query until the
replacement is ready. Capturing fixed predicates simplifies the integration at
the cost of planning again when another selection changes.

Selection's pixel and numeric classification functions work with the default
preaggregation planner. Expressions must be valid over the entire warm-up dataset,
including rows outside the current selection. See the planner's
[warm-up contract](../avenger-datafusion-preaggregate/README.md#warm-up-expression-contract).

The example's small [composition helper](examples/support/composition.rs) is
private chart-side code, not part of the selection library API. It checks whether
the fixed predicate and dimensions still match the focused preparation. The
optional `avenger-datafusion-preaggregate::dataflow` adapter installs the plans,
validates changing predicates, and applies their dataflow bindings. The base
retains the complete direct query so fallback cannot inherit a stale fixed
predicate. Selection has no normal dependency on this adapter.

```sh
cargo run -p avenger-selection --example preaggregate_crossfilter
cargo run -p avenger-selection --example preaggregate_crossfilter -- --sql
cargo run -p avenger-selection --example preaggregate_crossfilter -- --pixels --aggregates
cargo run -p avenger-selection --example preaggregate_crossfilter -- --pixels --force-direct
```

The example prints pretty tables and actual runtime reports. `--sql` adds plans
and predicate expressions using dataflow's SQL formatter. `--force-direct`
selects complete direct queries while preserving membership, including pixels.
Reference queries run after the reported sequence to avoid warming its cache.

Warm-up requests the same materialization output consumed by a rollup and does
not create a selection. A brush request can start while warm-up is pending.
Dataflow shares matching work and keeps it running while any request needs it.
The chart owns focus, preparation lifetime, and current-scene checks. Selection
has no scheduler, readiness tracking, cache, graph handles, or execution policy.

The [preaggregate README](../avenger-datafusion-preaggregate/README.md) owns query
eligibility and the aggregate numerical contract. The
[dataflow README](../avenger-datafusion-dataflow/README.md) owns execution,
caching, and cancellation. Both crates are development dependencies here, so
ordinary predicate users need neither companion crate as a selection dependency.
