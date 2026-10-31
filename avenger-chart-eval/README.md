# avenger-chart-eval

The evaluation of an `avenger-chart` chart to an Avenger scene graph at any size.

`ChartEvaluator::new` prepares a chart one time, with a label engine and a `Theme`: it resolves
the chart's shared scales, summarizes their domains, and prepares their guides, which format and
measure their labels. `ChartEvaluator::evaluate` then takes a width and a height in pixels and
returns an `EvaluatedChart`:

- `scene`, the scene graph of the chart at that size.
- `data`, the rows of each mark's table behind its instances, for tooltips and selections.
- `plots`, the rectangle and the coordinate system of each plot, with its fitted scales, and its
  `PlotProjection` from avenger-coords between its pixels and the positions of its two positional
  dimensions, through which the tools of avenger-app-tools pan, zoom, and select in any coordinate
  system.
- `warnings`, such as a plot too small for its marks and labels.

At each size, avenger-layout places the plots and the parts of their guides, each scale fits its
domain to the length of its plots, and the marks encode their rows through the fitted scales. A
resize reuses what the evaluator prepared, so an app can evaluate a chart on every frame of a
resize.

```rust
use avenger_chart::Chart;
use avenger_chart_eval::{ChartEvaluator, EvalError, Theme};
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_typst_label::LabelEngine;

/// The scene of `chart` at each of `sizes`.
fn scenes(
    chart: &Chart,
    engine: &LabelEngine,
    sizes: &[[f32; 2]],
) -> Result<Vec<SceneGraph>, EvalError> {
    let mut evaluator = ChartEvaluator::new(chart, engine, &Theme::default())?;
    sizes
        .iter()
        .map(|size| Ok(evaluator.evaluate(*size)?.scene))
        .collect()
}
```

`examples/chart-facets` evaluates a facet grid in an avenger-app window that resizes, and writes
it as SVG through avenger-svg.

## Theme

A `Theme` gives what a chart leaves unset: the color, size, and font of marks without encodings
of them, the palettes and ranges of scales, the styles of axes, legends, colorbars, titles, and
facet headers, the spaces between plots and guides, and the style of maps.

## Selections

`set_selections` replaces the contributions of a chart's selections, and the next evaluation
applies the conditions of its encodings and fits again the scales whose domains come from a
selection.

## Views

A continuous scale of `x` or `y` can have a view, a scale whose domain replaces the fitted domain
at every size, as a pan or a zoom gives. `scale_view_mut` returns the view of a plot's scale to
move, and starts it from the fitted scale at the plot's last size. Every plot of the scale shows
the view and clips its marks, its axes label the view's domain, and the layout measures again
only the axes whose views changed. `home_scale` gives a fitted scale, and `clear_scale_view`
returns a scale to it. `scale_navigation` reads the navigation of a scale from its spec, with the
fitted domain at the plot's last size in place of data bounds.

A scale whose domain comes from a selection has no view, because the selection holds its
domain.

## Updates

`PreparedChart::new` prepares a chart on any thread: its scale groups, its scales fitted to its
rows, its guides, and its layout. `install` replaces the evaluator's chart with a prepared one,
and carries the state of the old chart by its `UpdateOptions`. Each scale carries over to the
scale of the new chart with its identity and its scale type: its share family, or the first plot
of the scale with an id, by its id and its partition. Its fitted domain fits the new rows, keeps
its old domain, or extends it to hold both (`Fit`), and its view stays or goes (`Views`). Each
selection keeps its contributions, without those of the plots that the new chart no longer has,
or takes the initial contributions of the new chart (`Carry`).

## Map plots

The `geo` feature draws map plots, through avenger-geo and avenger-guides-geo, which bring
GeoArrow, WKB, and GeoJSON. Without it, preparing a chart with a map plot fails with an error that
names the feature. A map plot with raster tiles loads them through the host's image resolver,
which `set_image_resolver` sets.
