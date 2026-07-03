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

## Implemented Or Partially Completed

These notes describe features where a v1, partial implementation, or adjacent
architecture already exists. They remain here because follow-up design work is
still useful.

| Document | Current state |
| --- | --- |
| [mark-effects.md](mark-effects.md) | Implemented v1 surface plus remaining follow-ups for richer mark effects, derived outputs, and compound-mark encapsulation. |
| [geo-coordinate-system.md](geo-coordinate-system.md) | Implemented: the `avenger-geo` projection engine (d3-geo parity) and the `avenger-chart-geo` coordinate system — graticule/sphere guides, WKB-backed GeoJSON marks, great-circle lines, pan/zoom with the adaptive Web Mercator blend, warped raster tile layers, and the retirement of `avenger-chart-webmercator` at ≥0.9999 pixel parity. |
| [polar-geometry-space.md](polar-geometry-space.md) | Implemented v1 pieces exist; remaining work is mostly coordinate-vs-display geometry polish for Polar line and text marks. |
| [polar-line-implementation-plan.md](polar-line-implementation-plan.md) | Phase checklist for `Line<Polar>` and the `geometry_space` option; several implementation phases have landed and remaining phases are tracked in the note. |
| [transform-system.md](transform-system.md) | Core transform system and several transforms exist; remaining work covers additional built-ins, time/bin refinements, pushdown, caching, and user docs. |
| [tools.md](tools.md) | Pan/scroll-zoom, box-zoom, point-selection, and lasso-selection tools exist; richer editable selection chrome and toolbar composition remain future work. |
| [text-mark.md](text-mark.md) | `Text<Cartesian>` and fixed label placement exist; remaining work is richer label placement, polar text, and text-specific legends. |
| [faceting.md](faceting.md) | Facet row/column/wrap and mark data scope are implemented; remaining work is mostly `FacetGrid` sugar and polish. |
| [layout.md](layout.md) | Concat covers some composition needs; arbitrary dashboard composition remains a separate design spike. |
| [map-tiles.md](map-tiles.md) | Implementation started; `avenger-chart-webmercator` owns WebMercator, coordinate-owned tile configuration, guide-rendered resource image tiles, and WebMercator tools. |
| [hierarchical-coords.md](hierarchical-coords.md) | `avenger-chart-treemap` exists; sunburst/icicle and shared hierarchy abstractions remain future work. |

## Not Yet Implemented

These notes describe valid goals with no dedicated implementation in the current
chart stack.

| Document | Review status |
| --- | --- |
| [typst-math-typesetting.md](typst-math-typesetting.md) | Ready for staged implementation planning; keep cosmic for regular text, use optional `avenger-typst-label` for `$...$` math spans, start with metrics/paths/raster/PDF glyph data and path-based SVG/PDF output. |
| [view-domain-inference-implementation-plan.md](view-domain-inference-implementation-plan.md) | Ready for implementation plan; moves domain inference control to channels and sketches standalone View transforms with retained View results. |
| [raster-arrow-representations.md](raster-arrow-representations.md) | Design spike in progress; specifies Arrow struct representations for uniform, rectilinear, quadmesh, and future cellmesh rasters. |
| [rasterize-uniform-2d-udaf.md](rasterize-uniform-2d-udaf.md) | Design spike in progress; plans a non-materializing `RasterizeUniform2D` transform implemented with a custom DataFusion UDAF. |
| [async-rasterized-marks.md](async-rasterized-marks.md) | Ready for design spike; external Datashader-style rasterized mark crates need generic materialization/resource primitives. |
| [async-m4-lines.md](async-m4-lines.md) | Ready for design spike; external M4 line-downsampling mark crates need generic view-dependent materialized data primitives. |
| [multi-dim-coords.md](multi-dim-coords.md) | Ready for design spike; requires a repeated or indexed channel model. |
| [sankey-coords.md](sankey-coords.md) | Valid goal; likely a graph-layout coordinate/mark family, but other paradigms remain plausible. |
| [pattern-fill-requirements.md](pattern-fill-requirements.md) | Draft requirements for pattern fill overlays, including scenegraph/chart integration, CSS theme parsing, legend behavior, and renderer-facing constraints. |
| [number-formatting.md](number-formatting.md) | Master formatting plan; creates `avenger-format-number`, owns built-in locales and locale registration, defines the layered d3-style string plus override/context API, and adds CLDR-backed `S`, `L`, and `C[ISO]` types. |
| [datetime-formatting.md](datetime-formatting.md) | Master datetime formatting plan; creates `avenger-format-datetime`, uses LDML patterns plus `{datetime:medium}`-style CLDR presets, adds custom serde locales, `#datefmt`, and temporal axis tick-label fragments. |

## Readiness Scale

- **Ready for implementation plan**: the architecture boundary is clear and
  the next work can be chunked into code changes.
- **Ready for design spike**: the goal is valid, but one or two concrete
  design decisions should be resolved with prototypes or small experiments.
- **Discovery first**: the goal is valid, but the model still competes with
  other paradigms or depends on missing lower-level contracts.
