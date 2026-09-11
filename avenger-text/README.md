# avenger-text

Text measurement, rasterization, and SVG/PDF text extraction for the Avenger
rendering system.

## Responsibilities

This crate provides:
- Text measurement: computing bounding boxes, ascent/descent, and line height.
- Text rasterization: converting whole text lines to images and paths.
- Hybrid SVG/PDF extraction: native plain text runs plus paths for math and
  decoration shapes.

## Architecture

The active backend is the Typst-style text engine in `avenger-typst-label`.
Plain text is the default. Select `TextSyntaxMode::TypstMarkup` to enable
markup and `$...$` math fragments.

## Usage by Other Crates

- `avenger-scenegraph`: Uses font types in the scene graph text mark
- `avenger-wgpu`: Uses rasterized text lines for GPU text rendering
- `avenger-vega-scenegraph`: Processes Vega text marks using measurement and rasterization
- `avenger-geometry`: Uses text measurement for computing geometry of text marks
- Vector exporters can use hybrid text extraction to preserve selectable plain
  text while emitting math and decorations as paths

## Feature Flags

- `serde`: Enables serialization for text types (default)
