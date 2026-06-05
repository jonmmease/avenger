# Transform System

## Summary

Chart transforms are data-space operations that run before scale-domain
inference, aggregate channel evaluation, guide planning, and rendering. A
transform consumes source-data expressions, adds derived columns to the mark's
effective data table, and exposes typed output handles that mark encodings can
use through ordinary `ChannelValue`s.

Transforms are not scenegraph operations and should not introduce special mark
semantics. The first motivating transform is `Bin`; histogram-like charts should
be represented as ordinary `Rect` marks using bin output columns plus aggregate
channels.

Authoring should preserve a clear distinction between source data and generated
data. Source columns are referenced with ordinary DataFusion expressions such as
`col("value")`. Columns created by transforms should be referenced through the
transform output handle returned to the inline transform closure, not by spelling
generated column names with `col("...")`.

## Data Flow

Transforms attach to the mark/data context. At evaluation time, the mark's
effective input data is resolved first, including plot-level data inheritance,
mark-level data overrides, facet filtering, positioned subplot partitioning, and
store data. The transform pipeline then produces an augmented table that is used
by the normal channel, scale, guide, and rendering paths.

The pipeline should preserve this order:

1. Resolve effective mark data.
2. Apply mark/data-context transforms.
3. Evaluate aggregate channel plans over transformed data.
4. Build scale-domain data from transformed channels.
5. Render ordinary marks from transformed and aggregated data.

This keeps transforms reusable across marks while still allowing scale domains,
axis ticks, legends, selections, and event datum rows to observe transformed
columns consistently.

## Bin Transform

`Bin` consumes a numeric source expression and produces additional columns for
the bin interval. Output names are derived from the pretty form of the source
expression by default, with an explicit name override for collisions or clarity.

For example, `Bin::new(col("value"))` may produce:

- `value_bin_start`
- `value_bin_end`
- `value_bin_index`

The intended authoring shape keeps the transform inline while making the
derived columns explicit through a typed output handle:

```rust
Rect::new()
    .transform(Bin::new(col("value")).maxbins(30), |mark, bin| {
        mark.x(bin.start())
            .x2(bin.end())
            .y(lit(0.0))
            .y2(count())
    })
```

`bin.start()` and `bin.end()` return `ChannelValue`s, not raw DataFusion
expressions. Internally, they reference the generated columns, such as
`col("value_bin_start")` and `col("value_bin_end")`. Because they are
`ChannelValue`s, they can carry ordinary scale configuration defaults while
still being accepted by existing mark channel methods.

Histograms do not require a dedicated histogram mark. The `Rect` mark consumes
the bin start/end columns for x/x2 and uses existing aggregate channel support,
such as `count()`, for the bar height.

## Output Handles

Transform output handles are the public access point for generated columns.
They should expose methods named by the role of the output in the transform, not
by dynamically generated Rust method names.

Fixed structural outputs should use stable methods:

```rust
bin.start()
bin.end()
bin.index()

stack.start()
stack.end()
stack.mid()
```

Transforms with user-named outputs should expose lookup methods:

```rust
join.output("total_value")
window.output("rank")
calculate.output("residual")
```

This keeps generated column names internal to the transform while still allowing
authors to reference every output. Output handle methods may return `Expr` or
`ChannelValue` depending on the output's role. Plain derived values can return
`Expr`; positional interval outputs that need scale defaults can return
`ChannelValue`.

## Other Transform Fits

The same model applies to other Vega-Lite-style transforms:

- `JoinAggregate` is row-preserving. It computes grouped aggregate values and
  joins them back onto each input row. Its output handle exposes user-named
  aggregate columns such as `join.output("group_total")`.
- `Window` is row-preserving and order-sensitive. It computes values such as
  ranks, lag/lead values, running sums, or frame aggregates. Its output handle
  exposes named window results.
- `Stack` creates interval columns such as start, end, and midpoint. A stacked
  bar or area remains an ordinary `Rect` or `Area` mark that references
  `stack.start()` and `stack.end()`.
- `Calculate`, `Filter`, `Fold`, and `Pivot` can use the same table-transform
  pipeline, even when their output shape is not row-preserving.

Explicit transforms form a pipeline over effective mark data. Some transforms,
such as `Stack`, may perform aggregation internally before producing derived
columns. Remaining implicit aggregate channel planning should run after the
explicit transform pipeline.

## Scale And Axis Behavior

The bin transform resolves ordinary values that can be consumed by scale config:

- bin extent start
- bin extent end
- bin step

The binned position channel should configure its scale with ordinary scale
options:

- domain interval set to the resolved bin extent;
- `nice(false)` and `zero(false)` defaults for the binned axis;
- tick values generated from a general start/step tick option.

The scale API should add a reusable start/step tick feature rather than
bin-specific methods. The design should avoid APIs such as
`domain_from_bin_extent` or `ticks_from_bin_edges`; binning should produce
values, and scale configuration should consume those values through general
domain and tick options.

For `Rect`, `x2` and `y2` already resolve to the same scale names as `x` and
`y` by default, so the scale does not need special knowledge that the start and
end channels are paired. The configured domain and tick behavior on the primary
binned channel is sufficient for Vega-like histogram axes that use bin extents
and bin edges.

## Facets And Sharing

Resolved bin parameters follow the sharing scope of the transform stage that
produces the binned channel:

- `Sharing::Free` computes bins independently for each cell.
- `Sharing::Level(n)` computes bins at the matching ancestor sharing scope.
- `Sharing::Shared` computes one bin plan for the shared plot scope.

This prevents shared histograms from producing different bin steps or extents in
linked cells. Explicit bin step or extent options remain deterministic at every
scope; inferred options such as `maxbins` require raw-row extent calculation at
the owning sharing scope.

## Implementation Direction

The transform contract should be serializable because transform specs become
part of compiled plot specs. The stable contract belongs in
`avenger-chart-core` near `DataContext`, so custom marks and external transform
crates can participate without depending on built-in transforms.

Built-in transform implementations should live in a peer crate,
`avenger-chart-transforms`, analogous to built-in scales, legends, marks, and
tools. The chart facade can re-export these built-ins and link their typetag
registrations by default, but third-party transform crates should depend only
on `avenger-chart-core` plus lower-level runtime crates they need.

The intended contract is a typetag-serialized compiled transform object.
Authoring transforms return a boxed `CompiledDataTransform` plus a typed output
handle. `CompiledPlot` stores those boxed compiled transforms directly, and
evaluation calls each transform object in order. This matches the compiled
extension model used for marks, coordinate transforms, axes, scale specs, and
subplot child plots. A compiled plot containing an external transform must be
deserialized by a binary that links the external transform crate.

A narrow v1 should implement:

- mark/data-context transform attachment;
- core transform builder and compiled-transform traits;
- a new `avenger-chart-transforms` crate for built-in transforms;
- transform output column declaration and default name derivation;
- inline transform closure API returning typed output handles;
- output-handle access for generated columns instead of author-facing
  `col("generated_name")` references;
- `Bin` with explicit step/extent and `max_bins`;
- scale domain inference from transformed data;
- general start/step scale ticks;
- focused visual and unit coverage for `Rect + Bin + count()` histograms.
