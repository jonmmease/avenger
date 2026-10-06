# avenger-text

Text measurement, rasterization, and SVG/PDF text extraction for the Avenger
rendering system.

## Responsibilities

This crate provides:
- Text measurement: computing bounding boxes, ascent/descent, and line height.
- Text rasterization: converting whole text lines to images and paths.
- SVG and PDF extraction. For SVG, a run of glyphs is native text when shaping its
  text with its face gives the same glyphs; other runs, such as math and runs with
  OpenType features, are outlines. For PDF, text is glyph runs. Shapes are paths and
  bitmap glyphs are images in both.

## Architecture

The active backend is the Typst-style text engine in `avenger-typst-label`.
Plain text is the default. Select `TextSyntaxMode::TypstMarkup` to enable
markup and `$...$` math fragments.

## Number formatting

Call `TextEngine::with_number_formatting(Arc::new(provider))` to select a provider
for numeric labels. Measurement and rendering share this selection. Per-label
configuration overrides it, and caches match providers by identity. A new engine
has no number formatter configured.

## Usage by Other Crates

- `avenger-scenegraph`: Uses font types in the scene graph text mark
- `avenger-wgpu`: Uses rasterized text lines for GPU text rendering
- `avenger-vega-scenegraph`: Processes Vega text marks using measurement and rasterization
- `avenger-geometry`: Uses text measurement for computing geometry of text marks
- Vector exporters use the SVG and PDF extraction, which keeps text selectable
  where viewers reproduce it

## Feature Flags

- `serde`: Enables serialization for text types (default)
