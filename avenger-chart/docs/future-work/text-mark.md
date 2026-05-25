# Text Mark

## Goal Review

The goal is valid: the chart authoring API needs a high-level data text mark
for labels, annotations, direct labeling, and derived mark workflows.

Several lower-level pieces already exist:

- `avenger-scenegraph::marks::text::SceneTextMark` is the rendered scenegraph
  primitive.
- Cartesian and Polar guides render text labels and titles through
  `SceneTextMark`.
- `CompiledPlot` renders plot titles and subtitles with text scene marks.
- Container/facet labels use shared scenegraph text utilities.

What is missing is a user-facing `Text<C>` mark with channel configs,
coordinate-specific position support, scale/legend behavior for visual
channels, and tests.

## Current System Fit

`Text<C>` should be a normal mark in the `avenger-chart-marks` family with
coordinate-specific render implementations in coordinate crates where needed.
It should use the same core contracts as `Line`, `Rect`, and `Symbol`:

- `Mark<C>` and `CompiledMark`,
- `MarkState` and `CompiledMarkState`,
- `ChannelValue`,
- coordinate-specific position channel extension traits,
- normal visual channels such as `fill`, `opacity`, and `angle`.

The text mark should not be implemented as a guide-only feature. Guides already
use text internally, but data labels need ordinary mark behavior.

## Recommended Direction

Implement a minimal `Text<C>` mark before smart label placement:

- required or defaultable `text` channel,
- coordinate position channels from the target coordinate crate,
- unscaled visual channels for font family, font size, align, baseline, angle,
  dx, dy, and limit,
- scaled visual channels only where they make sense, probably `fill` and
  `opacity`,
- `Cartesian` render support first, then `Polar` if the positioning semantics
  are clear.

Smart label placement should wait for the post-scale adjustment boundary in
[adjust-api.md](adjust-api.md).

## Alternate Paradigms

- **Annotation objects outside marks**: useful for fixed annotations, but not
  enough for data-driven labels.
- **Derived labels only**: makes common labels convenient, but still needs an
  underlying text mark.
- **Guide labels**: appropriate for axes and facets, not for arbitrary data
  rows.

## Readiness

Ready for an implementation plan for the minimal mark.

Smart placement is not ready; it depends on adjustment/collision contracts.

## Decisions Needed

- Which channels are scaled and which are literal by default.
- Whether `text` can be conditional and whether null text suppresses rows.
- How text bounds are measured before rendering for collision and layout.
- Which coordinate crates implement text positioning in the first slice.
- How text participates in legends, if at all.
