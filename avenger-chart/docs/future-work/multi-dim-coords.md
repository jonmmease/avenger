# Multi-Dimensional Coordinates

## Goal Review

The goal is valid: parallel coordinates, radar charts, star glyphs, and related
views need a way to map one row into multiple positioned samples. The current
channel model is mostly single-valued by channel name, so this is not just a
new coordinate crate.

The old repeated `.y(...)` or `.r(...)` sketch is conceptually good, but it is
not compatible with the current `DataContext` shape without additional design.
Channels are stored by string key, and scale/axis extraction expects stable
channel names. Calling `.y(...)` repeatedly would overwrite the same channel
unless the mark stores indexed channels outside normal `MarkState`.

## Current System Fit

Existing pieces that help:

- Custom coordinate systems can live outside the facade and depend on
  `avenger-chart-core`.
- Custom marks can own coordinate-specific authoring methods.
- Coordinate transforms can return custom `PlotGeometry`; mark renderers
  already downcast to the geometry they expect.
- Scale specs and axis configs are keyed by channel names, so indexed names
  such as `y_0`, `y_1`, or field-derived names could work today.

The missing piece is a first-class way to represent multiple instances of the
same semantic channel while still giving each instance independent scale,
axis, and title configuration.

## Recommended Direction

Start outside core with explicit indexed channel names. A `ParallelLine` mark
can provide repeated authoring methods but internally write channels such as
`dim_0`, `dim_1`, and `dim_2` into `MarkState`. The mark's compiled metadata
can preserve display order and original labels.

That lets the project build a real parallel-coordinate prototype without first
changing core channel storage. If the prototype proves useful, introduce a
core `ChannelInstance` or `ChannelKey { name, index }` model later.

Radar can follow the same model, but it should be treated as a separate design
because radar has polar/radial guide expectations and often aggregates or
normalizes dimensions differently.

## Alternate Paradigms

- **Long-form data plus ordinary lines**: reshape rows into `(entity,
  dimension, value)` records and plot with Cartesian or Polar coordinates.
  This is often enough for simple radar/parallel views, but it makes per-row
  path assembly and per-dimension axis configuration less direct.
- **Dedicated mark with internal layout**: fastest prototype, but it bypasses
  normal scale/guide ownership unless carefully integrated.
- **Core repeated channel model**: clean long term, but a larger breaking
  change that should be justified by a working prototype.

## Readiness

Ready for a design spike.

The first spike should implement parallel coordinates with explicit indexed
channels in a coordinate/mark crate. The spike should prove scale extraction,
axis title extraction, and legend behavior before changing core channel
storage.

## Decisions Needed

- Whether repeated semantic channels become a core concept.
- How ordered dimensions are named, serialized, and shown in axis titles.
- Whether all marks in a multi-dimensional coordinate plot must share the same
  dimension list.
- Whether dimensions default to independent scales, normalized shared scales,
  or user-selected sharing.
- How null values split or skip polylines.
