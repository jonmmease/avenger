# avenger-pdf

Export an Avenger `SceneGraph` to PDF bytes or a file without a GPU or window. Marks, gradients, clipping paths, and pattern masks remain vector geometry. Fonts are embedded, and ordinary text and math glyphs remain selectable.

```rust
use avenger_pdf::PdfRenderer;

fn export(scene: &avenger_scenegraph::scene_graph::SceneGraph) -> Result<Vec<u8>, avenger_pdf::AvengerPdfError> {
    PdfRenderer::new().render_scene_graph(scene)
}
```

`PdfRenderer::write_scene_graph_pdf` renders a complete document and writes it to a path, creating parent directories as needed. Each scene produces one page. One scene unit maps to one PDF point. Dimensions must be positive and finite and are preserved without rounding or minimum-size clamping.

Pass the same `TextEngine` used for layout to `PdfRenderer::with_text_engine`. The supplied engine takes precedence over `font_resolution`. The default renderer uses bundled core fonts and permits system fonts. For reproducible output, disable `load_system_fonts` and register the required faces. Unicode text does not override that setting.

Positive finite text limits ellipsize plain text at grapheme boundaries. Typst labels compile in full and clip before rotation and placement. Clipping preserves the complete typeset semantic text for extraction. The renderer uses the Typst subset and plain fallback from `avenger-text`. Gradient text paint and non-translation glyph transforms return errors.

Resolve resource-backed images with `avenger_scenegraph::image_resources::resolve_ready_image_resources` before export. The helper reads ready resources from the caller's resolver and returns a scene with inline images. It does not wait for loading. Ordinary images preserve their `smooth` setting. Warped images rasterize at two pixels per scene unit, capped at 8192 pixels per dimension. The cap reduces resolution while preserving the complete mesh.

## Gallery and tests

Run these commands from the repository root:

```sh
cargo run --release -p avenger-pdf --example pdf_gallery -- pdf-gallery.pdf
cargo test --release -p avenger-pdf
```

PDF generation uses Krilla. PDFium is a development dependency used only to inspect raster output. The ordinary test suite verifies PDF structure, text extraction, fonts, and image-resource resolution without loading PDFium.

The setup helper downloads PDFium 7763 and verifies its archive checksum. It supports macOS ARM64 and Linux x86_64. On macOS:

```sh
avenger-pdf/scripts/fetch_pdfium.sh
export AVENGER_PDFIUM_LIBRARY_PATH="$PWD/target/pdfium/lib/libpdfium.dylib"
cargo test --release -p avenger-pdf --test export -- --include-ignored
cargo run --release -p avenger-pdf --example pdf_gallery -- pdf-gallery.pdf
```

On Linux, use `libpdfium.so` in the environment variable. With that variable set, the gallery also writes a PNG preview. CI installs the pinned library and runs all PDF raster checks explicitly.

The visual suite compares the shared gallery with its PDF baseline and SVG rasterization. It also tests rotated text clipping, empty group clips, image smoothing, and images that share pixels but have different dimensions. Set `AVENGER_UPDATE_EXPORT_BASELINES=1` only when intentionally regenerating `tests/baselines/gallery.png`, then inspect the result.
