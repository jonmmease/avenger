# avenger-pdf

Export an Avenger `SceneGraph` as a one-page PDF document. Marks, gradients, clip paths, and pattern
masks are vector geometry. Fonts are embedded, and text, including math, is selectable.

```rust
use avenger_pdf::{AvengerPdfError, PdfRenderer};
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_typst_label::LabelEngine;

fn export(scene: &SceneGraph, engine: &LabelEngine) -> Result<Vec<u8>, AvengerPdfError> {
    PdfRenderer::new().render_scene_graph(scene, engine)
}
```

A scene unit is a point, and the page is the scene's width and height, which must be positive and
finite. `PdfRenderOptions` sets the page's background. The renderer draws by the scene graph's
[rendering semantics](../avenger-scenegraph/README.md#rendering-semantics), with these PDF
specifics:

- Radial gradients are native PDF shadings. Their circles scale up, with a transform that undoes
  the scale, so that viewers resolve nearly identical circles precisely.
- A smooth image's transparent pixels take the color of their visible neighbors, so that filtering
  keeps their hidden color out of the image's edges.

## Text

`render_scene_graph` lays out text with the `LabelEngine` it's given, which should be the engine
that laid out the scene. For reproducible output, build the engine from `FontOptions` with
`load_system_fonts` off, every face the scene needs registered, and `missing_font` set to
`MissingFontPolicy::Error`.

Labels lay out through the label crate, so a text mark's width, wrapping, line limits, ellipsis,
and line spacing apply to plain text and Typst markup alike, and invalid markup draws as its
literal source. Text extracts as the characters it shows, including the ellipses of cut lines.

Each font face embeds as a subset. A face whose license, the embedding flags of its `OS/2` table,
forbids embedding or subsetting is an error.

## Images

PDF has no textured mesh, so a mesh image mark draws all its instances as one image, rasterized at
two pixels per scene unit. Its pixels lie on the page's pixel grid at that scale, so adjacent
instances, such as map tiles, meet without a seam, and a clipped group's vector clip draws the
layer's edge. An image wider or taller than 8192 pixels halves its scale until it fits.

## Tests

```sh
cargo test --release -p avenger-pdf
cargo run --release -p avenger-pdf --example pdf_gallery -- pdf-gallery.pdf
```

The ordinary tests check documents, text extraction, fonts, and images. The ignored tests
rasterize documents with PDFium 7763, which `scripts/fetch_pdfium.sh` downloads and verifies for
macOS ARM64 and Linux x86_64. On macOS:

```sh
avenger-pdf/scripts/fetch_pdfium.sh
export AVENGER_PDFIUM_LIBRARY_PATH="$PWD/target/pdfium/lib/libpdfium.dylib"
cargo test --release -p avenger-pdf -- --ignored
```

On Linux, the library is `libpdfium.so`. CI runs the ignored tests.

They compare the gallery with `tests/baselines/gallery.png`, and check text placement, rotated and
cut text, image identity and smoothing, and each parity scene's samples. They write their images to
the ignored `tests/output`, and copying `tests/output/gallery.png` over the baseline accepts a
change.
