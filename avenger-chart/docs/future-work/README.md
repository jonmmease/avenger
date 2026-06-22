# Avenger Chart Future Work

This directory contains current design notes for features that are not yet part
of the architecture reference. The canonical description of the implemented
system is in [../architecture/README.md](../architecture/README.md).

Each note answers the same review questions:

- Is the goal still valid for the current project?
- What parts are already implemented in another form?
- Which direction fits the current crate, coordinate, mark, scale, guide, and
  child-frame architecture?
- Which alternate paradigms are plausible?
- How close is the topic to an implementation plan?
- Which major decisions still need to be made?

The library is not public yet, so these notes may propose breaking changes.
They should not preserve compatibility with older sketches when a cleaner
current design is available.

## Current Notes

| Document | Review status |
| --- | --- |
| [mark-effects.md](mark-effects.md) | Implemented v1 surface plus remaining follow-ups for richer mark effects, derived outputs, and compound-mark encapsulation. |
| [polar-geometry-space.md](polar-geometry-space.md) | Plan for coordinate-vs-display geometry semantics for Polar line and text marks. |
| [polar-line-implementation-plan.md](polar-line-implementation-plan.md) | Phase checklist for implementing `Line<Polar>` and the `geometry_space` option. |
| [transform-system.md](transform-system.md) | Remaining built-in data transforms, time/bin refinements, pushdown, caching, and user-doc work. |
| [tools.md](tools.md) | Pan/scroll-zoom, box-zoom, point-selection, and lasso-selection tools exist; richer editable selection chrome and toolbar composition remain future work. |
| [text-mark.md](text-mark.md) | Follow-up goals after `Text<Cartesian>` and fixed label placement: richer label placement, polar text, and text-specific legends. |
| [faceting.md](faceting.md) | Facet row/column/wrap and mark data scope are implemented; remaining work is mostly `FacetGrid` sugar and polish. |
| [layout.md](layout.md) | Valid goal; concat covers some composition, but arbitrary dashboard composition is still separate. |
| [async-rasterized-marks.md](async-rasterized-marks.md) | Valid goal; external Datashader-style rasterized mark crates need generic materialization/resource primitives. |
| [async-m4-lines.md](async-m4-lines.md) | Valid goal; external M4 line-downsampling mark crates need generic view-dependent materialized data primitives. |
| [map-tiles.md](map-tiles.md) | Valid goal; external Web Mercator coordinate crates can use coordinate-guide tile underlays after generic async-resource primitives land. |
| [multi-dim-coords.md](multi-dim-coords.md) | Valid goal; requires a repeated or indexed channel model. |
| [sankey-coords.md](sankey-coords.md) | Valid goal; likely a graph-layout coordinate/mark family, but other paradigms remain plausible. |
| [hierarchical-coords.md](hierarchical-coords.md) | Follow-up goal; `avenger-chart-treemap` exists, while sunburst/icicle and shared hierarchy abstractions remain future work. |

## Readiness Scale

- **Ready for implementation plan**: the architecture boundary is clear and
  the next work can be chunked into code changes.
- **Ready for design spike**: the goal is valid, but one or two concrete
  design decisions should be resolved with prototypes or small experiments.
- **Discovery first**: the goal is valid, but the model still competes with
  other paradigms or depends on missing lower-level contracts.
