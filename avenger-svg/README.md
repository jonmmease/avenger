# avenger-svg

Export an Avenger `SceneGraph` to an SVG string without a GPU or window. Marks, gradients, clip
paths, and pattern masks stay vector geometry. Text stays text, with its fonts embedded, and images
are embedded as PNGs.

```rust
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_svg::{AvengerSvgError, SvgRenderer};
use avenger_typst_label::LabelEngine;

fn export(scene: &SceneGraph, engine: &LabelEngine) -> Result<String, AvengerSvgError> {
    SvgRenderer::new().render_scene_graph(scene, engine)
}
```

The document's size and `viewBox` are the scene's width and height, which must be positive and
finite. `SvgRenderOptions` sets the background, the coordinate precision, font embedding, and
whether color glyphs, such as emoji, draw as images. The renderer draws by the scene graph's
[rendering semantics](../avenger-scenegraph/README.md#rendering-semantics).

## Text

`render_scene_graph` lays out text with the `LabelEngine` it's given, which should be the engine
that laid out the scene, as WGPU's `set_scene` does. For reproducible output, build the engine
from `FontOptions` with `load_system_fonts` off, every face the scene needs registered, and
`missing_font` set to `MissingFontPolicy::Error`.

Labels lay out through the label crate, so a text mark's width, wrapping, line limits, ellipsis,
and line spacing apply to plain text and Typst markup alike, and invalid markup draws as its
literal source. Text runs stay native SVG text. Math, and runs whose OpenType features a viewer
might not apply, such as typographic scripts and small caps, draw as glyph outlines, which keep the
shaped glyphs and spacing but aren't selectable.

Embedded faces have unique family names and the exact face data that the text engine used.
TrueType faces embed as WOFF2, and other OpenType faces as OpenType data. A face is subset to the
text's characters only when that keeps its shaping, so faces with kerning, shaping, variation, or
color tables embed whole. Embedding follows each face's license, the embedding flags of its `OS/2`
table. A face that forbids embedding, or allows only bitmaps, isn't embedded, and its text names
the face's family. A face that forbids subsetting embeds whole. With `embed_fonts` off, viewers
draw text with the fonts they have.

## Images

Image marks embed their pixels as PNG data. SVG has no textured mesh, so a mesh image draws as an
image rasterized at two pixels per scene unit, capped at 8192 pixels per side, which lowers the
resolution but keeps the whole mesh.

## Tests

```sh
cargo test --release -p avenger-svg
cargo run --release -p avenger-svg --example svg_gallery -- svg-gallery.svg
```

The gallery test rasterizes the shared gallery scene with resvg, registering the document's
embedded fonts because resvg doesn't load CSS web fonts, and compares it with
`tests/baselines/gallery.png`. It writes its image to the ignored `tests/output/gallery.png`, and
copying that file over the baseline accepts a change.

The parity scenes check the shared rendering semantics. `avenger-wgpu`'s `test_svg_parity` compares
WGPU's image of each scene with resvg's image of its SVG, and writes both to
`avenger-wgpu/tests/output/parity`. resvg ignores a radial gradient's starting radius, so the
radial scenes compare in a browser.
