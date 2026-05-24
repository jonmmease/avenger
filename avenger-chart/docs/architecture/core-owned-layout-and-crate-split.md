# Core-Owned Layout And Crate Split

Avenger treats the layout system as a core chart feature. Built-in facet,
concat, partition, layout refinement, and runtime child-frame placement stay in
the top-level `avenger-chart` crate rather than becoming external extension
points.

The related extension point is narrower: coordinate-system crates may opt into
`Subplot<Coord>` by implementing `SubplotContainerCoordinateSystem`. That hook
lets a coordinate system compile subplot marks into its own coordinate-specific
compiled mark representation. It does not expose the facet/concat layout engine
or adaptive refinement machinery.

Planned crate direction:

- `avenger-chart-core`: shared traits, shared types, spec types,
  `SubplotContainerCoordinateSystem`, `AxisPosition`, and `EvaluationContext`.
- `avenger-chart-marks`: `Mark`, `Subplot`, and generic mark families.
- `avenger-chart-scales`: builders, inference, defaults, UDFs, and codecs.
- `avenger-chart-legend`: legend rendering, colorbars, and symbol/line/rect
  legend support.
- `avenger-chart-cartesian`: Cartesian coordinate system, axes, Cartesian mark
  implementations, and Cartesian subplot positioning.
- `avenger-chart-polar`: Polar coordinate system and polar mark
  implementations.
- `avenger-chart`: `Plot`, `CompiledPlot`, built-in containers, partitioning,
  layout/runtime engines, WGPU integration, prelude, and facade re-exports.

This direction supersedes earlier experiments that tried to make facet-like and
concat-like layout containers implementable from external crates. The useful
lesson from those experiments is retained in the `Subplot<Coord>` compile hook;
the broader container layout and mutation surface is intentionally not carried
forward.
