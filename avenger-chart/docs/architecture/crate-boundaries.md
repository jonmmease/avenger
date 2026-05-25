# Crate Boundaries

Avenger treats chart layout as a core chart feature. Built-in facet, concat,
partitioning, layout refinement, child-frame placement, and render-time layout
coordination live in the top-level `avenger-chart` crate.

The user-extensible boundary is narrower:

- custom marks depend on `avenger-chart-core` for mark contracts,
- custom scales depend on `avenger-chart-core` plus lower-level
  `avenger-scales` when they implement scale math,
- custom legend renderers depend on `avenger-chart-core`,
- custom coordinate systems depend on `avenger-chart-core` and any lower-level
  runtime crates needed for their implementation,
- coordinate systems that support coordinate-positioned child plots implement
  `SubplotContainerCoordinateSystem`.

Facet and concat containers are not external layout-container APIs.

## Chart-Layer Crate Graph

Dependency arrows are provider-to-consumer.

```text
avenger-chart-core
  -> avenger-chart-marks
  -> avenger-chart-scales
  -> avenger-chart-legend
avenger-chart-marks
  -> avenger-chart-cartesian
  -> avenger-chart-polar
avenger-chart-cartesian
  -> avenger-chart
avenger-chart-polar
  -> avenger-chart
avenger-chart-legend
  -> avenger-chart
```

Lower-level runtime crates such as `avenger-scales`, `avenger-scenegraph`,
`avenger-guides`, `avenger-text`, `avenger-app`, and `avenger-wgpu` remain
separate dependencies below this chart-layer graph.

## `avenger-chart-core`

`avenger-chart-core` owns the shared chart kernel. It does not depend on the
top-level `avenger-chart` crate or on the built-in Cartesian, Polar, mark,
scale, or legend implementation crates.

Core owns:

- object-safe mark contracts: `Mark`, `CompiledMark`, `CompiledMarkCore`,
  `MarkRuntimeContext`, `MarkRenderContext`, `MarkState`,
  `CompiledMarkState`, `DataContext`, and `CompiledDataContext`,
- custom-mark helper macros and common channel configuration contracts,
- shared channel/spec value types such as `ChannelValue`, `ConditionalValue`,
  `ChannelDescriptor`, `ChannelDefault`, `BaseChannelName`, `ScaleSharing`,
  `SharingLevel`, `Maybe`, `Param`, `RadiusExpression`, and serializable
  expression/scalar wrappers,
- scale authoring contracts and preferences: `Scale`, `Auto`,
  `ScaleChannelConfig`, `ScaleChannelValue`, `ScaleSpec`, `ScaleRange`,
  `ScaleDomain`, `ScaleDefaultDomain`, `DomainExpr`, `ResolvedDomain`, and
  `ScaleTypePreference`,
- legend specs and renderer contracts: `Legend`, `LegendRenderer`,
  `LegendRendererSelection`, `LegendChannel`, `ChannelInfo`, `MergeKey`,
  `ConfiguredScaleLegendExt`, and `DomainValues`,
- guide and coordinate contracts: `Axis`, `CoordinateSystemCore`,
  `CoordinateSystem`, `CoordinateSystemTransformCore`,
  `CoordinateSystemTransform`, `CoordinateGuide`, `CompiledGuide`,
  `GuideSharingContext`, `GuideUpdate`, `AxisVisibility`, and guide-sharing
  view traits,
- subplot contracts: `SubplotChildPlotSpec`, `CompiledSubplotChildPlot`,
  `SubplotMarkCore`, `SubplotDataSource`, `CompiledSubplotPayload`,
  `SubplotContainerCoordinateSystem`, `PositionedSubplotChannel`,
  `PositionedSubplotSpec`, `PositionedSubplotMarkCore`,
  `CompiledPositionedSubplot`, `compile_subplot_payload`, and
  `compile_positioned_subplot_mark`,
- geometry and layout value types such as `PlotGeometry`, `PointGeometry`,
  `SubplotRect`, `SubplotGeometry`, `BandPosition`, `PaddingSpec`,
  `LayoutBounds`, `Size2D`, `EdgeSlabs`, `OverflowSide`, `FrameAllocation`,
  `FrameDemand`, `FrameLayout`, `AxisPosition`, `LegendPosition`,
  `LegendOrientation`, `FacetAxis`, `CoordinatedOverflow`, and
  `CoordinatedLayout`,
- `FacetEmptyCellPolicy`, `FacetDimensionConfig`, `RowDimensionConfig`, and
  `ColumnDimensionConfig`,
- core `EvaluationContext`, theme and color evaluation, strict color parsing,
  expression evaluation helpers, DataFusion/scalar helper traits, channel
  coercion helpers, `AvengerChartError`, and `ChannelResolutionError`,
- `ZeroDCoord`.

Core does not own layout solvers, facet/concat runtime state, child-frame
measurement, positioned-subplot measurement/rendering, WGPU rendering, app
integration, or visual-test harness code.

## `avenger-chart-marks`

`avenger-chart-marks` owns neutral built-in mark families and shared mark
authoring ergonomics:

- `Line<C>`,
- `Rect<C>`,
- `Symbol<C>`,
- `Subplot<C>`,
- common mark-channel builders,
- mark-specific default descriptors,
- small neutral helpers used by coordinate-specific mark implementations.

Coordinate-specific render implementations do not live here. They live with
the coordinate crate that owns the coordinate system.

## `avenger-chart-scales`

`avenger-chart-scales` owns built-in chart scale implementations and scale
runtime construction:

- built-in scale marker types: `Linear`, `Log`, `Pow`, `Sqrt`, `Symlog`,
  `Time`, `Band`, `Point`, `Ordinal`, `Threshold`, `Quantile`, and `Quantize`,
- built-in scale option extension traits such as `LinearScaleExt`,
  `BandScaleExt`, and `OrdinalScaleExt`,
- `ScaleBuilder`,
- `ChannelScaleData`,
- `DataExtents`,
- `DomainExtent`,
- `ConfiguredScaleWithSpec`,
- scale runtime extension traits,
- default range helpers,
- scale UDF construction,
- `AvengerChartExtensionCodec`,
- chart-specific logical expression and logical plan serialization helpers
  that need the scale extension codec,
- `build_scale_builder_from_marks`.

The lower-level `avenger-scales` crate owns runtime scale implementations.

## `avenger-chart-legend`

`avenger-chart-legend` owns built-in legend authoring and rendering:

- `LegendBuilder` and typed legend builders,
- `LegendableChannel`,
- `LegendableChannelValue`,
- `renderer_for_kind`,
- `CompiledSymbolLegend`,
- `CompiledLineLegend`,
- `CompiledRectLegend`,
- `CompiledColorbar`,
- `LegendMeasurement` and `LegendMeasurements`,
- legend size measurement,
- legend theme/default application helpers.

Plot-level legend planning remains in `avenger-chart` because it walks compiled
plots, coordinates legend hoisting through facets and child-frame containers,
and feeds the core-owned layout runtime.

## `avenger-chart-cartesian`

`avenger-chart-cartesian` owns the Cartesian coordinate package:

- `Cartesian`,
- `CartesianAxis`,
- `CartesianGuide`,
- `CartesianOptions`,
- `CartesianPositionConfig`,
- Cartesian channel and axis evaluation,
- Cartesian guide measurement/rendering,
- coordinate-specific `Mark<Cartesian>` and `CompiledMark` implementations for
  `Line`, `Rect`, and `Symbol`,
- Cartesian position-channel builder extension traits,
- `CartesianSubplotPositionChannels`,
- `CompiledCartesianSubplot` as the Cartesian positioned-subplot compiled mark
  alias,
- the `SubplotContainerCoordinateSystem` implementation for `Cartesian`.

Cartesian declares positioned-subplot placement channels `subplot_x` and
`subplot_y`, mapped to transform channels `x` and `y`.

## `avenger-chart-polar`

`avenger-chart-polar` owns the Polar coordinate package:

- `Polar`,
- `PolarAxis`,
- `PolarAxisType`,
- `PolarDirection`,
- `PolarGuide`,
- `PolarOptions`,
- `PolarPositionConfig`,
- Polar channel and axis evaluation,
- Polar guide measurement/rendering and circular clipping,
- coordinate-specific `Mark<Polar>` and `CompiledMark` implementations for
  `Symbol`,
- `PolarSymbolPositionChannels`,
- `PolarSubplotPositionChannels`,
- `CompiledPolarSubplot` as the Polar positioned-subplot compiled mark alias,
- the `SubplotContainerCoordinateSystem` implementation for `Polar`.

Polar declares positioned-subplot placement channels `r` and `theta`, mapped to
the same transform channel names.

## `avenger-chart`

`avenger-chart` is the high-level facade and the owner of core chart layout and
runtime evaluation.

It owns:

- `Plot`,
- `CompiledPlot`,
- plot compilation and evaluation,
- top-level plot specs that are not pure shared value types,
- facet and concat layout/container marks,
- partition tree construction for built-in layout containers,
- child-frame preparation, measurement, sharing, placement, and rendering,
- generic coordinate-positioned subplot measurement/rendering,
- plot-level scale planning and legend planning,
- layout solvers and runtime layout loops,
- layout/debug overlays,
- WGPU/app/canvas integration,
- facade prelude exports,
- integration and visual regression tests that exercise the full stack.

Facade modules re-export owner-crate APIs for chart-author ergonomics. The
source of behavior remains the owner crate listed above.

## Extension Contracts

External custom marks use `avenger-chart-core` mark contracts and can render
against any coordinate system whose transform implements the core position
projection contract.

External custom scales use `avenger-chart-core` authoring contracts. A custom
scale crate also depends on `avenger-scales` when it implements `ScaleImpl`.

External custom legend renderers implement `LegendRenderer` from
`avenger-chart-core`. Built-in legend builders and renderers are provided by
`avenger-chart-legend`.

External coordinate systems implement the core coordinate and guide traits. To
support coordinate-positioned `Subplot<Coord>` marks, a coordinate crate also
implements `SubplotContainerCoordinateSystem` and calls
`compile_positioned_subplot_mark` with its own `PositionedSubplotSpec`.

The top-level layout engine measures and renders positioned child frames. The
coordinate transform supplies the anchor positions and must return
`PointGeometry` for positioned-subplot placement.
