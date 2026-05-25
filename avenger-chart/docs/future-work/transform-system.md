# Transform System

## Goal Review

The goal is valid: chart authors need ergonomic data transformations such as
bin, stack, density, window, pivot, fold, and grouped summaries. The current
library already uses DataFusion heavily and supports aggregate expressions in
channels, but it does not have a first-class chart transform pipeline.

Some of the old proposal is already covered:

- `DataContext` and `CompiledDataContext` already store plot-level and
  mark-level data.
- `Plot::compile` detects aggregate expressions in mark channels and rewrites
  mark data for those aggregate encodings.
- Users can always transform a DataFusion `DataFrame` before passing it to
  `Plot::data` or mark `.data(...)`.

Those pieces do not replace a transform API. They are lower-level building
blocks.

## Current System Fit

The transform system should sit before scale-domain inference. That means
transforms must run before `build_scale_builder_from_marks` inspects channel
data. They should operate on `DataContext`, not on scenegraph output.

The cleanest current boundary is probably:

```rust
pub trait DataTransform: Send + Sync {
    fn transform(
        &self,
        data: DataFrame,
        context: &TransformContext,
    ) -> Result<TransformedData, AvengerChartError>;
}
```

`TransformedData` should include the output `DataFrame` plus channel rewrite
metadata. For example, `bin_x(col("value"))` might create `x0` and `x1`
columns and update a `Rect` mark to use them.

## Recommended Direction

Start with a narrow built-in transform layer in the facade or
`avenger-chart-marks`, then move stable contracts to core only after the shape
is proven. A good first slice is:

- `Bin` for Cartesian `Rect` histogram-style marks.
- `Stack` for Cartesian bars/areas once area/path marks exist.
- `Fold` or `PivotLonger` because it can power repeat-like use cases without
  a dedicated repeat container.

The transform should be serializable if it becomes part of plot specs.

## Alternate Paradigms

- **External DataFusion preprocessing**: simplest and already possible, but it
  does not provide chart-aware channel rewrites or portable specs.
- **Aggregate channel expressions only**: good for concise summaries, but too
  implicit for binning, stacking, and reshaping.
- **Mark-specific constructors** such as `Histogram::new()`: ergonomic for
  common cases, but can duplicate transform logic across marks.

## Readiness

Ready for an implementation plan for a narrow v1.

The plan should not attempt a complete grammar. It should choose one transform
that clearly runs before scale inference, define how output channel names are
created, and add tests proving scale domains use transformed data.

## Decisions Needed

- Whether transforms attach to `Plot`, to individual marks, or to
  `DataContext`.
- Whether transform contracts live in core immediately or start as built-ins.
- How transforms declare output columns and channel rewrites.
- How transform output is serialized.
- How transforms interact with faceting, positioned subplot partitioning, and
  mark-level data overrides.
