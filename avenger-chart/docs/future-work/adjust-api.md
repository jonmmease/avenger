# Adjust API

## Goal Review

The goal is still valid: chart authors need post-scale placement adjustments
such as jitter, dodge, collision avoidance, and label nudging. These operations
act after data values have been scaled into plot-area coordinates, so they do
not belong in ordinary DataFusion input transforms.

One possible design is for every mark to hand a DataFrame of scaled positions
and bounding boxes to a generic `Adjust` trait. That shape does not match the
current runtime yet. The current renderer prepares mark data through
`prepare_mark_data_runtime`, then each `CompiledMark::render_from_data`
produces scenegraph marks with mark-specific geometry. There is no shared
post-scale table containing mark bounds.

## Current System Fit

Useful existing pieces:

- `MarkRuntimeContext` and `CoordinateSystemTransformCore` give marks access
  to configured scales and coordinate transforms.
- `CompiledMarkCore::default_channel_range` and coordinate
  `ScaleRangeBinding` already separate data domain from visual range.
- `EvaluatedPlot` can include a `SceneGraphRTree`, which can support overlap
  and hit-test queries after rendering.
- Debug/layout paths already create scenegraph overlays, proving that
  evaluation can add geometry outside ordinary data marks.

The missing piece is an adjustment boundary between scaled channel evaluation
and final scenegraph mark creation.

## Recommended Direction

Design adjustments as mark-runtime modifiers, not as generic plot-level
DataFrame transforms. A first version should target marks whose geometry can be
represented as point anchors plus optional extents:

- `Jitter`: deterministic offsets on point anchors.
- `Dodge`: grouped offsets along one coordinate channel.
- `Nudge`: explicit pixel offsets for labels and annotations.

The compiled mark should decide whether an adjustment is supported. A generic
`Adjust` trait can operate over a shared `MarkGeometryTable` only after that
table exists.

Possible boundary:

```rust
pub trait MarkAdjustment: Send + Sync {
    fn adjust_points(
        &self,
        points: &mut PointAdjustmentTable,
        context: &AdjustmentContext,
    ) -> Result<(), AvengerChartError>;
}
```

`PointAdjustmentTable` should contain scaled x/y anchors, optional width/height
or radius, original row identity, and relevant grouping values. It should not
try to represent every possible mark shape in v1.

## Alternate Paradigms

- **Data transform before scaling**: good for binning and grouping, but wrong
  for jitter/dodge measured in pixels.
- **Scenegraph post-processing**: could adjust rendered scene marks directly,
  but it would lose channel/domain context and make legends/guides harder to
  reason about.
- **Per-mark builder options only**: simplest for built-ins, but it blocks a
  reusable adjustment ecosystem.

## Readiness

Ready for a design spike, not a full implementation plan.

The spike should implement one narrow point-only adjustment, probably
deterministic jitter for Cartesian `Symbol`, and use that to decide whether the
shared geometry table belongs in `avenger-chart-core`, `avenger-chart-marks`,
or the facade runtime.

## Decisions Needed

- Whether adjustment traits are core extension contracts or built-in facade
  features.
- How to represent adjusted geometry without forcing all marks into the same
  shape model.
- How adjusted positions interact with clipping, legends, hit testing, and
  scale-domain inference.
- Whether collision avoidance uses pre-render approximations or the final
  `SceneGraphRTree`.
- How deterministic randomness is configured and serialized.
