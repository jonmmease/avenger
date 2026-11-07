# avenger-chart-app

An avenger app that shows an avenger chart and serves the tools that its plots and layers declare.
`ChartApp::new(chart, cascade, engine)` builds an `AvengerApp<ChartAppState>` with four scenes:
the chart, the boxes and lassos of selections above it, the tooltips above them, and the widgets
above all.

## Tools

- A `Navigate` of a plot pans and zooms the scales of its navigated channels, keyed by the plot's
  identity and bounded by the scales' navigation limits, and with the `geo` feature, the projection
  of a map plot. A double click returns the scales home. A navigation of a scale whose domain
  reads a selection writes a range of the selection.
- A `Select` of a plot or a layer draws boxes, lassos, and clicks through the coordinates of its
  plot, so a box is a rectangle in a Cartesian plot, an annular sector in a polar plot, and ranges
  of longitude and latitude on a map. A layer's selection writes its own clauses and selects the
  instances of its own marks. A box with a `cell` selects whole cells and reports its focus to the
  host.
- A `Tooltip` of a plot or a layer describes the instances of its marks.
- A widget of a layer draws at the rectangle that the evaluation gave it, and each change of its
  value writes its clause to its selection. A slider's drag reports as it moves and settles at its
  end, and each click of a radio group or a check box settles at once. The app draws up to `SLOTS`
  drawings of widgets.
- On a plot that declares both a navigation and a box or a lasso, the app's `Mode` decides what a
  drag does.

## Updates and reports

A host changes the app only through one `ChartUpdates` handle, which any thread clones and sends
to: a `ChartUpdate` with a new chart, new style inputs, or both, the mode, and the states of
selections. The app prepares each new chart as a job of its runtime, so a native host gives the
built app a multi-thread runtime with `AvengerApp::with_runtime`, and it applies the updates and
the commands in the order of the calls. A newer chart overtakes the preparation of an older one,
and a chart that fails to prepare keeps the app's chart, with its error in `ChartUpdates::error`.

The app reports each `ChartChange`, the domains, the projections, and the selections that a
gesture changed, to the callback of `ChartApp::on_change`: every change, or with
`Reports::Settled`, the changes when a gesture settles.
