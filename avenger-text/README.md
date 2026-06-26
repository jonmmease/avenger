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

The active backend is the owned Typst-style text engine in `avenger-typst`.
It supports regular text and `$...$` math fragments by default.

## Usage by Other Crates

- `avenger-scenegraph`: Uses font types in the scene graph text mark
- `avenger-wgpu`: Uses rasterized text lines for GPU text rendering
- `avenger-vega`: Processes Vega text marks using measurement and rasterization
- `avenger-geometry`: Uses text measurement for computing geometry of text marks
- `avenger-svg` and `avenger-pdf`: Use hybrid text extraction so regular text
  remains native/selectable while math remains vector paths

## Feature Flags

- `serde`: Enables serialization for text types (default)
