# Foundation API migration

## Color and formatting

Import `ColorOrGradient`, `Gradient`, `GradientStop`, `LinearGradient`, and `RadialGradient` from `avenger_color`. These types previously lived in `avenger_common::types`. Scene marks, guides, scales, and Vega imports use the shared definitions.

`avenger-color` also provides color parsing, conversion, interpolation, weighted Oklab mixing, and contrast calculations. `avenger-format-number` and `avenger-format-datetime` provide standalone format parsing and locale resolution without dependencies on rendering or chart libraries.

Run the color example from the repository root:

```sh
cargo run --release -p avenger-color --example interpolation -- docs/images/color-interpolation.png
```

The image shows sRGB, HSL, Lab, and Oklab interpolation from top to bottom.

![Color interpolation in four spaces](images/color-interpolation.png)

Use `--release` for development runs and tests. The `release` profile favors iteration, while `release-perf` retains optimized distribution settings. Geometry consumers resolve a pinned rstar revision without relying on this repository's lockfile.

## Share one text engine

`avenger-text` now uses the portable Typst label engine on native and browser targets. Remove the old `cosmic-text` feature and global font-registration calls.

Create a `TextEngine` with `FontResolutionOptions` when an application needs explicit fonts. Pass clones to `CanvasConfig::text_engine`, `AvengerApp::try_new_with_text_engine`, and `SceneGraphRTree::from_scene_graph_with_text_engine` to keep rendering and picking consistent.

Set `SceneTextMark::text_syntax` to `TextSyntaxMode::TypstMarkup` for styled labels and inline math. Plain labels use grapheme-safe ellipsis under a width limit. Markup is compiled intact and clipped after layout. Locale options and read-only parameters travel with each text mark.

```sh
cargo run --release -p avenger-wgpu --example rich_text -- docs/images/rich-text.png
```

![Rich labels and width limits](images/rich-text.png)

## Text leaders

Text marks keep their target in `x` and `y`. The `dx` and `dy` channels move the label in canvas coordinates. Leader channels control the connecting path, stroke, and arrowhead. Rendering and picking use the same leader geometry and configured text engine.

A leader stops at the target radius and the padded label boundary. When these boundaries overlap, the leader is omitted.

```sh
cargo run --release -p avenger-wgpu --example annotation_leaders -- docs/images/annotation-leaders.png
```

![Curved annotation leader](images/annotation-leaders.png)

## Geometric selections

`SceneGraphRTree::query_shape` accepts rectangle, circle, and polygon regions. Policies test envelopes, geometry intersection, complete geometry containment, mark anchors, or geometric centroids. Intersection and containment account for stroke width using the same distance model as point picking. Region boundaries are included.

`AnchorInside` uses each instance's explicit placement point, including group translations and text label offsets. Whole line, area, and trail marks have no single anchor and do not match this policy. An arc center or a path origin can lie outside its geometry, so anchor queries inspect all instances.

Rendering and picking use `SceneDisplayList` for root marks, inherited z-index, and document order. Set `interactive` to `false` to exclude a mark from scene picking while retaining its rendering and layout bounds. Query results use scene-path and instance-index order.

## Additional symbol shapes

`SymbolShape::from_vega_str` accepts `star`, `wye`, `pentagon`, and `cushion`. The name `concave-square` is an alias for `cushion`. These names use the existing path rendering and picking machinery.

Star and wye use the area normalization from [D3 star](https://github.com/d3/d3-shape/blob/main/src/symbol/star.js) and [D3 wye](https://github.com/d3/d3-shape/blob/main/src/symbol/wye.js), so their filled area equals `size`. Pentagon has circumradius `sqrt(size) / 2`. Cushion fits a square of width `sqrt(size)` with quadratic sides curved inward.

```sh
cargo run --release -p avenger-wgpu --example extra_symbols -- docs/images/extra-symbols.png
```

![Additional symbols at sizes 64, 400, and 1600](images/extra-symbols.png)

## Render into an application-owned target

`AvengerWgpuRenderer` accepts an existing WGPU device and queue. `AvengerRenderTarget` describes the destination texture view, load operation, and optional multisample resolve target. `OffscreenTarget` allocates a texture for ordinary offscreen rendering. Applications own presentation and scheduling. The workspace uses WGPU 27.

Window, PNG, and browser canvases use the same renderer. A resize rebuilds dimension-dependent scene state while preserving logical coordinates, clipping, and the configured text engine. The window canvas retains its supported multisampling default.

## Configure scales

`ConfiguredScale` validates options and normalizes domains with typed context. Scale domain access and number formatting preserve binary64 precision where supplied by the source data. Scale formatters use the shared number and datetime libraries with D3 formats and resolved locales. Arrow consumers use version 58.

## Configure axes

Numeric and temporal axes accept explicit formats, locale/timezone context, and tick spacing. `AxisConfig` supplies optional styling and label-template fields; use `..Default::default()` when setting a subset. The text-engine entry points use the caller's fonts for measurement. Labels retain their clearance from tick ends when `tick_length` changes.

## Configure legends

Discrete scales expose labels and representative values through `legend_entries`. Legend renderers accept styling and explicit text engines. Itemized output returns scene paths for discrete items and continuous surfaces so callers can attach interactions. Existing scene-only entry points return the rendered group.

## Nested categorical bands

`NestedBandScale` maps struct-valued category paths to bands. Free nesting allocates space for the children present in each parent. Shared nesting aligns child categories across parents. The matching axis places labels at each hierarchy level.

```sh
cargo run --release -p wgpu-scales --bin nested_bands -- docs/images/nested-bands.png
```

![Revenue by region and channel](images/nested-bands.png)

## Fit domains around marker extents

`compute_domain_from_data_with_padding_linear` accepts data values, lower and upper extents in screen units, and a positive range width in the same units. It returns an increasing linear domain that fits the marker extents. Inputs must be finite, and extents must be nonnegative. Infeasible geometry and unrepresentable solutions return errors.

The standalone solver does not run automatically during scale configuration. Call it when fitting a linear scale, then use the returned domain. Coincident values use a conventional positive span because no unique minimum width exists. Empty data returns `(0, 1)`.

## Fill marks with patterns

Set `fill_pattern` on filled marks to combine stripe or symbol layers. Layers support add, subtract, and XOR operations. Pattern coverage is clipped to the host shape and its group clip; overlapping crosshatch strokes do not accumulate opacity.

Choose mark, plot, or chart anchoring. Plot anchoring requires a `PatternReferenceFrame` on the mark's group or an ancestor. The frame uses that group's local coordinates, so neighboring marks can share a stripe phase. Symbol legends accept the same pattern definitions.

```sh
cargo run --release -p avenger-wgpu --example patterns_and_text -- docs/images/patterns-and-text.png
```

![Bars with aligned stripe fills and a math label](images/patterns-and-text.png)

## Supply images through a resolver

`SceneImageMark::image` now contains `SceneImageSource` values. Wrap owned pixels or a shared `Arc<RgbaImage>` in `SceneImageSource::inline`, or use `SceneImageSource::Resource` with a stable key and intrinsic dimensions. Set the image resolver in `CanvasConfig::image_resource_config`.

`ImageResourceCache` handles requests, freshness, eviction, and render invalidation. Required requests precede prefetch requests, then higher priorities load first. The cache bounds concurrent loads on native and browser targets. Applications compute request priorities from their own viewport and interaction state. Installed scenes retain their image keys until replacement or renderer destruction. Pending resources can use a placeholder or skip drawing. A ready resource appears on a later render without another `set_scene` call.

`SceneWarpedImageMark` accepts a textured triangle mesh. Tile hints route eligible resources through persistent texture arrays. Tiles must match the declared tile size. Ordinary atlas images can resize to their declared intrinsic dimensions. Fallback images must match the required dimensions in either path. Tile slots distinguish each mark’s fallback and unavailable-image policy, so sharing a resource key preserves each mark’s behavior. Local data URI decoding works without the HTTP feature. Opt into `avenger-image`'s `reqwest` feature for native URL fetching.

```sh
cargo run --release -p avenger-wgpu --example image_resources -- docs/images/image-resources.png
```

![Pending and ready resources with a warped image](images/image-resources.png)
