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
