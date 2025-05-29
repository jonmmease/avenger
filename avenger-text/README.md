# avenger-text

Text measurement and rasterization for the Avenger rendering system.

## Responsibilities

This crate provides:
- Text measurement: computing bounding boxes, ascent/descent, and line height
- Text rasterization: converting text to glyph images and paths
- Cross-platform text rendering backends

## Architecture

### Core Traits

- `TextMeasurer`: Defines interface for measuring text dimensions
- `TextRasterizer`: Defines interface for converting text to rendered glyphs

### Backends

The crate provides two backend implementations:

#### COSMIC Text (Native)
- Uses the COSMIC Text library for text shaping and rendering
- Supports system fonts via fontdb
- Handles complex text layout including emoji
- Enabled with the `cosmic-text` feature (default)

#### HTML Canvas (WASM)
- Uses the browser's OffscreenCanvas API for text operations
- Leverages browser font rendering capabilities
- Automatically selected when targeting `wasm32`

## Usage by Other Crates

- `avenger-scenegraph`: Uses font types in the scene graph text mark
- `avenger-wgpu`: Uses rasterized glyphs for GPU text rendering
- `avenger-vega`: Processes Vega text marks using measurement and rasterization
- `avegner-geometry`: Uses text measurement for computing geometry of text marks

## Feature Flags

- `serde`: Enables serialization for text types (default)
- `cosmic-text`: Enables COSMIC Text backend for native platforms (default)