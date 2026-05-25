# Architecture Docs Outline

This outline lists architecture sections that would make the current system
easier to navigate and maintain.

## Architecture Index

Add `avenger-chart/docs/architecture/README.md` with:

- the current chart-layer crate graph,
- links to each architecture document,
- a short description of the system area each document owns,
- rules for documentation hygiene: use type names and module paths, avoid
  source line-number anchors, and keep implementation plans separate from
  current architecture references.

## Child-Frame Runtime

Add a canonical child-frame runtime document covering:

- `ChildFrameRuntime`,
- `PreparedChildFramePlot`,
- `ChildFrameScopeKey`,
- `ChildFrameKey`,
- `ContainerPathSegment`,
- `ChildFrameSharingLevel`,
- `ChildFrameSharingPath`,
- `ChildFrameContainerView`,
- `ChildFrameDomainSharingInput`,
- `ChildFrameRenderPlacement`.

This section should explain how facet, concat, and coordinate-positioned
subplots all use child-frame identity, sharing paths, measured child-frame
views, and render placements.

## Legend And Domain Hoisting

Add a focused section for domain and legend ownership across nested containers.

Important names:

- `ScaleSharing`,
- `SharingLevel`,
- `ChildFrameDomainSharingInput`,
- `HoistedLegendRequest`,
- `HoistedLegendAnchor`,
- `ChildFrameSharingLevel`,
- `ContainerPathSegment`,
- `GuideSharingContext`.

The section should state the core rule: the scale/legend channel owns its own
sharing level. Positional scale sharing does not by itself promote visual
legends for `fill`, `stroke`, `size`, or `shape`.

## Extension Contracts

Add an extension-contract section organized by user-extensible surface:

- custom marks: `Mark`, `CompiledMark`, `CompiledMarkCore`,
  `MarkRuntimeContext`, `CompiledMarkState`, `DataContext`,
  `CompiledDataContext`, channel macros, and scale/legend capability
  descriptors,
- custom scales: `Scale`, `Auto`, `ScaleSpec`, `ScaleChannelConfig`,
  `ScaleChannelValue`, and the lower-level `ScaleImpl` handoff,
- custom legend renderers: `LegendRenderer`, `LegendRendererSelection`,
  `LegendChannel`, `ChannelInfo`, `ConfiguredScaleLegendExt`, and built-in
  renderer dispatch,
- custom coordinate systems: `CoordinateSystemCore`, `CoordinateSystem`,
  `CoordinateSystemTransformCore`, `CoordinateSystemTransform`,
  `CoordinateGuide`, `CompiledGuide`, and `GuideSharingContext`,
- positioned subplot support: `SubplotContainerCoordinateSystem`,
  `PositionedSubplotSpec`, `PositionedSubplotChannel`, and
  `compile_positioned_subplot_mark`.

This section should make the layout boundary explicit: facet and concat are
core-owned layout containers, while coordinate systems can opt into
coordinate-positioned `Subplot<Coord>` marks.

## Public Surface And Re-Exports

Add a short policy section that distinguishes:

- owner-crate public APIs,
- top-level facade re-exports,
- `#[doc(hidden)]` extension hooks,
- crate-private runtime machinery.

This will make it clearer which crate owns behavior when the same type is
reachable through the facade prelude and its owner crate.

## Testing And Baseline Map

Add a validation map that links architecture areas to focused checks:

- facet tree and sharing: `test_evaluated_facet_tree`,
- facet coordination: focused `facet` lib tests,
- child-frame/domain sharing: `container_domain_sharing`,
- positioned subplots: `test_cartesian_subplot`,
  `test_positioned_subplot_scale_sharing`, and
  `test_positioned_subplot_legend_sharing`,
- external extension boundaries: `avenger-chart-external-test`,
- visual regression categories for facet, concat, positioned subplots, and
  legend sharing.

The map should help contributors choose focused validation without requiring a
full release test suite for every mechanical architecture edit.
