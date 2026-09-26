# avenger-svg

Export an Avenger `SceneGraph` to an SVG string without a GPU or window. Marks, gradients, clipping paths, pattern masks, and math outlines remain vector geometry. Ordinary text remains text. Images are embedded as PNGs.

```rust
use avenger_svg::SvgRenderer;

fn export(scene: &avenger_scenegraph::scene_graph::SceneGraph) -> Result<String, avenger_svg::AvengerSvgError> {
SvgRenderer::new().render_scene_graph(scene)
}
```

`SvgRenderOptions` controls the background, coordinate precision, font embedding, and color glyph images. Dimensions use scene units and must be positive and finite.

Pass the same `TextEngine` used for layout to `SvgRenderer::with_text_engine`. The supplied engine takes precedence over `font_resolution`. The default renderer uses the bundled core fonts and permits system fonts. For reproducible output, set `load_system_fonts` to `false` and register every required face.

Embedded fonts use unique document names and the exact face bytes returned by the text engine. TrueType faces use WOFF2. Subsetting is used only when it can preserve shaping behavior. Faces with kerning, shaping, variation, or color tables remain complete. Other supported OpenType faces use embedded OpenType data. `SvgFontEmbedding::None` omits font data and requires the viewer to supply the fonts. Missing fonts still follow the text engine's policy during layout.

Positive finite text limits ellipsize plain text at grapheme boundaries. Typst labels compile in full, then clip at the limit before rotation and placement. SVG supports the same Typst subset and plain fallback as `avenger-text`. Gradient text paint returns an error. Math and runs with explicit OpenType features (such as typographic subscripts, superscripts, and small caps) use paths and are not selectable text. Outlining these runs preserves the shaped glyphs and spacing in viewers that do not support those features. Synthesized scripts remain native text. Color glyph images are embedded by default when the text engine supplies them.

Resolve resource-backed images with `avenger_scenegraph::image_resources::resolve_ready_image_resources` before export. The helper reads ready resources from the caller's resolver and returns a scene with inline images. Neither the helper nor the SVG renderer waits for loading. Warped images use software rasterization at two pixels per scene unit, capped at 8192 pixels per dimension. The cap reduces resolution while preserving the complete mesh.

## Shared rendering semantics

Ordinary scene fills use `FillRule::NonZero`. Custom path marks, symbol marks, and path clips can select `FillRule::EvenOdd`. The rule also applies to pattern-host clipping and filled-geometry queries. Pattern symbols keep their even-odd default. Text and math retain their own glyph-outline rules. Path clips carry both values as `Clip::Path { path, fill_rule }`.

SVG and WGPU share the filled outline of each trail, including round caps, variable widths, isolated samples, and overlapping segments. Translucent paint applies once within a trail. Ordinary strokes and open pattern symbols use an SVG miter limit of 8, equivalent to Lyon's limit of 4. Explicit Typst stroke limits remain unchanged.

Symbol gradients use the nominal unrotated square with side length `sqrt(size)`. Trail gradients use centerline bounds. Radial gradients expand those bounds to a centered square and interpolate between the starting and ending circles. SVG retains native gradient elements, while WGPU evaluates the circle equation in its fragment shader. Circle controls preserve float precision independently of the SVG scene-coordinate precision setting. Identical radial circles produce no paint. A linear gradient with a zero-width or zero-height reference box also produces no paint.

Image filtering uses premultiplied alpha to keep transparent source colors from producing fringes. Public image data and embedded PNGs remain straight RGBA. The same filtering convention applies to cached GPU tiles, resource resizing, and software-rasterized warped images.

## Gallery and validation

Run these commands from the repository root:

```sh
cargo run --release -p avenger-svg --example svg_gallery -- svg-gallery.svg
cargo test --release -p avenger-svg
cargo run --release -p avenger-wgpu --example export_gallery -- wgpu-gallery.png
```

The gallery writes an SVG and a PNG preview. The tests register the SVG's embedded fonts explicitly because resvg does not load CSS webfonts. Browser verification checks the webfont path separately.

The CPU baseline is `tests/baselines/gallery.png`. Set `AVENGER_UPDATE_EXPORT_BASELINES=1` when intentionally regenerating it, then inspect the image and diff. The shared scene fixture also supplies the WGPU comparison example.

The shared parity fixtures check filled coverage, clipping, trails, gradient bounds, miter joins, and image filtering. To also write the SVG and WGPU images for browser comparison:

```sh
AVENGER_PARITY_OUTPUT=svg-parity cargo test --release -p avenger-wgpu --test test_svg_parity
```

Compare the generated SVGs in Chrome with their corresponding WGPU PNGs at the same scale. The pinned resvg version ignores nonzero `fr`, so radial starting-circle cases require browser verification. Automated CPU comparisons cover the remaining fixtures. Rasterization at geometry edges and native font outlines can differ between backends.
