# Dashboard Layout

## Goal Review

The goal is valid: chart authors need to compose independent plots into
dashboards and multi-panel figures that are not driven by a single data
partition or a repeated template.

Part of the old layout goal is already covered in a different form:

- `HConcat` and `VConcat` are built-in layout coordinates that compose child
  plots as subplots.
- Concat children can have explicit child data or inherit parent data.
- Facet, concat, and positioned subplots share child-frame measurement,
  domain/guide/legend sharing, and render placement.
- `LayoutSpec`, `FrameLayout`, `ContentLayoutSolver`, and related types solve
  the internal frame layout of a single evaluated plot.

Those systems do not provide arbitrary dashboard layout. They are plot
coordinates or internal frame solvers.

## Current System Fit

A dashboard composition API should not return an image directly. The current
runtime separates evaluation from rasterization:

- `CompiledPlot::evaluate_with_options` returns `EvaluatedPlot`.
- `EvaluatedPlot` contains a `SceneGraph` and optionally a `SceneGraphRTree`.
- `WgpuRenderer` and `CanvasExt` consume compiled/evaluated plots for PNG or
  canvas output.

Dashboard layout should follow the same separation. A dashboard should compile
or evaluate into a scenegraph composition, then existing renderers should
rasterize it.

## Recommended Direction

Define a top-level composition type rather than overloading plot coordinates:

```rust
pub enum ChartNode {
    Plot(CompiledPlot),
    Grid(GridLayout),
    Flex(FlexLayout),
}
```

The evaluated output should be a scenegraph group with child plot scenegraphs
translated into allocated rectangles. A first slice can ignore cross-child
scale sharing and only compose already-compiled plots.

Use existing layout concepts where possible, but do not expose the internal
single-plot `LayoutSpec` as the dashboard API.

## Alternate Paradigms

- **Use `HConcat`/`VConcat` only**: good for simple stacks and side-by-side
  layouts, but insufficient for grid spanning, dashboards, and unrelated
  charts.
- **CSS/Taffy layout tree**: likely a good implementation detail, but the
  public API should be chart-oriented and typed.
- **HTML/SVG host layout**: keeps chart core smaller, but prevents WGPU/canvas
  rendering of complete dashboard scenegraphs.

## Readiness

Ready for a design spike, not a full implementation plan.

The first spike should compose two or three already-compiled plots into one
scenegraph with fixed pixel rectangles. After that, decide whether flexible
grid/flex sizing belongs in the same API.

## Decisions Needed

- Whether dashboard composition lives in `avenger-chart` or a new facade crate.
- Whether child plots are compiled before insertion or compiled as part of the
  dashboard.
- Whether dashboard-level titles, legends, params, themes, and interactions
  exist in v1.
- Whether cross-plot scale sharing is in scope, and if so how it differs from
  child-frame sharing.
- How dashboard layout interacts with `EvaluatedPlot` hit testing.
