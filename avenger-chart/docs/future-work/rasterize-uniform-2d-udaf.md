# RasterizeUniform2D UDAF Implementation Plan

## Status

Reviewed implementation design. This note covers the implementation model for
`RasterizeUniform2D`, a non-materializing transform that uses DataFusion's
aggregate machinery to bin source rows directly into uniform raster structs.

The public authoring API is sketched in
`scratch/rasterize-uniform-2d-public-api.md`. This document summarizes the
current uniform raster struct shape used by the implementation plan; older
multi-raster explorations in
[raster-arrow-representations.md](raster-arrow-representations.md) may predate
the DataArray-style `geometry.dimensions` and `values.dims` form.

## Goal

Implement `RasterizeUniform2D` as a public Avenger data transform whose row-heavy
work stays inside DataFusion. The transform should not materialize long-form
`(bin_x, bin_y, value)` rows in Rust. Instead, each DataFusion aggregate
partition should allocate dense cell grids, update them from input rows, and
merge partial grids with cell-wise operations.

The output remains ordinary dataframe rows:

```text
group columns... | raster
-----------------|-------------------------
A                | Struct<geometry, values>
B                | Struct<geometry, values>
```

Each `raster` value uses the existing `UniformRaster2D` raster struct schema.

## Reviewed Design

The design fits together if we separate three layers:

```text
Transform config:
  concrete extents, bins, aggregation mode, dimension names

UDAF intermediate state:
  dense mergeable value grids only

UDAF final output:
  full public UniformRaster2D struct
```

The UDAF should not use the full raster struct as its merge state. Geometry is
invariant for all partial states in one configured aggregate, so carrying
`geometry.kind`, dimension starts/stops, dimension names, `values.dims`, and
sampling through every partial state would duplicate data and create unnecessary
consistency checks. The merge state should contain only the dense cell state
that actually changes across partitions.

The UDAF should still return the full raster struct from `evaluate`. That keeps
Avenger's internal and public representation aligned: a raster produced by
`RasterizeUniform2D` looks the same as a precomputed raster column consumed by
`UniformRaster2D`.

## Core Decisions

- `RasterizeUniform2D` is a normal `DataTransform`.
- V1 supports `count` and `sum`.
- Future aggregation modes should be added in phases. Some can share a generic
  dense-grid UDAF with different cell merge/finalize operations; others need
  dedicated UDAFs because their state has multiple lanes, variable-sized
  sketches, or multiple input value expressions.
- The transform resolves scalar controls before building the aggregate plan:
  bins, explicit extents, and `.agg(param(...))`.
- Inferred extents use a small scalar prepass over the input plan. This may
  collect the min/max result row, but must not collect source rows or grouped
  bin rows.
- The configured UDAF is created inside `CompiledRasterizeUniform2D::apply`.
  It does not need to be registered as a global SQL function in v1.
- The UDAF owns coordinate binning and per-cell aggregation.
- The UDAF final result is the full uniform raster struct.
- The UDAF intermediate state is values-only dense grid data.
- Final-bin closure is required: values exactly on the stop edge are included
  in the last bin.
- The v1 transform only targets uniform rasters with `"linear"` sampling.
- The v1 transform does not need View integration.
- Direct-color no-scale rasters are supported by `UniformRaster2D` for
  precomputed raster structs. `RasterizeUniform2D` v1 outputs numeric aggregate
  rasters intended for scaled fill channels.

## Public Shape

The public API should match the scratch API sketch:

```rust
UniformRaster2D::new().transform(
    RasterizeUniform2D::new(col("x"), col("y"))
        .x(|x| x.bins(256).extent(0.0, 100.0))
        .y(|y| y.bins(256).extent(0.0, 50.0))
        .agg("count"),
    |mark, hist| {
        mark.raster_with(hist.raster(), |r| {
            r.x(dim("x")).y(dim("y"))
        })
    },
)
```

For value aggregation:

```rust
UniformRaster2D::new().transform(
    RasterizeUniform2D::new(col("x"), col("y"))
        .value(col("weight"))
        .x(|x| x.bins(param("plot_width")).extent(param("x0"), param("x1")))
        .y(|y| y.bins(param("plot_height")).extent(param("y0"), param("y1")))
        .agg(param("raster_agg")),
    |mark, hist| {
        mark.raster_with(hist.raster(), |r| {
            r.x(dim("x"))
             .y(dim("y"))
             .fill(|fill| fill.legend(|legend| legend.title("Density")))
        })
    },
)
```

`partition_by(...)` outputs one raster row per group and preserves grouping
columns for faceting, styling, filtering, ordering, and hover data.

## Transform Plan Shape

With explicit scalar extents:

```text
apply:
  resolve x bins, y bins, x extent, y extent, agg mode
  build configured RasterizeUniform2DUdf

input
  -> project x, y, optional value, partition/group expressions
  -> aggregate by partition/group expressions
       rasterize_uniform_2d(x, y[, value]) AS raster
  -> output partition/group columns plus raster
```

With inferred extents:

```text
extent_row =
  input.aggregate(
    [],
    [
      min(x) AS x_start,
      max(x) AS x_stop,
      min(y) AS y_start,
      max(y) AS y_stop,
    ],
  ).collect_one_row()

apply:
  validate inferred extents
  build configured RasterizeUniform2DUdf with those extents
  build the same aggregate plan as the explicit extent case
```

This scalar prepass is acceptable because it does not materialize source rows or
long-form bin rows. It also keeps the UDAF configuration concrete, so the hot
loop does not need to read repeated extent arguments from every batch.

Default inferred extents should be global across the transform input so
partitioned rasters share geometry. Per-partition extents are plausible later,
but they make overlays and comparisons harder because each output raster has a
different grid.

## UDAF Shape

Use `AggregateUDFImpl`, not the simple `create_udaf` helper. The implementation
needs a custom struct return type, custom intermediate state fields, and a
specialized `GroupsAccumulator`.

Conceptual configured call:

```text
rasterize_uniform_2d(x, y) -> UniformRaster2DStruct
rasterize_uniform_2d(x, y, value) -> UniformRaster2DStruct
```

The UDAF object should be configured by the transform with:

```text
x_bins: u32
y_bins: u32
x_start: f64
x_stop: f64
y_start: f64
y_stop: f64
aggregate_op: CountRows | CountValues | SumValues
output_value_type: UInt64 for count, Float64 for sum
x_dim_name: String
y_dim_name: String
x_sampling: "linear"
y_sampling: "linear"
```

`CountRows` receives x/y only. `CountValues` receives x/y/value and counts
non-null value rows after coordinate filtering. `SumValues` receives x/y/value
and sums valid numeric values after coordinate filtering.

The return type is the current uniform raster struct:

```text
Struct<
  geometry: Struct<
    kind: Utf8,
    dimensions: List<Struct<
      name: Utf8,
      coords: Struct<
        kind: Utf8, // "uniform"
        sampling: Utf8?,
        start: Float64,
        stop: Float64,
        count: UInt32,
      >,
    >>,
  >,
  values: Struct<
    dims: List<Utf8>,
    data: List<TAggregate>,
  >,
>
```

V1 should emit `geometry.kind = "grid"`, two uniform dimensions named from the
DataFusion x/y expression display names, and `values.dims = [y_dim_name,
x_dim_name]` so the data list is row-major y/x storage. The mark still binds the
plot axes explicitly with `r.x(dim(...)).y(dim(...))`.

Do not add value-domain extrema, `null_count`, or `non_finite_count` fields to
the transform output. Downstream domain inference should inspect `values.data`
directly through the generic ListArray numeric path.

## Binning Semantics

Binning uses the configured start and stop edges:

```text
t = (coord - start) / (stop - start)
raw_index = floor(t * count)
index = clamp(raw_index, 0, count - 1)
```

Rows outside the closed interval are ignored:

```text
coord < min(start, stop) -> skip
coord > max(start, stop) -> skip
```

Rows exactly equal to `stop` are included:

```text
coord == stop -> raw_index == count -> index == count - 1
```

This matches Datashader's final-bin closure behavior and prevents maximum input
values from being dropped.

Validation:

```text
x_bins > 0
y_bins > 0
x_start != x_stop
y_start != y_stop
x_bins * y_bins fits the selected Arrow list representation
```

For v1, use `List<T>` and reject rasters whose cell count exceeds the Arrow
list offset limit. A later `LargeList<T>` option can relax this.

## Null And Non-Finite Rules

The UDAF skips rows with invalid coordinates:

```text
x null      -> skip row
y null      -> skip row
x nonfinite -> skip row
y nonfinite -> skip row
outside extent -> skip row
```

`CountRows` counts every remaining row.

`CountValues` follows SQL/DataFusion count-expression intuition: it counts
non-null value rows. A floating `NaN` value is not null, so it is counted.

`SumValues` requires a numeric value expression. Null and non-finite numeric
values are skipped so one `NaN` does not poison the cell aggregate.

## Empty Cell Semantics

The accumulator must distinguish untouched cells from cells whose aggregate
value happens to be zero.

V1 output:

```text
count empty cell -> 0
sum empty cell   -> null
```

This matches the mathematical expectation for count while preserving "no
observation" for value aggregations. The sum accumulator should still track
occupancy internally so final output validity can be built without scanning for
sentinel values.

## Accumulator State

The UDAF must implement both DataFusion accumulator paths:

- `Accumulator` for a single aggregate group.
- `GroupsAccumulator` for efficient grouped aggregation.

DataFusion can use the scalar `Accumulator` even when a `GroupsAccumulator` is
available, for example in no-group-by plans. Both paths must be correct.

Scalar accumulator state:

```text
CountRows / CountValues:
  counts: Vec<u64> // length = x_bins * y_bins

SumValues:
  sums: Vec<f64>      // length = x_bins * y_bins
  occupied: Vec<bool> // length = x_bins * y_bins
```

Grouped accumulator state should be flattened by group index:

```text
cell_count = x_bins * y_bins
offset(group_index) = group_index * cell_count

CountRows / CountValues:
  counts: Vec<u64> // length = total_num_groups * cell_count

SumValues:
  sums: Vec<f64>
  occupied: Vec<bool>
```

`GroupsAccumulator::update_batch` must resize state when `total_num_groups`
increases. `evaluate` and `state` must honor `EmitTo::All` and
`EmitTo::First`, releasing emitted group state and retaining shifted later
groups.

`size()` must report dense state memory accurately enough for DataFusion memory
accounting:

```text
groups * x_bins * y_bins * state_lanes * bytes_per_lane
```

The transform should also validate `x_bins * y_bins` against a practical maximum
before accumulator allocation.

## Intermediate State Fields

`state_fields` should describe mergeable dense state, not the final raster
struct.

V1 state fields:

```text
CountRows / CountValues:
  counts: List<UInt64>

SumValues:
  sums: List<Float64>
  occupied: List<Boolean>
```

`merge_batch` combines partial states cell by cell:

```text
count: sum counts
sum:   sum partial sums where occupied; occupied = occupied_a OR occupied_b
```

`Accumulator::state` wraps one group's state as `ScalarValue::List(...)`.
`Accumulator::evaluate` wraps the final raster as `ScalarValue::Struct(...)`.
These are one-row Arrow arrays under DataFusion's scalar wrapper.

`GroupsAccumulator::state` returns `ArrayRef` list arrays with one row per
group. `GroupsAccumulator::evaluate` returns a `StructArray` with one raster row
per group. It should not allocate one `ScalarValue` per group.

## Future Aggregation Phases

Datashader is useful inspiration because its reducers are explicitly split into
create, append, combine, and finalize phases. Relevant source:

- [datashader/reductions.py](https://github.com/holoviz/datashader/blob/main/datashader/reductions.py)
- [datashader/utils.py](https://github.com/holoviz/datashader/blob/main/datashader/utils.py)

Do not treat those implementations as normative for Avenger. For every added
aggregation, define the state algebra independently, validate merge correctness,
and choose a formulation that is stable under DataFusion's partial/final
aggregation plans.

### Phase 2: Single-Lane Merge Reducers

Add reducers whose state is one value lane plus an occupancy/null lane. `min`
and `max` can share one generic "ordered extreme" UDAF with a direction flag.

```text
state:
  values: Vec<T>
  occupied: Vec<bool>

update_min(cell, x):
  if valid(x) and (!occupied[cell] or x < values[cell]):
    values[cell] = x
    occupied[cell] = true

merge_min(a, b):
  if !b.occupied -> a
  if !a.occupied or b.value < a.value -> b
  else -> a

update_max / merge_max use `>` instead of `<`.
```

Datashader's `min` and `max` use NaN-aware min/max over partial grids. Avenger
should use Arrow validity/occupancy rather than NaN sentinels so empty cells are
unambiguous and non-finite policy remains explicit.

`any` is also single-lane, but it produces boolean cell values. Defer unless a
clear raster mark use case appears, because numeric color scaling and
ListArray-domain inference are the main path for this transform.

Required validation:

- [ ] Property test: single-pass `min/max` equals randomized partial-state
      merges.
- [ ] Unit test: empty cells remain null.
- [ ] Unit test: null/non-finite input values are skipped consistently with
      `sum`.
- [ ] Unit test: `min` and `max` share the same implementation with only the
      comparison direction changed.

### Phase 3: Mean

Datashader computes mean from two base grids:

```text
sum_zero = sum of valid values, initialized to 0
count = count of valid values
mean = sum_zero / count where count > 0, else missing
```

Avenger should implement `mean` as a dedicated UDAF with two merge lanes:

```text
state:
  sums: Vec<f64>
  counts: Vec<u64>

update(cell, x):
  if valid(x):
    sums[cell] += x
    counts[cell] += 1

merge(a, b):
  sums = a.sums + b.sums
  counts = a.counts + b.counts

finalize(cell):
  if counts[cell] > 0:
    sums[cell] / counts[cell]
  else:
    null
```

This is simple, parallel, and sufficient for ordinary charting. If numerical
stability becomes a concern for very large magnitudes, revisit a compensated
sum lane before exposing additional precision guarantees.

Required validation:

- [ ] Property test: randomized partial merges match a direct per-cell
      reference calculation within a floating tolerance.
- [ ] Unit test: empty cells are null.
- [ ] Unit test: null/non-finite values are skipped.
- [ ] Unit test: merged counts are used for final validity, not sum value.

### Phase 4: Variance And Standard Deviation

Datashader uses `sum`, `count`, and an `m2` grid. Its incremental update is
equivalent to Welford's running second-moment update:

```text
old_mean = sum / count
new_mean = (sum + x) / (count + 1)
m2 += (x - old_mean) * (x - new_mean)
```

Its partial-state combine computes a global mean and applies a correction:

```text
mu = sum(partial_sums) / sum(partial_counts)
m2_total = sum(partial_m2 + partial_count * (partial_mean - mu)^2)
variance_population = m2_total / total_count
std_population = sqrt(variance_population)
```

For Avenger, prefer Chan/Welford state because it avoids storing both `sum` and
`mean` and has a standard associative merge:

```text
state:
  counts: Vec<u64>
  means: Vec<f64>
  m2s: Vec<f64>

update(cell, x):
  n1 = counts[cell]
  n2 = n1 + 1
  delta = x - means[cell]
  means[cell] += delta / n2
  delta2 = x - means[cell]
  m2s[cell] += delta * delta2
  counts[cell] = n2

merge(a, b):
  if a.count == 0 -> b
  if b.count == 0 -> a
  n = a.count + b.count
  delta = b.mean - a.mean
  mean = a.mean + delta * b.count / n
  m2 = a.m2 + b.m2 + delta * delta * a.count * b.count / n
  count = n

finalize_var_population(cell):
  if count > 0:
    m2 / count
  else:
    null

finalize_std_population(cell):
  sqrt(var_population)
```

Name the operation clearly as population variance/std in the first version.
Sample variance can be a separate mode later:

```text
var_sample = m2 / (count - 1), valid only when count > 1
```

Required validation:

- [ ] Property test: randomized partitioning and merge order produce stable
      results within tolerance.
- [ ] Unit test: one-value population variance is `0`.
- [ ] Unit test: one-value sample variance, if added, is null.
- [ ] Reference test: compare against a direct per-cell vector calculation.
- [ ] Stress test: large offset values with small variance do not suffer
      unacceptable cancellation.

### Phase 5: Selector Reducers

Datashader's `first`, `last`, and `where(selector, lookup_column)` become
row-index selector reductions under partitioned execution. Avenger should not
depend on implicit DataFusion row order. Any order-dependent reducer must
require an explicit ordering expression.

Useful Avenger reducer families:

```text
argmin(selector, value)
argmax(selector, value)
first_by(order, value)
last_by(order, value)
```

State for `argmin`/`argmax`:

```text
selector_values: Vec<TSelector>
lookup_values: Vec<TLookup>
occupied: Vec<bool>

update_argmax(cell, selector, lookup):
  if valid(selector, lookup) and (!occupied or selector > selector_values[cell]):
    selector_values[cell] = selector
    lookup_values[cell] = lookup
    occupied[cell] = true

merge_argmax(a, b):
  choose the occupied state with the larger selector
```

Tie behavior must be specified before implementation. Prefer deterministic
ties using a secondary explicit order expression, or document first-partial
winner semantics only if DataFusion makes that deterministic enough for tests.

Required validation:

- [ ] Reject implicit `first`/`last` without an explicit order expression.
- [ ] Property test: randomized partial merges match direct argmin/argmax.
- [ ] Unit test: ties follow the documented rule.
- [ ] Unit test: lookup nulls and selector nulls are handled explicitly.

### Phase 6: Top-K Reducers

Datashader supports `min_n`, `max_n`, `first_n`, and `last_n` by storing `n`
values per cell and merging sorted per-cell arrays.

These should not be added to `RasterizeUniform2D` until the raster value schema
supports non-scalar cell payloads or multiple output rasters. The current
`UniformRaster2D` mark expects a flat scalar `values.data` plane, so a top-k
cell is not directly renderable as a single color-scaled raster value.

Potential future representations:

- output `n` raster columns, one per rank,
- output a dataset-style raster with named value planes,
- output `values.data: List<FixedSizeList<T>>`, then add mark support for
  selecting one rank as fill.

Required validation before implementation:

- [ ] Decide the Arrow representation for per-cell top-k values.
- [ ] Define merge as sorted-list merge plus truncate to `k`.
- [ ] Define ordering, null, non-finite, and tie behavior.
- [ ] Add property tests against direct per-cell sorted vectors.

### Phase 7: Categorical And Summary Reducers

Datashader's `by(category, reduction)` / `count_cat` add a category dimension to
the aggregate, and `summary(...)` computes multiple named reductions in one
pass.

For Avenger, `partition_by(category)` already covers the simplest categorical
case by producing one raster row per category. True categorical planes should
wait for a dataset-style raster representation or multiple value planes.

`summary(...)` is attractive for efficiency because one row pass can update
multiple aggregate states. DataFusion can already evaluate multiple aggregate
expressions in one aggregate operator, but each UDAF still owns its own state
and update logic. A future combined raster UDAF could share coordinate binning
and update multiple grids, then return a struct/dataset with several raster
value planes.

Required validation before implementation:

- [ ] Decide whether multi-stat output is multiple raster columns or one
      dataset-style raster struct.
- [ ] Ensure value-plane names are visible to mark configuration.
- [ ] Benchmark separate UDAFs versus a combined multi-stat UDAF.
- [ ] Define categorical cardinality limits and output ordering.

### Deferred Or Out Of Scope

`mode` is intentionally hard for point rasterization because exact mode needs
unbounded per-cell storage unless the value domain is small and known. Treat it
as future work for categorical rasters or implement an approximate/sketch-based
mode only after a concrete use case appears.

Antialiased line/polygon aggregation behavior in Datashader is not relevant to
the first `RasterizeUniform2D` point workflow. If Avenger later rasterizes lines
or polygons directly, antialiasing should be designed as a separate geometry
rasterization feature rather than mixed into this transform.

## Arrow Builders

Add shared helpers so the UDAF boundary handles Arrow wrapping while the hot
loop works with plain Rust buffers:

```rust
struct UniformRaster2DSpec {
    dimensions: Vec<UniformRasterDimensionSpec>,
    values_dims: Vec<String>,
}

struct UniformRasterDimensionSpec {
    name: String,
    sampling: Option<String>, // Some("linear") for transform output
    start: f64,
    stop: f64,
    count: u32,
}
```

Suggested helper responsibilities:

```text
values_state_scalar(values) -> ScalarValue::List(...)
values_state_array(groups) -> ArrayRef<List<T>>
raster_scalar(spec, values, validity) -> ScalarValue::Struct(...)
raster_array(spec, groups) -> ArrayRef<Struct<geometry, values>>
```

These helpers should build the same schema consumed by `UniformRaster2D` and
should be reused by scalar and grouped accumulator paths.

## Interaction With Faceting And Transform Scope

`RasterizeUniform2D` should follow existing transform scope rules. The
transform's explicit `partition_by` expressions are user-visible grouping
columns.

The implementation also needs to account for `DataTransformFacetContext` when a
transform runs at a sharing level above the final mark. It must not collapse
data that should remain separate for downstream facet cells. Follow the KDE
transform pattern: add required `facet_context.partition_exprs` to the effective
grouping set when they are not already present.

For v1, facet partition expressions may be limited to simple column references
if that keeps aliasing and output-column preservation tractable.

## Domain Behavior

The output raster struct contains enough geometry and value data for ordinary
raster mark domain inference:

```text
x domain    <- geometry.dimensions[name = x_dim].coords.start/stop
y domain    <- geometry.dimensions[name = y_dim].coords.start/stop
fill domain <- values.data
```

For inferred global extents, all partitioned output rasters should share the
same x/y geometry. For explicit extents, the output geometry should exactly
match the supplied extents, including orientation.

The transform itself should not add separate derived domain scalars in v1. The
raster mark consumes geometry fields and value lists directly.

## Performance Notes

This design mirrors the useful part of Datashader's architecture:

```text
create fixed aggregate grids per partition
append rows into cells
merge partial grids with cheap cell-wise operations
finalize one raster per group
```

The critical performance property is that no long-form bin table is produced.
The only shuffled intermediate payload is one dense grid per aggregate group per
partial partition.

Expected costs:

```text
row scan: O(input_rows)
merge:    O(groups * x_bins * y_bins * partial_partitions)
memory:   O(groups * x_bins * y_bins * state_width) per aggregate partition
```

The transform should add tracing spans around:

- scalar-control resolution,
- inferred extent prepass,
- aggregate plan construction,
- UDAF accumulator allocation,
- UDAF state merge,
- final raster struct construction.

## Visual Baseline Plan

Add a new visual test module:

```text
avenger-chart/tests/visual_tests/test_rasterize_uniform_2d.rs
```

Use a new baseline category:

```text
transform_rasterize_uniform_2d
```

Baseline files should be generated for the normal WGPU PNG baseline suite and
for SVG/PDF sidecars when refreshing sidecar baselines:

```text
avenger-chart/tests/baselines/transform_rasterize_uniform_2d/*.png
avenger-chart/tests/baselines_svg/transform_rasterize_uniform_2d/*.svg
avenger-chart/tests/baselines_svg/transform_rasterize_uniform_2d/*.png
avenger-chart/tests/baselines_pdf/transform_rasterize_uniform_2d/*.pdf
avenger-chart/tests/baselines_pdf/transform_rasterize_uniform_2d/*.png
```

Planned baselines:

- [ ] `count_explicit_extent_edges`
      Synthetic point data, explicit x/y extents, small bins such as `5 x 4`,
      and points placed at start edges, stop edges, repeated cells, and just
      outside the extent. Render with `UniformRaster2D::new().transform(...)`
      and `smooth(false)`. This is the primary visual lock for orientation,
      row-major output, final-bin closure, and count empty-cell zero behavior.
- [ ] `sum_with_null_empty_cells`
      Synthetic point data with `.value(col("weight")).agg("sum")`. Include
      null values, non-finite values, repeated cells, and intentionally empty
      cells. Configure a visible `null_color` and explicit fill domain so empty
      sum cells are visually distinct from zero-valued sum cells. This locks
      down `SumValues`, Arrow null output for empty cells, and the raster mark's
      null-cell rendering path with transform-produced rasters.
- [ ] `cars_density_by_origin`
      Recreate the existing precomputed
      `uniform_raster_2d/dataset_cars_density_by_origin` story using
      `RasterizeUniform2D` instead of hand-built raster rows:
      `x = Weight_in_lbs`, `y = Miles_per_Gallon`, `agg = count`,
      explicit extents, `48 x 32` bins, and `partition_by([col("Origin")])`.
      Render through `FacetColumn` with a sqrt fill scale and axis titles. This
      is the main dataset baseline and should look very similar to the existing
      manual raster baseline while proving the transform-to-mark path.
- [ ] `cars_horsepower_sum`
      Use the cars dataset with
      `value(col("Horsepower")).agg("sum")`, explicit extents, and one
      unfaceted plot or the same origin facets if the first implementation
      needs more partition coverage. This gives a realistic `sum` baseline that
      differs visually from count density.
- [ ] `count_param_extent_and_bins`
      Small synthetic dataset where bins and extents are supplied by params.
      Evaluate with a fixed param set in the visual test. This baseline is less
      visually unique than the first two, but it catches the integration between
      param evaluation, configured UDAF creation, and final raster rendering.

Avoid adding baselines for every edge case. Use core/unit tests for exact cell
values, invalid inputs, `GroupsAccumulator` behavior, and `EmitTo::First`.
Visual baselines should focus on end-to-end transform plus raster rendering.

## Required Test Updates

- [ ] Transform unit test: explicit extent with points at both start and stop
      edges includes both extremes.
- [ ] Transform unit test: exact stop edge maps to the last bin, not an
      out-of-range bin.
- [ ] Transform unit test: out-of-range values just outside the closed extent
      are skipped.
- [ ] Transform unit test: inferred extents produce expected geometry
      start/stop/count fields.
- [ ] Transform unit test: output raster schema includes `geometry` and
      `values.data`, and does not include value extrema or diagnostic count
      fields.
- [ ] Transform unit test: reversed explicit extents preserve orientation and
      still bin correctly.
- [ ] Transform unit test: row-major cell order is stable.
- [ ] Transform unit test: `count` creates one raster row with `UInt64` values.
- [ ] Transform unit test: `count` empty cells are zero.
- [ ] Transform unit test: `count` with `value(...)` counts non-null value
      rows.
- [ ] Transform unit test: `value(None)` and literal `Null` behave like omitted
      value for `count`.
- [ ] Transform unit test: `sum` creates one raster row with `Float64` values.
- [ ] Transform unit test: `sum` empty cells are Arrow null.
- [ ] Transform unit test: `sum` skips null and non-finite values.
- [ ] Transform unit test: partitioned rasterization returns one row per
      partition and preserves partition columns.
- [ ] Transform unit test: facet context partition expressions are included in
      effective grouping when needed.
- [ ] Transform unit test: null x/y rows are skipped.
- [ ] Transform unit test: non-finite x/y rows are skipped.
- [ ] Transform unit test: all-empty `sum` raster has all-null `values.data`
      and no fill/color domain contribution through ListArray inference.
- [ ] UDAF unit test: scalar `Accumulator` and `GroupsAccumulator` produce the
      same output for the same input.
- [ ] UDAF unit test: partial state merge matches single-pass aggregation.
- [ ] UDAF unit test: `GroupsAccumulator` handles increasing
      `total_num_groups`.
- [ ] UDAF unit test: `EmitTo::First` releases emitted group state and keeps
      later groups correct.
- [ ] Schema test: output field is accepted by uniform raster schema
      validation.
- [ ] Error test: zero x or y bins are rejected.
- [ ] Error test: degenerate x or y extent is rejected.
- [ ] Error test: non-numeric value expression is rejected for `sum`.
- [ ] Error test: oversized raster dimensions are rejected before accumulator
      allocation.

## Implementation Checklist

- [ ] Add `RasterizeUniform2D` authoring builder and output handle.
- [ ] Add serializable `CompiledRasterizeUniform2D` transform spec.
- [ ] Add builder options for x/y axis builders, optional value, aggregate op,
      output name, explicit extents, and partition/group expressions.
- [ ] Add validation for generated output names and duplicate partition/output
      names.
- [ ] Resolve scalar controls with existing param evaluation helpers.
- [ ] Add inferred-extent scalar prepass.
- [ ] Build configured `AggregateUDFImpl` instance inside transform `apply`.
- [ ] Implement DataFusion return type and state field definitions.
- [ ] Implement scalar `Accumulator`.
- [ ] Implement `GroupsAccumulator`.
- [ ] Implement count state fields and merge logic.
- [ ] Implement sum state fields and merge logic.
- [ ] Add dense Arrow list/struct builders for uniform raster output.
- [ ] Add shared helpers for finite numeric checks and cell index computation.
- [ ] Add effective partition handling, including facet-context columns.
- [ ] Add transform tests listed above.
- [ ] Add public re-exports in `avenger-chart/src/prelude.rs` when the public
      transform type lands.

## Future Work

- Implement future aggregation phases from the roadmap above, starting with
  `min`/`max`, then `mean`, then variance/std.
- Support `LargeList<T>` for very large rasters.
- Support per-partition inferred extents when intentionally requested.
- Add View integration by supplying explicit view extents and pixel dimensions.
- Add geo-specific rasterization as a separate transform rather than overloading
  the local Cartesian v1.
- Consider configurable empty-cell policy for value aggregates after concrete
  use cases appear.

## Open Questions

- What max cell count should the transform enforce by default?
- Should `sum` use DataFusion's exact sum coercion rules in v1, or is `Float64`
  output the right first implementation for predictable raster scaling?
- Should `CountValues` count floating `NaN` values for strict SQL consistency,
  or should rasterization treat all non-finite values as invalid regardless of
  aggregation mode?
