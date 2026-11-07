# avenger-chart-eval

The evaluation of an `avenger-chart` chart to an Avenger scene graph at any size. This version
draws charts of Cartesian plots: their 10 marks, every scale type with every kind of domain, axes,
legends, colorbars, titles, and attributions, in concats, wraps, and facets with their headers and
empty cells, with the guide rules of compositions and scales that plots share. A polar plot and a
map plot are `EvalError::Unsupported`.

## Compositions

A concat lays out its children in a row or a column on the tracks of its table, and the plots of a
facet grid, a column of rows, line up in one table. A wrap fills rows with its children and reflows
as the chart resizes. A plot's width or height fixes the length of its track, and an empty cell
keeps its track. The guides of a scale that plots share follow the guide rules of the scale's
owner: by default a concat or a wrap collapses the ticks and the labels of axes and the labels and
titles of facet headers where they line up, and merges axis titles, legends, colorbars, and
attributions into one. A facet header labels the children of its concat, or each child of its wrap,
with the values of the facet's column in their partitions. The titles of concats and wraps run
along their side, outermost.

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
host gives none, and at its natural size in a dimension without either, where a plot without a
size counts at its computed `contain-intrinsic-size`. avenger-layout places the
plot and the parts of its guides, each position scale fits its domain to the plot's length with
room for the reach of its marks, and the marks encode their rows through the fitted scales.

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
The scales, the reach of the marks, the guides, and the layout follow the styles. A resize within a
breakpoint keeps all of them: it solves the layout again, asking the guides only for the
measurements that no longer hold, fits the position scales at the plot's new size, and encodes the
marks. The rows that draw and the selection states never recompute.

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
- `EvalWarning::NotDrawn`, a part that the evaluator draws without, such as a widget, whose initial
  clause still applies, a condition on a column that the layer does not have, the text of a style
  field that is not a value of its property, such as a color that CSS does not name, a percentage
  `size` of a mark without a band, or an `auto` size of a URL image.
- `EvalWarning::Fit`, marks that reach past the edges of their plot, because the plot is too small
  for them or the producer's summary did not hold their reach.
