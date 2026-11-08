# avenger-chart-eval

The evaluation of an `avenger-chart` chart to an Avenger scene graph at any size. This version draws
charts of Cartesian, polar, and geo plots: their marks, every scale type with every kind of domain,
axes, polar axes, graticules, symbol legends, colorbars, titles, and attributions, in concats, wraps,
and facets with their titles and labels, with empty cells, the part rules of compositions, and
scales that plots share. Geo plots need the `geo` feature, and without it a geo plot is
`EvalError::Unsupported`.

## Compositions

A concat lays out its children in a row or a column on the tracks of its table, and the plots of a
facet grid, a column of rows, line up in one table. A wrap fills rows with its children and reflows
as the chart resizes. A plot's width or height fixes the length of its track. An empty cell keeps
its track, and the shared axes of the plots in its row and column stand on the edge of the grid past
it. The guides of a scale that plots share follow the part rules of the scale's owner: by default a
concat or a wrap collapses the ticks and the labels of axes and the labels and titles of facets
where they line up, and merges axis titles, legends, colorbars, and attributions into one. A facet
labels the children of its concat, or each child of its wrap, with the values of its column in their
partitions, beside the facet's title. The titles of concats and wraps run along their side,
outermost.

## Polar plots

A polar plot draws its part of the circle inside its box, which keeps the shape of the box around
that part: a square for a whole circle, and twice as wide as tall for a half circle. The outer
radius is the largest at which the box holds the circle, the labels of its polar axes, and its
symbols and texts at their points, found by a fixed-point iteration because the labels of the axes
change with the radius. The polar plots that share a scale take the smallest outer radius among
them, so a value draws at one distance from each center. A text lies beside its point as its
computed `text-orient`, `radius-side`, and `theta-side` say, and a tangential text curves along the
circle through its point. A scale of `theta` or `radius` draws a polar axis only when its spec sets
one, with spokes and rings as its grid lines, and its labels have a halo of the plot's background.
The coordinates of a polar plot are its circle with the scale of `theta` and the scale of `radius`,
or without a scale of `radius`, the map of fractions of the outer radius to pixels.

## GeoPlots

With the `geo` feature, a geo plot projects longitudes and latitudes through an avenger-geo
projection: one of the typed projections of the format, a custom projection by its typetag name, or
the coordinates of a planar geo plot. The geo plots that share a projection fit one view of the
sphere, a box, or the data of their layers, inset by the largest reach of their marks past their
places, at the size of their first plot. A geo plot keeps the aspect of what it fits, or its own, or
none. A geo plot whose spec has a sphere fills and outlines the outline of its projection with the
computed background and border of its `plot[type=geo]`, and its graticule takes the styles of its
`graticule`, `grid`, and `label`, with its labels in slabs of the plot. Tiles load through
`ChartEvaluator::set_image_resolver`, and a geo plot without a resolver draws without them, with a
warning. A symbol, a text, an image, or a path draws the rows whose places the projection keeps. The
coordinates of a geo plot are its projection, without scales.

## Widgets

A widget of a layer draws where a legend would: on the side that its declaration or the computed
`orient` of its `widget` node gives, `offset` apart from the plot, on the node that holds the
copies of its key. The node draws it once, or once beside each plot of the group when its part rule
for `PART_WIDGET` keeps it. The widgets on one side share a line along it, in their order,
apart by their largest offset, and a widget that would pass the side's length starts a new line
farther out. The evaluator fits the domain once, when it prepares the chart:
the explicit values, the summaries of the copies, or the target's column in the drawn rows of the
copies' layers, with zero when the fit asks for it. A slider runs between the least and the
greatest number, and a radio group and a check box list list the distinct values in ascending
order, and a mode widget lists the chart's modes with their labels. The `widget` node gives the
font, the color of the text, and with `accent-color`, the color of the checked and filled parts.

A widget's value is the clause of its identity in the state of its selection, and its clear state
writes no clause. A mode widget's value is the chart's mode. `EvaluatedChart::widgets` holds each
widget with its target, its spec, its style, and the rectangle of each drawing.
`ChartEvaluator::widget_value` reads a widget's value, and `ChartEvaluator::write_widget` writes the
clause of a value to its selection, or sets the chart's mode. A `Control` is the avenger-widgets
control of a widget's spec over any state, which the evaluator draws over the value alone, and an
app over its own state. `ChartEvaluator::set_draws_widgets(false)` leaves the drawings out of the
scene for an app that draws its own at the widgets' rectangles.

## State and updates

An evaluation reads a state that changes without a new chart. `set_selections` replaces the clauses
of the selections, which the conditional values of encodings take at the next evaluation, and a
scale whose domain reads a selection fits again. `set_mode` sets the chart's mode, which `mode`
reads: the first mode of the chart until a mode is set, and `is_active` tells whether a tool that
names modes is active in it. `scale_view_mut` gives the view of a continuous
scale of `x` or `y`, which keeps its domain at every size, shows on its axes, and clips its plots,
and `projection_view_mut` gives the view of a geo plot's projection, which scales and moves with the
plot's fit as the plots resize. An evaluation drops a view that did not move. A scale has an
identity from the family of its spec or from the id of its first plot, and a projection from the
family of its spec or from the id of its first geo plot.

`PreparedChart::new` prepares a chart on any thread, and `ChartEvaluator::install` installs it with
`UpdateOptions`: a `Fit` that refits, keeps, or extends the fitted domain of each scale and
projection that carries over by identity, `Views` that keep or drop the views, and a `Carry` that
keeps or resets the clauses of each selection. The mode stays while the new chart declares it. A
chart whose style tree equals the old one keeps the computed styles.

## Preparation and evaluation

`ChartEvaluator::new` prepares a chart one time, with the cascade of a style table, a label engine,
and a `Registry` of custom scales. It checks that the label engine has a formatter of each format
syntax of the chart, builds the chart's style tree, finds the rows of each layer that draw an
instance, and reads the initial clauses of the selections.

`ChartEvaluator::evaluate` takes the host inputs of avenger-style and returns an `EvaluatedChart`:

- `scene`, the scene graph of the chart.
- `data`, the rows of each mark's table behind its instances, by the mark's name.
- `plots`, the rectangle of each plot in the scene, its `PlotCoords` from avenger-coords with the
  fitted scales of its first `x` and its first `y`, and its layers with their marks and
  coordinates. A layer with an independent scale of `x` or `y` has coordinates with that scale.
- `warnings`, such as a plot that is too small for its marks.

The chart draws at the host's size, at the chart's `width` and `height` in a dimension where the
host gives none, and at its natural size in a dimension without either, where a plot without a size
counts at its computed `contain-intrinsic-size`, or along a band, a point, or a nested band position
scale with a `step`, at the step times the steps of the scale, so a bar chart of 40 categories draws
wider than one of 3. avenger-layout places the plot and the parts of its axes and legends, each
position scale fits its domain to the plot's length with room for the reach of its marks, and the
marks encode their rows through the fitted scales.

```rust
use std::sync::Arc;

use avenger_chart::Chart;
use avenger_chart_eval::{ChartEvaluator, EvalError, Registry};
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_style::{Cascade, Env, HostInputs, ParseOptions, Sheet, SourceId, StyleTable};
use avenger_typst_label::LabelEngine;

/// The cascade of the base stylesheet.
fn base_cascade() -> Arc<Cascade> {
    let mut no_files = |url: &str, _: Option<&str>| Err(format!("no file `{url}`"));
    let options = ParseOptions {
        allow_import: true,
        ..Default::default()
    };
    let (sheet, _) = Sheet::load(
        "@import 'std:base.css';",
        None,
        SourceId(1),
        options,
        &mut no_files,
    );
    Arc::new(Cascade::new(&StyleTable::new().with_author(sheet, Env::new())))
}

/// The scene of `chart` at each of `sizes`.
fn scenes(
    chart: Chart,
    engine: LabelEngine,
    sizes: &[[f32; 2]],
) -> Result<Vec<SceneGraph>, EvalError> {
    let mut evaluator = ChartEvaluator::new(chart, base_cascade(), engine, Registry::new())?;
    sizes
        .iter()
        .map(|[width, height]| {
            Ok(evaluator.evaluate(&HostInputs::sized(*width, *height))?.scene)
        })
        .collect()
}
```

## What recomputes when

The styles of the style tree are computed again only when the host's preferences or theme change,
or when a resize changes the outcome of a condition, such as a `@media` query of the host's width.
The scales, the reach of the marks, the axes and legends, and the layout follow the styles. A resize within a
breakpoint reuses all of them as they were prepared. It solves the layout again, asking the axes and legends
only for the measurements that no longer hold, fits the position scales at the plot's new size, and
encodes the marks. The rows that draw and the selection states never recompute.

## Styles

Every style comes from the style table: the evaluator applies no default of its own. The base
stylesheet, which a table imports with `@import 'std:base.css'`, gives the look of Vega-Lite, and
the initial values of the properties give the rest. The style fields of the format are inline
styles on the nodes of the style tree, which `ChartEvaluator::style_tree` returns, so a stylesheet
selects the parts of a chart as CSS selects the elements of a page:

```css
plot > mark[type=symbol] { size: 60; }
axis[channel=y] > grid { display: none; }
legend { orient: bottom; }
```

`display: none` hides an axis, a legend, or a part of one, as the `show` and `show_*` fields of
`Axis` and `Legend` do, and a layer whose `mark` computes it draws nothing and has no rows. A hidden
layer's values still fit its scales, so a breakpoint that hides a layer does not rescale the others.

`ChartEvaluator::styles` returns the computed styles for host inputs, and their warnings are in the
warnings of each evaluation.

## Registry

A scale of a custom type names its implementation, and the `Registry` maps each name to an
`Arc<dyn ScaleImpl>` with `Registry::with_scale`. A name that the registry does not have is
`EvalError::UnknownScale`.

## Warnings and errors

The reader of the format guarantees a valid chart, so the errors are the parts that this version
does not draw, the names that the evaluator resolves, such as custom scales and format syntaxes,
and the failures of the crates that draw a chart. A warning is a part of the chart that draws, but
not as the chart says:

- `EvalWarning::Style`, a value of the styles that does not draw as written.
- `EvalWarning::NotDrawn`, a part that the evaluator draws without, such as a widget whose domain
  holds no values, whose initial clause still applies, a condition on a column that the layer does
  not have, the text of a style
  field that is not a value of its property, such as a color that CSS does not name, a percentage
  `size` of a mark without a band, or an `auto` size of a URL image.
- `EvalWarning::Fit`, marks that reach past the edges of their plot, because the plot is too small
  for them or the producer's summary did not hold their reach.
- `EvalWarning::Domain`, a domain that takes its fallback, such as a selection domain whose
  selection has clauses and none of them gives a range or a set through the domain's channel.
