# Facet Extensions

## Goal Review

The broad "faceting system" goal is mostly implemented for row and column
facets.

Implemented now:

- `FacetRow` and `FacetColumn` are built-in layout coordinates in
  `avenger-chart`.
- `Subplot<FacetRow>` and `Subplot<FacetColumn>` compile through
  `SubplotContainerCoordinateSystem`.
- Facet child plots inherit parent data and are filtered per facet cell.
- Nested row/column facets are supported.
- `ScaleSharing` and nested sharing levels coordinate domains, axis
  visibility, legends, and child-frame behavior.
- The runtime has `EvaluatedFacetTree`, `PartitionNode`, facet band
  measurement, guide sharing, and layout coordination.

Async mark compilation, row facets, column facets, nested facet coordination,
and shared scales are current implementation facts rather than future work.

## Remaining Valid Goals

### FacetWrap

`FacetWrap` remains useful as a convenience for one partition variable laid
out across a two-dimensional grid. It should be core-owned facade behavior,
not an external layout-container extension.

Potential implementation directions:

- a dedicated `FacetWrap` coordinate measurement that enumerates one partition
  dimension and assigns row/column grid positions,
- authoring sugar that rewrites to nested `FacetRow`/`FacetColumn` over
  computed row/column slot fields,
- a general "partition grid" primitive used only internally by built-in
  facets.

The rewrite approach is attractive if it can preserve scale-sharing and axis
ownership behavior without inventing another coordination pipeline.

### FacetGrid Convenience

Two-dimensional row-by-column faceting can already be represented by nested
`FacetRow` and `FacetColumn`. A `FacetGrid` API may still be worthwhile as
authoring sugar for the common case.

The main question is whether `FacetGrid` should be a real coordinate system or
a builder that expands to nested row/column facets. Expansion is preferable if
it can keep error messages and labels clear.

### Facet Mark Data Strategy

`FacetStrategy` exists in core with `Filter`, `Broadcast`, and `Skip`, and
mark builders have `broadcast_to_facets`. The current facet runtime does not
yet use that strategy as a complete data-selection policy. This is still valid
future work.

The motivating case is a faceted foreground layer over a broadcast background
layer, such as showing all points in gray and the current facet subset in
color.

## Already Accomplished By Other Means

Manual positioned faceting as a separate feature is partly covered by
coordinate-positioned subplots. `Subplot<Cartesian>` and `Subplot<Polar>` can
use placement channels and `partition_by(...)` to position child plots from
aggregate parent data. That is a better fit for "place subplots at data-driven
positions" than a separate manual `Facet` container.

`FacetGrid` examples that only need row/column partitioning are already
possible through nested `FacetRow`/`FacetColumn` plots, though the syntax is
verbose.

## Alternate Paradigms

- **Nested facets only**: keep runtime simple and add helper constructors or
  macros for grid/wrap authoring.
- **Dedicated facet coordinates for every shape**: clearer user-facing type
  names, but risks duplicating child-frame measurement logic.
- **Transform to synthetic row/column fields**: good for `FacetWrap`, but the
  transform must be deterministic, serializable, and visible to scale/guide
  planning.

## Readiness

FacetGrid convenience is ready for an implementation plan if it expands to
nested `FacetRow`/`FacetColumn`.

FacetWrap is ready for a design spike. The spike should prove whether computed
row/column fields can be inserted before `EvaluatedFacetTree` construction
without weakening domain sharing or empty-cell behavior.

Facet data strategy is ready for an implementation plan after one decision:
whether `Broadcast` and `Skip` are mark-level policies only, or whether
subplots need equivalent data-source policy controls.

## Decisions Needed

- Whether `FacetWrap` is a real coordinate system or expansion into nested
  facets over computed fields.
- Whether `FacetGrid` exists as a type or just as a builder/helper.
- How computed wrap slots are named and exposed in labels/debugging.
- How `FacetStrategy::Broadcast` interacts with mark-level data, plot-level
  data, child-frame inherited data, and positioned subplot partitioning.
- Whether empty-cell and axis-ownership modes need new defaults for wrapped
  facets.
