# Cartesian Partitioned Subplots Plan

## Status

Implemented in this milestone:

- `Subplot<Cartesian>::partition_by(...)` stores an unscaled internal
  `"partition"` channel.
- `CompiledCartesianSubplot` stores the original partition expression for
  runtime filtering against raw parent data.
- Partitioned mode rejects explicit child plot data, missing/conditional
  placement channels, and raw row-level non-aggregate `x`/`y` placement.
- Cartesian positioned measurement now has a partitioned path that creates one
  child frame per partition value, filters raw parent data per child, prepares a
  separate `PreparedChildFramePlot` per partition, and renders from saved
  measurement state.
- Child-frame scope identity distinguishes row-positioned children from
  partition-positioned children with `PositionedPartition` keys/path segments.
- Focused unit coverage verifies aggregate placement without explicit parent
  domains, filtered child data per partition, raw placement rejection, and
  explicit child-data rejection.

Still deferred:

- Broadcast/filter/skip data selection for child marks inside partitioned
  containers.
- Visual baseline dogfood for mini bar/scatter charts.
- Explicit ordering, size encodings, overlap handling, and the shorter
  `.partition(...)` alias.

## Visual Baseline Plan

Add a small set of image baselines that exercise partitioned positioned
subplots as an author-facing feature. These should live with the existing
positioned-subplot visual tests in
`avenger-chart/tests/visual_tests/test_cartesian_subplot.rs` until the test
module is renamed or split.

### Cartesian Outer Baselines

1. `cartesian_partitioned_subplot_scatter`

   Parent coordinate system: `Cartesian`.

   Child coordinate system: `Cartesian`.

   Shape: parent data partitioned by a categorical field such as `species`;
   parent placement uses `x = avg(col("sepal_length"))` and
   `y = avg(col("petal_length"))` with no explicit parent domains. Each child
   subplot renders a mini scatter plot from inherited partition data.

   Purpose: this is the canonical baseline for aggregate parent placement,
   inferred parent domains, filtered child data, child axes/guides, and stable
   partition group naming.

2. `cartesian_partitioned_subplot_mixed_child_coords`

   Parent coordinate system: `Cartesian`.

   Child coordinate systems: one partitioned subplot mark with `Cartesian`
   children and one partitioned subplot mark with `Polar` children, or a single
   partitioned subplot whose child plot is `Polar`.

   Shape: reuse the same partitioned parent data, but let the child plot render
   polar symbols using inherited partition rows.

   Purpose: proves the partitioned positioned-subplot data path is independent
   of the child plot coordinate system. This is not a Polar outer-coordinate
   test; it is still a Cartesian parent positioning test.

### Polar Outer Baselines

Polar outer baselines require a small implementation prerequisite: `Polar`
must implement `SubplotContainerCoordinateSystem` for `Subplot<Polar>` and the
top-level crate must add a Polar positioned-subplot measurement/render path.
That should reuse the same partition/filter child-data semantics introduced for
Cartesian positioned subplots, but with parent placement channels `r` and
`theta` instead of `x` and `y`.

3. `polar_partitioned_subplot_cartesian_children`

   Parent coordinate system: `Polar`.

   Child coordinate system: `Cartesian`.

   Shape: parent data partitioned by category; parent placement uses
   `theta = avg(col("angle"))` and `r = avg(col("radius"))`; each child subplot
   renders a compact Cartesian scatter or bar-like view of the inherited
   partition rows.

   Purpose: first true Polar outer-coordinate baseline. It should visibly place
   child frames around a polar coordinate system while proving partitioned child
   filtering still works.

4. `polar_partitioned_subplot_polar_children`

   Parent coordinate system: `Polar`.

   Child coordinate system: `Polar`.

   Shape: same parent partitioning and parent `theta`/`r` aggregate placement,
   but child subplots render mini polar scatter plots from each filtered
   partition.

   Purpose: exercises Polar as both parent and child coordinate system. Keep the
   visual simple so failures clearly indicate whether the parent placement,
   child filtering, or child polar rendering changed.

### Baseline Acceptance Checks

- No explicit parent domains in the aggregate-placement cases unless the visual
  design genuinely requires fixed framing.
- At least one partition has a different row count than another, so child mark
  counts make filtering visually and programmatically detectable.
- Use deterministic partition values and aggregate positions to avoid noisy
  baseline movement.
- Keep child plot sizes modest, with enough spacing that overlap is not the
  feature under test.
- Add focused scene-graph assertions before/alongside image baselines where
  practical: group count equals partition count, child group names include
  partition values, and child mark counts match filtered row counts.

## Goal

Add the first real partitioned mode for `Subplot<Cartesian>`:

```rust
Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Subplot::new(child_plot)
            .partition_by(col("species"))
            .x(avg(col("sepal_length")))
            .y(avg(col("petal_length")))
            .plot_size(90.0, 70.0)
    );
```

This means one positioned child frame per partition value. The parent
Cartesian coordinate system positions each child frame from partition-level
placement expressions, while the child plot receives the rows for that
partition as its inherited data.

This is not row/column faceting. It uses the Cartesian parent coordinate system
for placement rather than facet bands, but it shares the same core data idea:
child plots inherit from a partitioned parent dataset.

## Current Code Facts

- `Subplot<Cartesian>` already exists in `avenger-chart-cartesian` and supports
  `x`, `y`, `plot_width`, `plot_height`, and `plot_size`.
- Current Cartesian positioned subplot measurement evaluates `x` and `y` from
  the subplot mark data and creates one child per resulting row.
- Current child frame measurement is already routed through
  `ChildFrameRuntime`, so measured child frames participate in domain sharing,
  guide/legend layout, debug overlays, and rendering from saved measurements.
- Current row/column facets build filtered `data_override` values per facet
  cell and pass those into child plots.
- `FacetStrategy::Broadcast` exists on marks, but the current row/column facet
  render path does not yet choose full parent vs filtered partition data per
  child mark. Broadcast/filter/skip should be handled later for all partition
  containers together.
- Ordinary aggregate encodings are currently handled during `Plot::compile` by
  replacing the mark dataframe with an aggregated dataframe and rewriting
  channel expressions. That is correct for ordinary marks, but partitioned
  subplots need two data views:
  - a placement summary dataframe, one row per partition, for parent `x`/`y`,
  - the raw parent dataframe filtered per partition, for the child plot.

## MVP Semantics

### Non-partitioned mode

No behavior change. If a `Subplot<Cartesian>` has no partition channel, keep
the existing row-wise positioned-subplot behavior:

- evaluate `x` and `y` against the subplot mark data,
- create one child frame per output row,
- use the current explicit-child/inherited-parent data selection behavior.

### Partitioned mode

A `Subplot<Cartesian>` enters partitioned mode when it has a partition channel.
The public method should be:

```rust
.partition_by(expr)
```

Internally this can store a channel named `"partition"` on the subplot mark.

Rules:

- The child plot must not have plot-level `.data(...)` in partitioned mode.
- The child plot inherits data from the parent plot or from the current parent
  partition if this is nested under another container.
- The parent data is grouped by the partition expression.
- Parent placement channels `x` and `y` are evaluated once per partition.
- `x` and `y` expressions must be one of:
  - aggregate expressions, such as `avg(col("x"))`,
  - literals,
  - the partition expression itself.
- Non-aggregate raw row expressions such as `.x(col("longitude"))` should be a
  clear compile error in partitioned mode, because there is no unique row-level
  value for a partition.
- Each child receives `parent_data.filter(partition_expr == partition_value)`.
- Child identity should be stable by partition value, not only by transient
  output row index. The first implementation can preserve the existing
  `row_index` field for ordering, but the child key/path should include a
  formatted partition value where possible.

## Important Design Constraint

Do not let compile-time aggregate rewriting turn the partitioned subplot's
compiled mark dataframe into the only available data source.

For partitioned subplots, the implementation must preserve access to raw parent
data at measurement/render time. Otherwise the child plot would receive the
one-row-per-partition placement summary instead of the rows belonging to the
partition.

The clean shape is:

```text
parent data
   |
   |-- placement summary:
   |     group by partition, compute aggregate/literal/key x/y
   |
   |-- child data overrides:
         one filtered dataframe per partition value
```

## Full Implementation Milestone

Implement the full partitioned Cartesian subplot feature in one coherent pass,
not as a sequence of narrow product slices. Internal checkpoints are still
useful for keeping the code reviewable, but the milestone is not complete until
the API, scale inference, child data partitioning, measurement/rendering, and
dogfood coverage all work together.

### Public API

Add one public authoring method to `CartesianSubplotPositionChannels`:

```rust
fn partition_by<V: Into<ChannelValue>>(self, value: V) -> Self;
```

Do not add a shorter `.partition(...)` alias in the first implementation. The
more explicit name is easier to search, avoids ambiguity with internal
partition helpers, and can be aliased later without breaking users.

The method stores a channel named `"partition"` on `Subplot<Cartesian>`.
`CompiledCartesianSubplot::supported_channels()` should include this as an
optional column-reference channel so the existing mark data preparation path can
project partition values alongside scaled `x`/`y` placement values.

### Compile-Time Semantics

`SubplotContainerCoordinateSystem for Cartesian` should inspect the original
subplot mark view before it builds `CompiledCartesianSubplot`.

If no `"partition"` channel exists, compile exactly as today.

If `"partition"` exists:

- Reject child plots with plot-level `.data(...)`. Partitioned Cartesian
  subplots inherit parent data; explicit child data would make partitioning
  ambiguous.
- Store the original partition expression on `CompiledCartesianSubplot`.
  Runtime filtering must use this original expression against raw parent data,
  not a rewritten aggregate-output column.
- Validate parent placement channels:
  - literals are valid,
  - aggregate expressions are valid,
  - the partition expression itself is valid,
  - any other raw row-level expression is invalid.
- Keep using the existing aggregate compile rewrite when `x` or `y` contains
  aggregates. Because the partition channel is a non-aggregate channel on the
  mark, the existing rewrite groups by partition and produces a placement
  summary dataframe. That summary is useful for parent placement and scale
  inference as long as raw parent data is still used for child filtering.

This keeps aggregate placement domain inference aligned with existing aggregate
mark behavior. Parent `x`/`y` domains should infer from the placement summary
without requiring users to set explicit domains.

### Runtime Data Model

Refactor Cartesian positioned subplot preparation around an explicit child
planning model:

```text
PreparedPositionedSubplot
  - subplot: &CompiledCartesianSubplot
  - children: Vec<PreparedPositionedChild>

PreparedPositionedChild
  - spec: PositionedChildSpec
  - prepared_child_plot: PreparedChildFramePlot
```

The existing non-partitioned path may keep using a shared
`PreparedChildFramePlot` internally for efficiency, but the data model should
allow partitioned children to carry their own `PreparedChildFramePlot` because
each partition has a different inherited data override and therefore different
local child domains.

For partitioned subplots:

1. Resolve parent data from the current coordinate measurement input. This may
   be top-level plot data or a filtered parent data override when nested inside
   another container.
2. Use the prepared subplot mark data only as the placement summary source.
   For aggregate `x`/`y`, this will often be the compile-time aggregate output.
3. Extract one child spec per partition value from the placement summary:
   partition value, scaled `x`, scaled `y`, order index, mark index, and
   optional user key/label.
4. Sort partitioned children deterministically by partition value unless the
   user later gets an explicit order channel.
5. Build the child data override by filtering the raw parent dataframe with
   `original_partition_expr == partition_value`.
6. Call `ChildFrameRuntime::prepare_plot(...)` separately for each partition
   child with `ChildFrameDataSelection::InheritParent` and the filtered data.

For non-partitioned subplots, preserve the current behavior: one child per
prepared mark row and the existing inherited/explicit child data selection.

### Child Identity And Paths

Add explicit partitioned-positioned identity variants instead of overloading
row indices:

```rust
ChildFrameKey::PositionedPartition {
    mark_index,
    value,
    key,
}

ContainerPathSegment::PositionedPartition {
    mark_index,
    value,
    key,
}
```

Then add a `ChildFrameSharingLevel::positioned_partition(...)` constructor.
This keeps non-partitioned row-wise positioned subplots distinct from
partitioned positioned subplots in guide/legend/domain sharing keys, debug
paths, and future broadcast/filter/skip work.

Scene group names can continue to include the child index for uniqueness, but
partitioned groups should also include a formatted partition value when no user
key is provided. That makes debug output intelligible without making identity
depend on string formatting.

### Scale And Domain Behavior

No temporary explicit-domain requirement. The complete feature should infer
parent placement domains.

Expected behavior:

- Aggregate placement channels infer from the compile-time placement summary.
- Partition-key placement channels infer from the parent data domain, which is
  equivalent for min/max or distinct-domain purposes.
- Literal placement channels remain scalar and do not require a data domain.
- Child plot domains are built per partition through
  `ChildFrameRuntime::prepare_plot`, then coordinated through the existing
  child-frame domain sharing machinery.

If implementation reveals a case where scale inference still sees raw rows
instead of partition summaries, fix the scale-builder path as part of this
milestone rather than deferring it. The intended user experience is that the
example in the Goal section works without explicit parent `x`/`y` domains.

### Measurement And Rendering

Keep the existing shape: Cartesian positioned child frames are measured during
coordinate measurement and rendered from saved measurement state.

Update the positioned-subplot measurement path (now the generic facade-owned
runtime rather than the old Cartesian-specific measurement function) so
partitioned and non-partitioned children both produce:

- `CartesianPositionedChildMeasurement`,
- `ChildFrameDomainSharingInput`,
- `ChildFrameRenderPlacement`,
- child-frame overflow through the existing `CoordMeasurement` implementation.

Rendering should not recompute partitions. It should read the saved
`CartesianPositionedChildMeasurement`, pass the saved child measurement and
saved filtered data override to `build_plot_components(...)`, and translate the
result into the saved render placement.

### Error Messages

Add clear errors for:

- partitioned Cartesian subplot without parent data,
- partitioned Cartesian subplot whose child plot has `.data(...)`,
- partitioned Cartesian subplot missing `x` or `y`,
- partitioned Cartesian subplot with raw row-level non-aggregate placement,
- partitioned Cartesian subplot whose placement summary does not include the
  partition channel,
- partitioned Cartesian subplot where `x`, `y`, and partition arrays disagree
  in length.

These should be `InvalidArgument` when caused by chart author input and
`InternalError` only for violated internal invariants.

### Tests And Dogfood

Add focused tests before visual baselines:

- compile rejects child plot `.data(...)` in partitioned mode,
- compile rejects raw row-level `x`/`y` in partitioned mode,
- aggregate `x`/`y` with `.partition_by(...)` compiles without explicit parent
  domains,
- partition-key/literal placement compiles,
- child group count equals partition count,
- child groups are positioned at aggregate coordinates,
- child plot output changes per partition, proving the child sees filtered
  partition rows.

Add visual coverage in `avenger-chart/tests/visual_tests/test_cartesian_subplot.rs`:

- partitioned Cartesian mini bar charts:
  parent `x = avg(...)`, `y = avg(...)`, child plot is a small Cartesian bar
  chart using inherited partition data,
- partitioned Cartesian mini scatter plots:
  child plot shows only the partition rows,
- optional after Polar has the needed mark support:
  partitioned mini Polar scatter/pie-like child plot.

No broadcast coverage in this milestone.

### Validation For The Milestone

Use a broader validation gate than a tiny mechanical move:

```bash
cargo check -p avenger-chart --all-targets
cargo test -p avenger-chart --lib cartesian -- --nocapture
cargo test -p avenger-chart --test visual_regression cartesian_subplot -- --nocapture
cargo test --manifest-path avenger-chart-external-test/Cargo.toml -- --nocapture
cargo clippy --release -p avenger-chart --all-targets
cargo fmt --all --check
git diff --check
```

Run the full release chart lib test if the implementation touches generic mark
aggregation, scale-builder behavior, or child-frame sharing keys beyond
Cartesian positioned subplots.

## Broadcast/Filter/Skip Follow-Up

Set aside for a shared follow-up across row/column facets and Cartesian
partitioned subplots.

The right later shape is a partition-aware data context that carries:

- full inherited parent dataframe,
- current filtered partition dataframe,
- partition path/value metadata.

Then mark preparation chooses data by `FacetStrategy`:

- `Filter`: current partition dataframe,
- `Broadcast`: full inherited parent dataframe,
- `Skip`: render only when compatible with the current partition context.

The first partitioned Cartesian subplot implementation should continue using
filtered inherited data for all child marks, matching current row/column facet
behavior.

## Deferred Work

- Broadcast/filter/skip data selection for child marks inside partitioned
  containers.
- A shorter `.partition(...)` alias.
- Partition-level `plot_width` / `plot_height` / size encodings.
- Explicit partition ordering.
- Collision avoidance between overlapping child frames.
- Polar bar/pie-specific dogfood once Polar has the needed mark support.

## Suggested Commit Shape

This should land as one feature branch milestone, with commits split only along
reviewable code boundaries:

```text
feat(chart): add partitioned Cartesian subplots
test(chart): cover partitioned Cartesian subplots
```
