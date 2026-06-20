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
| [mark-effects.md](mark-effects.md) | Unifying proposal for post-scale mark evaluation frames, adjustments, derivations, geometry space, and effect scheduling. |
| [polar-geometry-space.md](polar-geometry-space.md) | Plan for coordinate-vs-display geometry semantics for Polar line and text marks. |
| [adjust-api.md](adjust-api.md) | Valid goal; not ready for implementation planning until the post-scale data/geometry boundary is designed. |
| [derive-api.md](derive-api.md) | Valid goal; depends on the same geometry boundary as adjustments plus a child-mark scheduling model. |
| [transform-system.md](transform-system.md) | Valid goal; closest to an implementation plan for a narrow data-transform v1. |
| [tools.md](tools.md) | Pan/scroll-zoom, box-zoom, point-selection, and lasso-selection tools exist; richer editable selection chrome and toolbar composition remain future work. |
| [text-mark.md](text-mark.md) | Follow-up goal; `Text<Cartesian>` exists, while smart placement and text-specific legends remain future work. |
| [faceting.md](faceting.md) | Mostly implemented for row/column facets; remaining work is facet convenience APIs and mark data strategy. |
| [repeat.md](repeat.md) | Valid goal; design should choose between schema reshaping, template expansion, or dedicated repeat containers. |
| [layout.md](layout.md) | Valid goal; concat covers some composition, but arbitrary dashboard composition is still separate. |
| [async-rasterized-marks.md](async-rasterized-marks.md) | Valid goal; external Datashader-style rasterized mark crates need generic materialization/resource primitives. |
| [async-m4-lines.md](async-m4-lines.md) | Valid goal; external M4 line-downsampling mark crates need generic view-dependent materialized data primitives. |
| [map-tiles.md](map-tiles.md) | Valid goal; external Web Mercator coordinate crates can use coordinate-guide tile underlays after generic async-resource primitives land. |
| [multi-dim-coords.md](multi-dim-coords.md) | Valid goal; requires a repeated or indexed channel model. |
| [sankey-coords.md](sankey-coords.md) | Valid goal; likely a graph-layout coordinate/mark family, but other paradigms remain plausible. |
| [hierarchical-coords.md](hierarchical-coords.md) | Valid goal; treemap/sunburst can be coordinates, transforms, or specialized marks. |

## Readiness Scale

- **Ready for implementation plan**: the architecture boundary is clear and
  the next work can be chunked into code changes.
- **Ready for design spike**: the goal is valid, but one or two concrete
  design decisions should be resolved with prototypes or small experiments.
- **Discovery first**: the goal is valid, but the model still competes with
  other paradigms or depends on missing lower-level contracts.
