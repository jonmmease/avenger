# Cartesian Partitioned Subplots Plan

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

## Recommended Implementation Chunks

### Chunk 1: Add API and compiled metadata

Files:

- `avenger-chart-cartesian/src/marks/subplot.rs`
- `avenger-chart-marks/src/subplot.rs`
- `avenger-chart-core/src/subplot_child_plot.rs` only if a hidden core accessor
  becomes necessary
- `avenger-chart/src/prelude.rs` if a new extension trait name is introduced

Work:

- Add `partition_by<V: Into<ChannelValue>>(...)` to
  `CartesianSubplotPositionChannels`.
- Store the value as the `"partition"` channel.
- Add `"partition"` to `CompiledCartesianSubplot::supported_channels()` as an
  optional column-ref channel.
- During `SubplotContainerCoordinateSystem for Cartesian` compilation, inspect
  the original subplot channel map for `"partition"`.
- Store the original partition expression on `CompiledCartesianSubplot`, for
  filtering raw parent data later.
- If partitioned and `subplot.has_plot_level_data()` is true, return a clear
  invalid-argument error explaining that partitioned Cartesian subplots inherit
  parent data.
- Validate `x`/`y` expressions for partitioned mode using the original channel
  expressions:
  - aggregate/literal/partition-key expression: OK,
  - other row-level expression: error.

Validation:

- Focused unit tests around compile-time validation:
  - partitioned subplot rejects child plot `.data(...)`,
  - partitioned subplot rejects raw row-level `x`/`y`,
  - aggregate `x`/`y` compiles,
  - partition-key/literal placement compiles.

### Chunk 2: Extract placement-summary planning

Files:

- `avenger-chart/src/cartesian/positioned_subplot.rs`
- optionally a new small helper module under `avenger-chart/src/cartesian/`

Work:

- Add a partitioned path alongside the current row-wise path in
  `prepare_positioned_subplot`.
- Build a placement summary from the current parent dataframe:
  - group by the original partition expression,
  - compute each aggregate placement expression once per partition,
  - preserve literal placement expressions as scalar channels,
  - preserve partition-key placement expressions by projecting the grouped key.
- Reuse the channel-scale application path so summary `x`/`y` values are scaled
  exactly like ordinary positioned subplot coordinates.
- Produce child specs containing:
  - child index,
  - mark index,
  - stable partition value,
  - formatted label/key fallback,
  - scaled `x` and `y`,
  - per-partition data override.

Implementation note:

- The placement summary is a planning artifact. It should not replace the raw
  parent dataframe in the child-frame runtime.
- If there are no rows for a partitioned subplot, produce no children, matching
  the current no-mark-output behavior.

Validation:

- Unit tests for the helper with a small dataframe:
  - one child per partition value,
  - `avg`/`sum` placement values are correct before scaling,
  - filtered child data contains only rows for that partition,
  - deterministic partition ordering.

### Chunk 3: Measure and render partitioned children

Files:

- `avenger-chart/src/cartesian/positioned_subplot.rs`
- child-frame scope/path helpers only if stable partition identity needs a
  small extension

Work:

- Feed each partition's filtered dataframe into `ChildFrameRuntime::prepare_plot`
  as inherited parent data.
- Preserve existing child-frame domain sharing behavior by creating one
  `ChildFrameDomainSharingInput` per partition child.
- Update container path / sharing key construction so partitioned children are
  stable and distinguishable.
- Render from saved measurements exactly like current positioned subplots.

Validation:

- Focused chart tests that inspect scene groups or measurement state:
  - child group count equals partition count,
  - child groups are positioned at aggregate coordinates,
  - child data changes child mark output per partition.

### Chunk 4: Scale-domain inference for aggregate placement

Why this matters:

Parent Cartesian `x`/`y` scales are built before coordinate measurement. If
partitioned placement aggregation is deferred to measurement, scale inference
must still see the placement-summary values. Otherwise users would need to set
explicit parent domains for every partitioned subplot chart.

Preferred work:

- Reuse or extract the existing aggregate-channel planning logic from
  `Plot::compile_mark_with_aggregation`.
- Teach the scale-builder path to handle aggregate channel expressions when a
  mark's aggregate transformation is deferred to runtime.
- For partitioned Cartesian subplots, infer `x`/`y` domains from the placement
  summary, not from raw rows.

Pragmatic fallback if this chunk gets too large:

- Temporarily require explicit parent `x`/`y` domains for partitioned
  aggregate placement and return a clear error when scale inference cannot
  evaluate aggregate placement domains.
- This fallback should be short-lived. The intended user experience is inferred
  domains.

Validation:

- A test with no explicit parent `x`/`y` domains should infer domains from
  aggregate placement values.
- A test nested under an existing facet data override should infer from the
  current inherited dataframe rather than compile-time top-level data.

### Chunk 5: Visual dogfood

Add visual coverage in `avenger-chart/tests/visual_tests/test_cartesian_subplot.rs`.

Scenarios:

- Partitioned Cartesian mini bar charts:
  - parent `x = avg(...)`, `y = avg(...)`,
  - child plot is a small Cartesian bar chart using inherited partition data.
- Partitioned Cartesian mini scatter plots:
  - child plot shows only the partition rows.
- Optional, after Polar has the needed mark support:
  - partitioned mini Polar scatter/pie-like child plot.

No broadcast coverage in this chunk.

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

## Open Questions To Resolve During Implementation

- Should the public method be only `.partition_by(...)`, or should we also add
  a shorter `.partition(...)` alias later?
- Should child frame identity gain an explicit `PartitionedPositionedSubplot`
  variant, or is the current `PositionedSubplot { mark_index, row_index, key }`
  enough if `key` includes the partition value?
- Should partition values be labels by default, or remain only identity unless
  `.label(...)` is provided?
- Should `plot_width`/`plot_height` eventually accept partition-level
  expressions? The first step keeps them fixed.

## Suggested First Commit

Start with Chunk 1 plus validation tests. It gives us the public shape and
strict semantics without touching the measurement loop yet:

```text
feat(chart): add partition metadata for Cartesian subplots
```

