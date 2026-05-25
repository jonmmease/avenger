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

This direction supersedes earlier experiments that tried to make facet-like and
concat-like layout containers implementable from external crates. The useful
lesson from those experiments is retained in the `Subplot<Coord>` compile hook;
the broader container layout and mutation surface is intentionally not carried
forward.

## Target Crate Graph

Dependency arrows below are provider-to-consumer: the left crate is lower in
the stack and is available to the right crate.

```text
avenger-chart-core
  -> avenger-chart-marks
  -> avenger-chart-scales
avenger-chart-marks
  -> avenger-chart-legend
avenger-chart-scales
  -> avenger-chart-legend
avenger-chart-legend
  -> avenger-chart-cartesian
  -> avenger-chart-polar
avenger-chart-cartesian
  -> avenger-chart
avenger-chart-polar
  -> avenger-chart
```

The existing `avenger-scales` crate remains the lower-level runtime scale
library. The planned `avenger-chart-scales` crate is the chart-layer scale
builder/inference/spec/codec crate that currently lives under
`avenger-chart/src/scales`.

## Crate Ownership

### `avenger-chart-core`

Own the smallest shared chart kernel. It should not depend on `avenger-chart`,
`avenger-chart-marks`, `avenger-chart-scales`, `avenger-chart-legend`,
`avenger-chart-cartesian`, `avenger-chart-polar`, `avenger-app`, or
`avenger-wgpu`.

Initial candidates:

- Object-safe runtime traits and kernel types: `CompiledMark`, `CompiledGuide`,
  `Axis`, `CoordinateSystem`, `CoordinateSystemTransform`, `PlotGeometry`,
  `CoordMeasurement`, and `SubplotContainerCoordinateSystem`.
- Custom-mark authoring contracts: `DataContext`, `CompiledDataContext`,
  `MarkState`, `CompiledMarkState`, `FacetStrategy`, and the base
  `ChannelConfig` trait and common channel config structs, plus mark-facing
  scale preference helpers and core-safe mark-constructor/common-channel
  macros. The goal is that an external custom mark can depend on
  `avenger-chart-core` rather than pulling in the crate that contains built-in
  marks.
- Custom-coordinate authoring contracts: coordinate geometry value types and,
  after the measurement/render signatures are narrowed, the coordinate system,
  transform, guide, position-channel config, range-binding, and subplot
  compile-hook traits. The goal is that an external coordinate crate can depend
  on core rather than depending on Cartesian or Polar implementation crates.
- Core context types: the public/base `EvaluationContext`, mark-facing
  `MarkRenderContext`, parameter values, and session/theme/scale lookup views.
- Shared expression-evaluation helpers for spec expressions that resolve to
  primitive runtime values, axis/legend positions, and legend orientation.
- Shared DataFusion/scalar helper contracts used by marks, scales, and plot
  preparation: `DataFrameChartHelpers`, `ExprHelpers`, `ScalarValueHelpers`,
  `ArrayRefHelpers`, `eval_to_scalars`, `params_to_datafusion`,
  `scalar_to_scalar_value`, and aggregate-expression partitioning helpers.
- Shared mark channel coercion helpers that convert prepared data/scalar
  record batches into typed `ScalarOrArray` channel values. Top-level
  `CompiledMark` default-aware wrappers may remain above core until the
  object-safe mark render contract moves.
- Shared specs and value types: `ChannelValue`, `ConditionalValue`,
  `ChannelDescriptor`, `ScaleSharing`, scale/axis/legend spec data, `Maybe`,
  `Param`, `RadiusExpression`, and serialization wrappers. The `Legend` spec
  data now lives in core; renderer implementations and legend-builder
  extension traits stay in the legend layer.
- Shared layout value types only: `LayoutBounds`, `Size2D`, `EdgeSlabs`,
  `FrameAllocation`, `FrameDemand`, `FrameLayout`, `OverflowSide`,
  `AxisPosition`, `LegendPosition`, and `FacetAxis` or a neutral replacement.
- Theme and color evaluation, unless a later pass creates a dedicated
  `avenger-chart-theme` crate. Current mark, guide, legend, and scale APIs all
  need theme access, so theme cannot remain only in the top-level facade. The
  current implementation now lives in `avenger-chart-core`, with old
  `avenger_chart::theme::*` and `avenger_chart::color::*` paths kept as
  compatibility re-exports.
- `ZeroDCoord`, unless a separate no-op coordinate crate is introduced.

Do not move layout solvers, facet/concat runtime state, child-frame placement,
WGPU rendering, app integration, or visual-test harness code into core.

Current extraction state:

- `CompiledMarkCore` now lives in `avenger-chart-core` and owns the
  compiled-mark metadata, channel-planning, default-value, scale-preference,
  legend-capability, and radius-expression hooks that custom marks need without
  depending on the built-in mark crate.
- Top-level `avenger-chart::marks::CompiledMark` extends `CompiledMarkCore` and
  currently contains only the async render hook, because rendering still needs
  facade-owned runtime context and scene assembly types. Built-in marks,
  facet/concat subplot marks, Cartesian positioned subplot marks, and external
  dogfood marks implement the two traits separately.

### `avenger-chart-marks`

Own built-in generic mark definitions and their authoring ergonomics. The
shared custom-mark contracts belong in `avenger-chart-core` so external custom
marks do not need to depend on the built-in mark crate.

Initial candidates:

- Generic mark families: `Line<C>`, `Rect<C>`, `Symbol<C>`, and `Subplot<C>`.
- Common mark channel builders/macros and mark-specific default descriptors.
- Neutral `Subplot` payload plumbing after it no longer imports `Plot`,
  `CompiledPlot`, `concat`, `facet`, or `cartesian`.
- Compatibility re-exports for core mark contracts while the facade remains
  stable.

This crate must not own coordinate-specific compiled/render implementations.
Those belong to coordinate crates. It also must not depend on
`avenger-chart-scales` or `avenger-chart-legend`; channel `.scale(...)` and
`.legend(...)` ergonomics need to be supplied by extension traits from those
crates and re-exported by the facade prelude.

The generic built-in mark structs should not put a coordinate bound on the
struct definition itself. Coordinate bounds belong on coordinate-specific mark
impls and extension traits. This keeps the neutral mark family independent of
Cartesian/Polar and lets external marks use the same pattern.

### `avenger-chart-scales`

Own chart-layer scale construction.

Initial candidates:

- `Scale` user configuration and builder ergonomics. The `ScaleSpec` trait and
  built-in scale marker types (`Auto`, `Linear`, `Band`, etc.) now live in
  core as shared spec descriptors.
- `ScaleBuilder`, domain/range inference, default ranges, `DomainExtent`,
  `ConfiguredScaleWithSpec`, range bindings, UDFs, and chart extension codec.
- Extension traits that add `.scale(...)`, `.scale_with(...)`, and related
  scale configuration methods to core channel config types.

This crate may depend on the existing `avenger-scales` runtime crate. It should
consume core scale-domain requests and mark scale preferences rather than
importing concrete mark modules.

Current extraction state:

- The real `avenger-chart-scales` crate now owns scale authoring/configuration
  types and scale-adjacent utilities that no longer need top-level plot/layout
  state: `Scale<S>`, scale channel extension traits, scale UDF and codec,
  configured-scale extension traits, default range helpers, domain extent
  values, `ScaleBuilder`, and the chart-specific logical expr/plan
  serialization helpers that need the scale codec.
- Top-level `avenger-chart` still owns the adapter that constructs a
  `ScaleBuilder` from `CompiledMark` values and plot compilation state. The
  builder itself no longer imports `CompiledMark`; it accepts a default-range
  resolver callback supplied by the facade.

### `avenger-chart-legend`

Own legend planning and rendering.

Initial candidates:

- `LegendRenderer`, colorbar, line, rect, and symbol legend renderers.
- Legend builders and legend render plan assembly.
- Extension traits that add `.legend(...)` methods to channel config types.

To preserve the target graph, `CompiledMark` should not return
`Arc<dyn LegendRenderer>` directly unless the `LegendRenderer` trait is moved
down into core. The preferred direction is to refactor compiled marks to expose
core legend capability descriptors, then let this crate map descriptors to
concrete renderers.

The core `Legend` spec no longer stores custom `Arc<dyn LegendRenderer>`
overrides. Renderer selection is owned by the legend layer through
`LegendRendererKind` and concrete renderers, which keeps core spec data free of
legend-runtime trait objects.

Current extraction state:

- The real `avenger-chart-legend` crate now owns legend authoring builders and
  channel extension traits: `LegendBuilder`, the typed legend builders, and
  `LegendableChannel` / `LegendableChannelValue`.
- The real `avenger-chart-legend` crate now also owns legend renderer
  implementations and renderer dispatch: `LegendRenderer`, `LegendChannel`,
  `MergeKey`, `renderer_for_kind`, `CompiledSymbolLegend`,
  `CompiledLineLegend`, `CompiledRectLegend`, and `CompiledColorbar`.
- Plot legend planning remains in `avenger-chart` for now because it still
  walks compiled marks, configured scales, and layout placement state. Renderer
  implementations and legend size measurement consume core
  `LegendRendererKind`, core expression/color helpers, and scale-crate
  configured-scale extension traits through the legend crate.

### `avenger-chart-cartesian`

Own the Cartesian coordinate package.

Initial candidates:

- `Cartesian`, `CartesianAxis`, `CartesianGuide`, `CartesianOptions`,
  `CartesianPositionConfig`, axis rendering/evaluation, and Cartesian channels.
- Coordinate-specific `Mark<Cartesian>` impls and compiled renderers for
  `Line`, `Rect`, `Symbol`, and any other Cartesian marks.
- Cartesian positioned subplot compile/measure/render implementation, including
  `CompiledCartesianSubplot`.
- Cartesian-specific legend capability descriptors or adapter impls.

It may depend on core, marks, scales, and legend. It must not depend on the
top-level `avenger-chart` facade.

### `avenger-chart-polar`

Own the Polar coordinate package.

Initial candidates:

- `Polar`, `PolarAxis`, `PolarAxisType`, `PolarDirection`, `PolarGuide`,
  `PolarOptions`, and polar channels.
- Coordinate-specific `Mark<Polar>` impls and compiled renderers, currently
  `Symbol<Polar>`.

It has the same dependency rules as `avenger-chart-cartesian`.

### `avenger-chart`

Remain the high-level facade and core-owned layout/runtime engine.

Initial candidates:

- `Plot`, `CompiledPlot`, plot compilation, plot specs that are not pure shared
  config data, and facade prelude re-exports.
- Built-in containers: facet, concat, partition, child-frame placement,
  domain/guide coordination, layout refinement, and container debug overlays.
- Layout solvers and runtime evaluation loops.
- WGPU/app/canvas integration and examples/tests that exercise the full stack.

Facet/concat extraction is explicitly out of scope.

Current extraction state:

- The facade still owns the render-time `CompiledMark` trait and all
  core-owned layout/container marks. This is intentional for now: the current
  split has moved mark metadata/planning contracts down to core without trying
  to externalize layout runtime behavior.

## Current Dependency Pressure Points

The current single-crate module graph is not yet ready for folder-to-crate
movement. The key cycles and leaks found in the code are:

- `marks` depends on `legend` through `CompiledMark::preferred_legend_renderer`
  and `preferred_merged_legend_renderer`.
- `marks` depends on `scales` through default scale/range APIs, while `scales`
  depends on mark radius expressions through `ScaleDomain`.
- `marks::subplot` imports `Plot`, `CompiledPlot`, `concat`, `facet`, and
  `cartesian`, so `Subplot` is not a neutral mark crate type yet.
- `channel` value/config code stores scale and legend config directly and its
  fluent traits import both scale builders and legend builders.
- `GuideSharingContext` no longer imports concrete facet/container runtime
  types directly, but the guide traits still live in the top-level crate until
  the remaining guide/runtime signatures move to core.
- `coords::CoordinateSystemTransform::measure` imports top-level rendering,
  mark, scale, and layout runtime types. That signature needs a core-owned
  request/view type before coordinate crates can stand alone.
- `layout::types` imports `AxisPosition`, `LegendPosition`, and `FacetAxis`.
  These side/axis concepts need a core home or neutral replacements.
- `plot::plot` imports Cartesian, Polar, Legend, and layout-specific enums for
  `IntoExpr` impls and builder ergonomics. Those impls need to move to the
  owning crates after `IntoExpr` has a lower-level home.
- `AvengerChartError` has been reduced to core-compatible guide, scale,
  scenegraph, DataFusion/Arrow, layout, coordinate, and channel-resolution
  errors. Runtime app/WGPU/image errors are mapped explicitly at top-level call
  sites instead of being core error variants.

## Preliminary Refactors Before Any Crate Move

These chunks should be completed inside the existing `avenger-chart` crate
before creating new crates. Each chunk should preserve public facade behavior
and keep tests green.

Migration discipline:

- Prefer moving the real implementation to its future owner as soon as a type
  or module enters a split boundary.
- Do not create long-lived duplicate modules or broad mirror namespaces in the
  hope of reconciling them later.
- Use compatibility shims only where existing public paths must continue to
  work. Those shims should be tiny `pub use` modules with no independent
  behavior.
- Update internal imports toward the new owner in the same chunk whenever doing
  so is low-risk and mechanical.
- Keep the phase 1 staging namespaces as temporary wayfinding only; they should
  shrink as real modules move under their future owners.

1. Create internal future-boundary modules. Implemented in phase 1.
   Add internal modules or re-export namespaces that mirror the future crates,
   without changing behavior. This makes later file movement mechanical and
   gives imports a target shape:
   `chart_core`, `chart_marks`, `chart_scales`, `chart_legend`,
   `chart_cartesian`, and `chart_polar`. These are currently `pub(crate)`
   staging namespaces in `avenger-chart/src`, not new public API.

2. Move shared side/spec/value types to the future core boundary.
   Centralize `AxisPosition`, `LegendPosition`, `LegendOrientation`,
   `ScaleSharing`, `Maybe`, `Param`, `LayoutBounds`, `Size2D`, `EdgeSlabs`,
   `OverflowSide`, `RadiusExpression`, and serialization wrappers behind the
   future core namespace. Keep facade re-exports unchanged.

   Progress:

   - `maybe` and `param` now live under `chart_core`. `crate::maybe` and
     `crate::param` are compatibility shims only, and internal imports have
     moved to the new owner.
   - `AxisPosition` now lives under `chart_core`. `cartesian::axis::AxisPosition`
     and `cartesian::AxisPosition` remain compatibility re-exports for existing
     public paths.
   - `LegendPosition` and `LegendOrientation` now live under `chart_core`.
     `legend::LegendPosition`, `legend::LegendOrientation`, and prelude exports
     remain compatibility re-exports for existing public paths.
   - `ScaleSharing` now lives under `chart_core`.
     `channel::config_traits::ScaleSharing` remains a compatibility re-export,
     and internal imports have moved to the new owner.
   - `RadiusExpression` now lives in the real `avenger-chart-core` crate.
     `chart_core`, `marks::RadiusExpression`, and prelude exports remain
     compatibility re-exports, and scale/domain code no longer imports it
     through `marks`.
   - `ScaleRange`, `ScaleDomain`, `ScaleDefaultDomain`, `DomainExpr`, and
     `ResolvedDomain` now live in the real `avenger-chart-core` crate.
     `scales::*` paths remain compatibility re-exports.
   - The `ScaleSpec` trait and built-in scale marker types now live in the real
     `avenger-chart-core` crate. `scales::ScaleSpec`, `scales::Auto`,
     `scales::Linear`, and the other marker-type paths remain compatibility
     re-exports for existing public APIs and external custom scale impls.
   - `ScaleConfigSpec`, the owned scale configuration payload, now lives in the
     real `avenger-chart-core` crate. `Scale<S>` remains in the scales layer as
     the typed authoring/runtime wrapper around that payload, and
     `ChannelValue` plus plot-level scale overrides now store the core payload
     directly.
   - `ChannelValue` and `ConditionalValue` now live in the real
     `avenger-chart-core` crate. The public `avenger_chart::channel::value::*`,
     `avenger_chart::channel::*`, `avenger_chart::marks::*`, and prelude paths
     remain compatibility re-exports.
   - The object-safe `Axis` customization trait now lives in the real
     `avenger-chart-core` crate. `avenger_chart::axis::Axis` remains a
     compatibility re-export for existing coordinate-system implementations.
   - `ChannelDescriptor`, `ChannelDefault`, `BaseChannelName`, and the shared
     trailing-number stripping helper now live in the real
     `avenger-chart-core` crate. Existing `channel::*`, `channel::value::*`,
     and `marks::*` public paths remain compatibility re-exports.
   - `Legend` spec data now lives in the real `avenger-chart-core` crate.
     `legend::Legend` remains a compatibility re-export, while legend builders,
     renderer traits, and renderer implementations remain in `legend`.
   - `OverflowSpaceRequirement` and `MeasurementResult` now live under
     `chart_core`. `guide::*` and `coords::OverflowSpaceRequirement` remain
     compatibility re-exports for existing paths.
   - `FacetAxis` now lives under `chart_core`. `coords::FacetAxis` remains a
     compatibility re-export for existing paths.
   - `LayoutBounds`, `Size2D`, `EdgeSlabs`, `OverflowSide`,
     `FrameAllocation`, `FrameDemand`, `FrameLayout`, and related frame sizing
     types now live under `chart_core`. `layout::*` remains a compatibility
     re-export, while layout-private grid component metadata remains in
     `layout::types`.
   - `CoordinatedOverflow` and `CoordinatedLayout` now live in the real
     `avenger-chart-core` crate. `coords::*` remains a compatibility re-export
     for existing layout and facet code while the coordinate measurement trait
     boundary is narrowed.
   - `Maybe<T>` and the `MaybeOptionalExpr` serde adapter now live in the real
     `avenger-chart-core` crate.
   - Core-safe serialization wrappers now live in the real
     `avenger-chart-core` crate: `SerializableExpr`, `SerializableScalar`,
     `SerializableScalarMap`, `SerializableNestedScalarMap`, and
     `SerializableDataType`. The public `avenger_chart::serialization::*` path
     remains a compatibility re-export.
   - The chart-specific `Expr -> LogicalExprNode` conversion remains in
     `avenger-chart::serialization::LogicalExprNodeExt` because it uses the
     chart scale extension codec. Chart-local call sites now explicitly convert
     expressions through that helper before wrapping them as `SerializableExpr`.

   Do not duplicate the implementations.

3. Split channel values from scale/legend fluent extensions.
   Keep `ChannelValue`, `ConditionalValue`, `ChannelDescriptor`, and base
   conditional/no-scale methods in core-compatible channel modules. Move
   `.scale(...)` and `.scale_with(...)` into a scales extension trait. Move
   `.legend(...)` into a legend extension trait. Re-export these trait sets
   from `avenger_chart::prelude::*` so chart-author ergonomics do not change.

   Progress:

   - `ChannelConfig` now owns the core channel operations only: value access,
     conditional branches, and `no_scale`.
   - `.scale(...)`, `.scale_with(...)`, `with_scale_sharing(...)`,
     `share_scale()`, and `free_scale()` now live on
     `scales::ScaleChannelConfig`.
   - `.legend(...)` and `no_legend()` now live on `legend::LegendableChannel`.
     `channel::LegendableChannel` remains a compatibility re-export.
   - The prelude re-exports `ChannelConfig`, `ScaleChannelConfig`,
     `ScaleChannelValue`, `LegendableChannel`, and `LegendableChannelValue` so
     ordinary chart-author imports continue to provide the fluent methods.
   - `ChannelDescriptor`, `ChannelDefault`, `BaseChannelName`, and shared
     channel-name normalization now live in the real `avenger-chart-core`
     crate.
   - `ChannelValue` and `ConditionalValue` now live in the real
     `avenger-chart-core` crate. `avenger-chart/src/channel/value.rs` is a
     compatibility re-export plus a chart-only test for the scale extension.
   - Raw `ChannelValue` `.scale(...)` and `.scale_with(...)` methods now live
     on `scales::ScaleChannelValue`; raw value `.legend(...)` and
     `.no_legend()` methods now live on `legend::LegendableChannelValue`.
     `ChannelValue` itself keeps only core data access, expression/domain
     helpers, scale-name/band/no-scale helpers, and core spec storage.
   - Core `ChannelValue` expression serialization uses DataFusion's default
     logical-expression codec, matching the other moved core spec/value types.
     The chart-specific `AvengerChartExtensionCodec` remains in
     `avenger-chart::serialization` for scale-UDF-bearing expressions.
   - `impl_mark_base!` and `define_common_mark_channels!` now live in
     `avenger-chart-core`. They only depend on core mark state, data context,
     and channel configuration contracts. `avenger-chart` re-exports the macros
     for compatibility.
   - `GenericPositionConfig<A>` now lives in the real `avenger-chart-core`
     crate next to the `PositionConfig` trait. The old
     `avenger_chart::channel::GenericPositionConfig` path is a compatibility
     re-export. Scale-specific fluent methods remain extension-trait behavior
     owned by `avenger-chart-scales`.

4. Decouple legend selection from `CompiledMark`.
   Replace direct `LegendRenderer` returns on `CompiledMark` with core legend
   capability descriptors, or move only the object-safe `LegendRenderer` trait
   to core. The descriptor approach best matches the target graph because
   `avenger-chart-legend` remains the owner of renderer implementations.

   Progress:

   - `CompiledMark` now returns `chart_core::LegendRendererKind` from
     `preferred_legend_renderer_kind(...)` instead of returning
     `Arc<dyn LegendRenderer>`.
   - Concrete mark implementations no longer import legend renderer
     implementations. They select among core renderer descriptors such as
     `Symbol`, `Line`, `Rect`, and `Colorbar`.
   - The legend layer owns descriptor-to-renderer instantiation through
     `legend::renderer_for_kind(...)`, and merged legend planning checks merge
     support on the resolved concrete renderer.

5. Decouple mark scale preferences from concrete chart-scale modules.
   Move `RadiusExpression`, resolved-domain shape, and scale preference
   descriptors to core. Make scale building consume those descriptors instead
   of importing concrete mark modules. Keep concrete scale construction and UDFs
   in the future scales boundary.

   Progress:

   - `ScaleRange` now lives in the real `avenger-chart-core` crate;
     `scales::ScaleRange` remains a compatibility re-export.
   - `ScaleDomain`, `ScaleDefaultDomain`, and `DomainExpr` now live in the real
     `avenger-chart-core` crate; `scales::*` paths remain compatibility
     re-exports. Domain inference, data caching, and configured-scale creation
     remain in the scales layer.
   - `ResolvedDomain` now lives in the real `avenger-chart-core` crate;
     `scales::ResolvedDomain` remains a compatibility re-export.
   - Mark `preferred_scale_type(...)` methods now return
     `chart_core::ScaleTypePreference` instead of `Box<dyn ScaleSpec>`.
   - Core maps `ScaleTypePreference` back to concrete `ScaleSpec`
     implementations with `scale_spec_for_preference(...)`; the scales layer
     consumes those descriptors when constructing configured scales.
   - Mark fallback scale inference now returns core scale type descriptors via
     `default_scale_type_for_data_type(...)`.
   - Shared DataFusion/scalar utility traits and helpers now live in the real
     `avenger-chart-core` crate. Scale internals import
     `DataFrameChartHelpers`, `ScalarValueHelpers`, `eval_to_scalars`,
     `params_to_datafusion`, and `scalar_to_scalar_value` through the core
     boundary rather than through `avenger-chart::utils`.

6. Make `Subplot` neutral.
   Introduce core child-plot traits, for example `ChildPlotSpec` and
   `CompiledChildPlot`, implemented by the current `Plot<C>` and
   `CompiledPlot`. Change `Subplot` and `CompiledSubplotPayload` to depend on
   those traits rather than directly on `Plot` and `CompiledPlot`.

   Progress:

   - `Subplot` now stores a `SubplotChildPlotSpec` trait object instead of a
     concrete `Plot<C>`. The `Plot<C>` implementation lives with `plot`, so the
     neutral subplot mark no longer imports the concrete plot builder type.
   - The neutral `Subplot<OuterC>` type and shared `compile_subplot_payload`
     helper are now bounded only by `CoordinateSystemCore`. The
     `SubplotContainerCoordinateSystem` compile hook still requires the full
     top-level coordinate trait because it remains the layout/runtime extension
     boundary for positioned child plots.
   - `CompiledSubplotPayload` still stores `Arc<CompiledPlot>`. Moving this to a
     compiled-child trait should wait until the render/measurement context
     signatures are reduced; otherwise the trait would merely expose the current
     top-level layout runtime wholesale.

7. Move built-in subplot implementations out of `marks::subplot`.
   Keep only the neutral `Subplot`, payload, and compile hook in the mark/core
   boundary. Move `CompiledConcatSubplot` and H/V concat implementations to
   `concat`. Move `CompiledCartesianSubplot`, Cartesian builder methods, and
   Cartesian compile implementation to `cartesian::positioned_subplot`. Keep
   facet row/column implementations in `facet::marks`.

   Progress:

   - Concat now owns `CompiledConcatSubplot`, the H/V
     `SubplotContainerCoordinateSystem` impls, and the concat
     `compiled_subplot(...)` downcast helper.
   - Cartesian positioned subplot support now lives with
     `cartesian::positioned_subplot`, including the `Subplot<Cartesian>`
     builder methods, `SubplotContainerCoordinateSystem` impl,
     `CompiledCartesianSubplot`, and its render implementation.
   - `marks::subplot` now contains the neutral subplot mark, shared compiled
     payload, shared compile hook, and unit coverage only. Temporary
     compatibility re-exports remain for the old compiled subplot type paths.

8. Replace concrete guide sharing context with an opaque core view.
   `CoordinateGuide` should receive a core-owned `GuideSharingContext` that
   exposes only the queries guides need. Top-level facet/concat layout can back
   that view with `EvaluatedFacetTree` and child-frame paths internally, but
   coordinate crates should not import those concrete types. Move or delete
   stale `FacetDirection` terminology during this pass.

   Progress:

   - `GuideSharingContext` fields are now private, and Cartesian/facet guide
     code uses query methods instead of reaching into the facet tree or
     child-frame sharing path directly.
   - The unused `CoordinateGuide::facet_unifiable_channel(...)` hook and its
     `UnifiableChannelInfo` payload have been removed rather than carried into
     the split as dead facet-specific guide API.
   - `FacetDirection` now lives under `facet`; the generic `guide` module no
     longer defines or re-exports it.
   - `GuideUpdate` now lives in the real `avenger-chart-core` crate, with
     `guide::GuideUpdate` preserved as a compatibility re-export. This keeps a
     small guide-authoring contract available to external coordinate dogfood
     without depending on the top-level guide module.
   - The default axis/legend title helper
     `extract_channel_title_from_marks(...)` now lives in the real
     `avenger-chart-core` crate and works over `CompiledMarkCore`. The top-level
     `coords::*` path remains a compatibility re-export.
   - `CoordinateGuide::set_compiled_marks(...)` now accepts any compiled mark
     collection whose items implement `CompiledMarkCore`, rather than requiring
     top-level render-capable `CompiledMark` values. Guide setup can derive
     axis titles, sharing levels, and facet guide metadata from core mark
     metadata/downcast support while the render hook remains top-level.
   - `GuideContext` and `GuideOverflowPhase` now live in the real
     `avenger-chart-core` crate, with `guide::*` compatibility re-exports.
     The remaining guide split work is therefore focused on `CompiledGuide`,
     `CoordinateGuide`, and the facet-backed `GuideSharingContext`.
   - `GuideSharingContext` now stores a narrow `FacetGuideSharingView` trait
     object instead of a concrete `EvaluatedFacetTree`. Facet remains the
     provider of that view, but coordinate guides query guide-level visibility,
     title ownership, jagged-axis state, domain-sharing level, and effective
     edge indices through the context rather than reaching back into the facet
     tree.
   - `GuideSharingContext` now also stores a narrow
     `ChildFrameGuideSharingView` trait object instead of a concrete
     `ChildFrameSharingPath`. Child-frame containers remain owned by the
     top-level layout runtime, but coordinate guide ownership logic now depends
     on stable child-frame position/count/axis queries. The context still lives
     in the top-level guide module, so moving `CoordinateGuide` and
     `CompiledGuide` remains future work.

9. Reduce coordinate transform measurement signatures.
   Replace direct references to top-level `EvaluationContext`,
   `ComponentsMeasurement`, `ConfiguredScaleWithSpec`, and
   `Arc<dyn CompiledMark>` with core request/view types. Keep facet/concat
   runtime state in the top-level crate; the coordinate crates should only see
   the stable views needed for coordinate measurement.

   Progress:

   - `CoordinateSystemTransform::measure(...)` now accepts a single
     `CoordMeasureRequest` instead of a long argument list. The request still
     carries current in-crate runtime types, but it gives the split one owned
     surface to narrow into stable core views.
   - Only plot dimensions are public on `CoordMeasureRequest` for external
     coordinate implementations. Built-in container/facet measurement can still
     access scales, evaluation state, data, marks, and facet paths through
     crate-private accessors while those internals are being split.
   - The object-safe `CoordMeasurement` trait and `EmptyCoordMeasurement` now
     live in the real `avenger-chart-core` crate. The top-level chart crate
     keeps layout-only measurement behavior, such as facet band scale
     adjustment and child-frame container projection, as explicit dispatch over
     built-in measurement types instead of methods on the core trait.
   - Core-safe transform behavior now lives in the real
     `avenger-chart-core` crate as `CoordinateSystemTransformCore`: required
     channels, position transformation, default range bindings/ranges, and
     coordinate-specific scale options. The top-level
     `CoordinateSystemTransform` trait extends this core trait and keeps only
     chart-runtime measurement, chart-object cloning, and measured-padding
     hooks. This makes the future coordinate-crate transform surface real
     without moving facet/concat measurement runtime into core.
   - Core-safe coordinate metadata now lives in the real `avenger-chart-core`
     crate as `CoordinateSystemCore`. The top-level `CoordinateSystem` trait
     extends it and keeps only guide association plus serializable transform
     creation. The object-safe `Mark<C>` trait now depends on
     `CoordinateSystemCore`, so mark authoring no longer needs the full
     top-level coordinate/layout contract.
   - `ZeroDCoord` now lives in the real `avenger-chart-core` crate with its
     core-safe coordinate metadata and transform implementation. The top-level
     `avenger_chart::zerod` module remains a compatibility adapter that adds
     the facade-owned guide/transform trait impls. External subplot-coordinate
     dogfood imports `ZeroDCoord` directly from core.

10. Split public/base evaluation context from internal layout state.
    The core `EvaluationContext` should contain theme, session context,
    params, and stable runtime options. Facet tree, scale-precompute stores,
    child-frame paths, refinement state, and debug snapshot capture should move
    into a top-level internal evaluation state layered around the core context.

    Progress:

    - The stable `theme`, `session_context`, and `params` fields now live in
      `chart_core::evaluation_context::EvaluationContext`.
    - `render::EvaluationContext` wraps that core context and continues to own
      facet/layout runtime state. It implements `Deref` to the core context as
      a temporary compatibility bridge while in-crate callers are migrated.
    - Mark data preparation now accepts the core evaluation context directly,
      proving that shared mark data-query code no longer needs facet/layout
      runtime state.
    - Child-frame plot preparation now also takes the core context for
      scale/domain setup; only the later measurement step still requires the
      full top-level render context.
    - Scale-builder construction now accepts the core evaluation context
      instead of separate `SessionContext` and params arguments, so shared
      scale/domain preparation depends on the base context shape.
    - Mark default-channel lookup now has a core-context helper. Scale/domain
      and legend planning use it directly, so they no longer create dummy
      `RenderContext` values only to read theme defaults.
    - Theme/params helpers for `query_theme`, `font_size`, and mark defaults
      now live on the core evaluation context; `RenderContext` delegates to
      those methods instead of owning the behavior.
    - `ThemeContext`, `Theme`, `ThemeValue`, theme parsing/evaluation, and
      color helpers now live in the real `avenger-chart-core` crate and are
      re-exported through the existing `avenger_chart::theme::*` and
      `avenger_chart::color::*` compatibility paths.
    - The base `EvaluationContext` implementation now lives in the real
      `avenger-chart-core` crate. The top-level `render::EvaluationContext`
      remains the layout/runtime wrapper for facet trees, child-frame paths,
      refinement state, and debug capture.
   - `MarkRenderContext` now lives in the real `avenger-chart-core` crate as a
     narrow mark-facing render view over the base evaluation context plus plot
     dimensions. Built-in ordinary mark renderers now use this core view for
     default-channel lookup and channel coercion while retaining the top-level
     `RenderContext` only for coordinate transforms and layout/runtime views.
     Moving the object-safe `CompiledMark` contract itself should wait until
     coordinate transform measurement and render-time coordinate views have the
     same core-owned shape.
   - Mark rendering now receives a borrowed `CoordinateSystemTransformCore`
     rather than an owned top-level `CoordinateSystemTransform` trait object.
     Ordinary built-in marks and external custom-mark dogfood no longer need
     the chart-runtime transform trait for render-time position projection.
     The top-level transform trait remains the serializable/measurement layer
     used by plot compilation and layout-owned coordinates.
   - Shared expression-evaluation helpers now live in the real
     `avenger-chart-core` crate. Guide, title, layout, legend, and render
     call sites import `evaluate_*_expr` helpers through the core boundary,
     leaving `plot::compiled::expr_eval` as a compatibility shim.
   - Strict color-string parsing and color-to-RGBA helpers now live in the
     real `avenger-chart-core` crate. Guide, legend renderer, and mark-data
     call sites import them through `chart_core::color`, while
     `avenger_chart::utils::*` remains a compatibility path.
   - Base mark channel coercion helpers now live in the real
     `avenger-chart-core` crate under `mark_channel_coercion`. The old
     `avenger_chart::marks::util::*` path re-exports the raw helpers and keeps
     only the `CompiledMark` default-aware wrappers top-level for now.

11. Move `IntoExpr` to the future core boundary.
    Once `IntoExpr` is low-level, `AxisPosition`, `LegendPosition`,
    `PolarAxisType`, and `PolarDirection` impls can live beside their owning
    types instead of making `plot` import coordinate crates.

    Progress:

    - `IntoExpr` now lives under `chart_core`, with `plot::IntoExpr` preserved
      as a compatibility re-export.
    - Core-owned implementations for primitive values, `Param`, `AxisPosition`,
      `LegendPosition`, and `LegendOrientation` live with `chart_core`.
    - Owner-specific implementations for `PolarAxisType`, `PolarDirection`,
      `TitleSpan`, and `TitleAlign` now live beside those owning types instead
      of in `plot::plot`.

12. Split `AvengerChartError`.
    Create a core chart error that covers invalid arguments, serialization,
    DataFusion/Arrow, scenegraph, scale, and guide errors. Keep app, image, and
    WGPU integration out of core.

    Progress:

    - The real `AvengerChartError` enum now lives in the real
      `avenger-chart-core` crate with the core-compatible variants. The public
      `crate::error` and `chart_core::error` modules are compatibility
      re-exports.
    - `ChannelResolutionError` now lives in `avenger-chart-core`, which removes
      the final chart-local dependency from the core error enum.
    - Runtime-only `AvengerAppError`, `image::ImageError`, and
      `AvengerWgpuError` conversions are no longer trait impls because
      `AvengerChartError` is now owned by `avenger-chart-core`. Top-level
      runtime code maps those errors explicitly to `InternalError` at call
      sites.

## Crate Extraction Order

Only start these after the preliminary refactors above have made the intra-crate
boundaries boring.

1. Extract `avenger-chart-core`.
   Move the core-compatible modules first and make `avenger-chart` depend on the
   new crate while re-exporting the old paths. Do not move layout solvers or
   container runtime.

   Progress:

   - Created the real `avenger-chart-core` workspace crate.
   - Moved the first low-risk value modules into it: `Axis`, `AxisPosition`,
     `FacetAxis`, `IntoExpr`, shared frame/layout value types,
     `LegendPosition`, `LegendOrientation`, `LegendRendererKind`,
     `OverflowSpaceRequirement`, `MeasurementResult`, `Maybe`,
     `MaybeOptionalExpr`, `Param`, `RadiusExpression`, `ResolvedDomain`,
     `ScaleRange`, `ScaleDomain`, `ScaleDefaultDomain`, `DomainExpr`,
     `ScaleSharing`, `SharingLevel`, `CoordinationAxis`, `PositionConfig`,
     `ScaleRangeBinding`, plot-area range binding endpoint types, `ScaleSpec`,
     built-in scale marker types, `ScaleTypePreference`, mark-facing scale
     helper functions, `ChannelConfig`, common channel config structs,
     `ChannelDescriptor`, `ChannelDefault`,
     `BaseChannelName`, `ChannelValue`, `ConditionalValue`, `DataContext`,
     `CompiledDataContext`, `MarkState`, `CompiledMarkState`, `FacetStrategy`,
     `AvengerChartError`, `ChannelResolutionError`, `ThemeContext`, `Theme`,
     `ThemeValue`, `Legend`, color helpers, theme parsing/evaluation, the base
     `EvaluationContext`, shared expression-evaluation helpers, strict
     color-string parsing helpers, base mark channel coercion helpers, and core-safe
     expression/scalar/datatype/logical-plan serialization wrappers.
   - Moved the shared channel-title extraction helper into core and made it
     generic over `CompiledMarkCore`, so guide and legend title inference can
     use compiled mark metadata without depending on the top-level render trait.
   - Moved shared DataFusion/scalar helper traits and functions into core:
     `DataFrameChartHelpers`, `ExprHelpers`, `ScalarValueHelpers`,
     `ArrayRefHelpers`, `eval_to_scalars`, `simplify_to_scalar_sync`,
     `params_to_datafusion`, `scalar_to_scalar_value`, `contains_aggregate`,
     and `partition_expressions`. The old `avenger_chart::utils::*` helper
     paths are compatibility re-exports only; source imports have moved to the
     core boundary where doing so was mechanical.
   - Moved the first coordinate authoring trait into core:
     `CoordinateSystemCore` owns required position-channel metadata. The
     top-level `CoordinateSystem` trait remains the layout/runtime extension
     wrapper for guide association and transform creation.
   - Moved the first coordinate-extension value contracts into core:
     `PlotGeometry`, `PointGeometry`, `SubplotRect`, `SubplotGeometry`,
     `PaddingSpec`, the pure `BandPosition` value, and `ZeroDCoord`. The
     band-scale iterator remains in top-level layout because it depends on
     chart-layer configured scales.
   - `SerializableDataFrame` now lives in `avenger-chart-core` and is
     re-exported through `avenger_chart::serialization`. The chart-specific
     logical expression/plan conversion traits that use
     `AvengerChartExtensionCodec` remain in `avenger-chart` with the scale UDF
     codec.
   - `avenger-chart` now depends on `avenger-chart-core` and re-exports those
     moved items through the existing `chart_core` staging namespace, including
     thin compatibility submodules for old in-crate paths.
   - Theme-only parser dependencies (`cssparser`, `selectors`, and
     `precomputed-hash`) are now owned by `avenger-chart-core`, not the
     top-level facade crate.
   - The old custom legend renderer override field/methods were not moved into
     core. The split keeps renderer dispatch in the legend layer and uses core
     renderer descriptors instead of storing `LegendRenderer` trait objects in
     serializable spec data.

2. Extract `avenger-chart-marks`.
   Move built-in generic mark families. The facade keeps
   `avenger_chart::marks::*` and `avenger_chart::prelude::*` stable.

   Progress:

   - Created the real `avenger-chart-marks` workspace crate.
   - The initial neutral mark state/data types were promoted to
     `avenger-chart-core` after clarifying that custom marks should not need to
     depend on the built-in mark crate.
   - `avenger-chart-marks` owns the generic built-in `Line<C>`, `Rect<C>`, and
     `Symbol<C>` mark families, their common-channel builders, mark-specific
     default descriptors, and small shared helpers such as line dictionary
     partition keys and symbol legend-kind selection.
   - The old `avenger-chart::marks::{line,rect,symbol}` modules are
     compatibility re-export shims over `avenger-chart-marks`.
   - Cartesian and Polar now own coordinate-specific position-channel builder
     extension traits for those generic marks:
     `CartesianLinePositionChannels`, `CartesianRectPositionChannels`,
     `CartesianSymbolPositionChannels`, and `PolarSymbolPositionChannels`.
     These are re-exported from the facade prelude so normal chart-author
     ergonomics stay intact while the ownership boundary becomes real.
   - The next real mark move is neutral `Subplot`, but it should wait until the
     compiled child payload and coordinate subplot hook no longer pull top-level
     plot/layout runtime types into the mark crate.
   - `avenger-chart` now depends on `avenger-chart-marks`, and
     `avenger-chart/src/marks/data_context.rs`,
     `avenger-chart/src/marks/compiled_data_context.rs`, and
     `avenger-chart/src/marks/facet_strategy.rs`, and
     `avenger-chart/src/marks/state.rs` are compatibility re-export shims.
   - `CompiledDataContext` uses core `ChannelValue`, `SerializableDataFrame`,
     and the core default logical-plan codec. The chart-specific scale-UDF
     codec remains in `avenger-chart`.
   - The external custom-mark dogfood now imports the already-moved custom mark
     state/data/channel contracts and base/common-channel macros directly from
     `avenger-chart-core`. Its `HexBin<C>` mark implementation is generic over
     `CoordinateSystemCore`, proving mark authoring no longer requires the full
     top-level coordinate/layout trait. It still imports the object-safe
     `Mark` / `CompiledMark` traits and full render boundary from the top-level
     facade, which keeps the remaining split gap explicit.

3. Extract `avenger-chart-scales`.
   Move the remaining chart-layer scale user configuration, builders,
   inference, UDF/codec, and scale channel extension traits. Keep
   `avenger-scales` as the lower-level runtime scale dependency and keep the
   `ScaleSpec` marker descriptors in core.

   Progress:

   - Created the real `avenger-chart-scales` workspace crate.
   - Moved the scale authoring surface into the new crate:
     `Scale<S>`, `ScaleChannelConfig`, `ScaleChannelValue`, domain/range/spec
     compatibility modules, default range helpers, domain extent values,
     configured-scale DataFusion/legend extension traits, scale UDF creation,
     `AvengerChartExtensionCodec`, and the chart-specific logical expr/plan
     serialization traits that use that codec.
   - Moved the real `ScaleBuilder`, `ChannelScaleData`, and `DataExtents`
     implementation into `avenger-chart-scales`. The builder now receives a
     `DefaultScaleRangeResolver` callback instead of importing top-level
     `CompiledMark`, and plot scale override specs now live in the scale crate
     as `PlotScaleSpec`.
   - `avenger-chart` now depends on `avenger-chart-scales` and re-exports the
     moved surface through `avenger_chart::scales::*`, including the old
     submodule paths such as `scales::spec`, `scales::domain_extent`, and
     `scales::udf`.
   - Prerequisite utility ownership is in place: scale-domain inference and
     configured-scale creation now use core-owned DataFusion/scalar helpers
     instead of the top-level `utils` module.
   - The remaining top-level scale orchestration is
     `build_scale_builder_from_marks`, which still walks compiled marks,
     resolves channel data, and provides mark-specific default ranges through
     the callback. This is the next scale-related boundary to revisit after
     object-safe mark contracts and render/measurement views move toward core.
   - The external custom-scale dogfood now imports `Scale` and `ScaleSpec`
     directly from `avenger-chart-scales`, while the plot-integration test still
     uses the top-level facade to prove the two paths compose.

4. Extract `avenger-chart-legend`.
   Move legend builders, renderer implementations, and legend planning. Ensure
   legend code consumes mark/scale descriptors rather than reaching into
   top-level plot internals.

   Progress:

   - Created the real `avenger-chart-legend` workspace crate.
   - Moved legend authoring builders and channel extension traits into it:
     typed legend builders, `LegendBuilder`, `LegendableChannel`, and
     `LegendableChannelValue`.
   - The channel-specific `LegendableChannel` impls moved with the trait into
     `avenger-chart-legend`, which keeps the orphan-rule ownership correct now
     that channel config types live in core.
   - `avenger-chart` now depends on `avenger-chart-legend` and re-exports the
     moved authoring surface through `avenger_chart::legend::*` and the
     prelude.
   - Moved legend renderer implementations and dispatch into
     `avenger-chart-legend`: `LegendRenderer`, `LegendChannel`, `MergeKey`,
     `renderer_for_kind`, `CompiledSymbolLegend`, `CompiledLineLegend`,
     `CompiledRectLegend`, and `CompiledColorbar`. The old
     `avenger_chart::legend::renderer::*` path is now a compatibility
     re-export.
   - Moved legend size measurement into `avenger-chart-legend` as
     `measure_legend_size_with_channels`. The old
     `avenger_chart::layout::legend::*` path is now a compatibility re-export.
   - Plot legend planning remains top-level until its mark-walking and layout
     placement dependencies are narrowed.
   - The external legend dogfood now imports `LegendBuilder`,
     `LegendableChannel`, `LegendableChannelValue`, renderer dispatch, and
     concrete renderer types directly from `avenger-chart-legend`, with
     channel values/configs imported directly from `avenger-chart-core`.

5. Extract `avenger-chart-cartesian`.
   Move Cartesian coordinate, axes, guides, channels, coordinate-specific mark
   impls, and Cartesian positioned subplot support.

   Progress:

   - Created the real `avenger-chart-cartesian` workspace crate.
   - Moved the real `Cartesian` coordinate type plus its core-safe
     `CoordinateSystemCore` and `CoordinateSystemTransformCore` implementation
     into `avenger-chart-cartesian`. The top-level
     `avenger_chart::cartesian::coord` module is now a compatibility/adapter
     module that re-exports the moved type and implements the still-facade-owned
     `CoordinateSystem` and `CoordinateSystemTransform` runtime traits so
     Cartesian positioned subplot measurement remains in core-owned layout for
     now.
   - External custom mark/scale dogfood imports the `Cartesian` type directly
     from `avenger-chart-cartesian` while still using the top-level facade for
     `Plot`, `Mark`, Cartesian position-channel extension traits, and render
     integration that have not moved yet.
   - The external custom-coordinate dogfood now imports already-moved core and
     scale authoring contracts directly from `avenger-chart-core` and
     `avenger-chart-scales`: axis/channel/config/state/data/geometry types,
     `CoordMeasurement`, `CoordinateSystemCore`,
     `CoordinateSystemTransformCore`, mark-constructor macros, and scale
     builders. It still imports the top-level coordinate runtime trait, guide
     traits, `CompiledMark`, `Mark`, `RenderContext`, and the `Subplot` compile
     hook from the top-level facade, making the remaining coordinate-crate
     extraction boundary explicit.
   - The external subplot-coordinate dogfood now imports
     `CompiledDataContext`, `CompiledMarkState`, channel descriptors, geometry,
     error types, `CoordMeasurement`, `CoordinateSystemCore`,
     `CoordinateSystemTransformCore`, and `GuideUpdate` directly from
     `avenger-chart-core` while still using the top-level facade for `Subplot`,
     `CompiledSubplotPayload`,
     `SubplotContainerCoordinateSystem`, compiled guide traits, coordinate
     traits, and render context. This confirms the narrow extension goal is
     still alive while the runtime trait boundary remains to be moved.
   - External coordinate dogfood now implements the generic
     `CoordinateGuide::set_compiled_marks<M: CompiledMarkCore>(...)` hook,
     proving guide setup no longer requires the top-level compiled mark render
     trait even though `CoordinateGuide` itself has not moved to core yet.
   - `CartesianAxis` and `CartesianPositionConfig` now live in the real
     `avenger-chart-cartesian` crate. The top-level
     `avenger_chart::cartesian::{axis,channels}` modules are compatibility
     shims plus the still-facade-owned, layout-aware Cartesian axis evaluation
     helper. External custom-mark dogfood imports `CartesianPositionConfig`
     directly from `avenger-chart-cartesian`.
   - `CartesianOptions`, the pure coordinate guide/options spec, now lives in
     `avenger-chart-cartesian`. The top-level Cartesian guide module still owns
     guide measurement/rendering because it depends on facet/child-frame
     sharing and layout runtime state.
   - Cartesian position-channel builder extension traits for the generic
     `Line`, `Rect`, and `Symbol` marks now live in
     `avenger-chart-cartesian`. The top-level Cartesian mark modules keep the
     render/compile implementations for now and re-export the moved traits for
     compatibility.

6. Extract `avenger-chart-polar`.
   Move Polar coordinate, axes, guides, channels, and Polar mark impls.

   Progress:

   - Created the real `avenger-chart-polar` workspace crate.
   - Moved the real `Polar` coordinate type plus its core-safe
     `CoordinateSystemCore` and `CoordinateSystemTransformCore` implementation
     into `avenger-chart-polar`. The top-level `avenger_chart::polar::coord`
     module is now a compatibility/adapter module that re-exports the moved
     type and implements the still-facade-owned `CoordinateSystem` and
     `CoordinateSystemTransform` runtime traits.
   - `PolarAxis`, `PolarAxisType`, `PolarDirection`, and
     `PolarPositionConfig` now live in the real `avenger-chart-polar` crate.
     The top-level `avenger_chart::polar::{axis,channels}` modules are
     compatibility shims plus the still-facade-owned Polar guide/axis
     evaluation glue.
   - `PolarOptions`, the pure coordinate guide/options spec, now lives in
     `avenger-chart-polar`. The top-level Polar guide module still owns guide
     measurement/rendering until the guide runtime boundary moves.
   - `PolarSymbolPositionChannels` now lives in `avenger-chart-polar`. The
     top-level Polar symbol module keeps the render/compile implementation for
     now and re-exports the moved trait for compatibility.

7. Shrink the top-level `avenger-chart` crate.
   Leave `Plot`, `CompiledPlot`, facet, concat, partition, layout solvers,
   runtime evaluation, WGPU/app/canvas integration, and facade re-exports.

## Testing And Validation Strategy

Use tiered validation so the split does not stall on full release suites after
every mechanical move. Compilation is the primary signal for most boundary
refactors; add focused tests only where the changed boundary has runtime
behavior, and reserve broad test/clippy runs for milestone boundaries.

Default per-phase checks for mechanical moves:

```bash
cargo check -p avenger-chart --all-targets
cargo fmt --all --check
git diff --check
```

Add this when a public or external dogfood boundary changes:

```bash
cargo check --manifest-path avenger-chart-external-test/Cargo.toml --all-targets
```

Run focused tests based on the touched boundary:

```bash
cargo test -p avenger-chart --lib subplot -- --nocapture
cargo test -p avenger-chart --lib facet -- --nocapture
cargo test -p avenger-chart --lib concat -- --nocapture
cargo test -p avenger-chart --test visual_regression cartesian_positioned -- --nocapture
```

Run the full release/clippy suite at milestone boundaries, before each actual
crate extraction, and before pushing/reviewing a batch:

```bash
cargo test -p avenger-chart --release --lib -- --nocapture
cargo test --release --manifest-path avenger-chart-external-test/Cargo.toml -- --nocapture
cargo clippy --release -p avenger-chart --all-targets
cargo clippy --release -p avenger-chart-external-test --all-targets
```

After each actual crate extraction, add a focused external-test module proving
the intended public dependency direction:

- External custom mark crate uses `avenger-chart-core` for the mark contracts,
  without depending on the built-in mark crate.
- External scale crate uses `avenger-chart-core` plus `avenger-chart-scales`.
- External legend authoring code uses `avenger-chart-core` plus
  `avenger-chart-legend`, without depending on the top-level facade for legend
  builders or channel extension traits.
- External coordinate crate uses core, scales, and legend contracts as needed
  to implement a minimal coordinate system and
  `SubplotContainerCoordinateSystem`, without depending on Cartesian or Polar.
- Top-level `avenger-chart` still supports the old facade imports.

## Open Decisions

- Whether the object-safe `LegendRenderer` trait moves to core or remains in
  `avenger-chart-legend` behind descriptor-based dispatch. Descriptor-based
  dispatch better preserves the requested graph.
- Whether the current core-owned theme/color engine eventually becomes a
  dedicated lower crate. Keeping it in core is the lowest-risk first split
  because marks, scales, legends, and guides all need it.
- Whether `ZeroDCoord` belongs in core or in a tiny coordinate crate. Keeping it
  in core avoids creating an extra crate before the first split.
- `CompiledMark` and `CompiledGuide` should move to core once their signatures
  use core render/measurement views. `CompiledMarkState` already lives in core.
  Moving the object-safe runtime traits is what lets external custom marks and
  external coordinate systems avoid depending on built-in mark or coordinate
  crates.
