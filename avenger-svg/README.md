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

## Gallery and validation

Run these commands from the repository root:

```sh
cargo run --release -p avenger-svg --example svg_gallery -- svg-gallery.svg
cargo test --release -p avenger-svg
cargo run --release -p avenger-wgpu --example export_gallery -- wgpu-gallery.png
```

The gallery writes an SVG and a PNG preview. The tests register the SVG's embedded fonts explicitly because resvg does not load CSS webfonts. Browser verification checks the webfont path separately.

The CPU baseline is `tests/baselines/gallery.png`. Set `AVENGER_UPDATE_EXPORT_BASELINES=1` when intentionally regenerating it, then inspect the image and diff. The shared scene fixture also supplies the WGPU comparison example.
