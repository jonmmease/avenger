# avenger-image

Image loading and processing crate for the Avenger rendering system.

## Responsibilities

This crate handles:
- Loading images from various sources (URLs, data URIs)
- Decoding image formats (PNG, JPEG, etc.) into RGBA pixel data
- Rasterizing SVG documents to PNG format
- Providing a uniform `RgbaImage` type for downstream crates

## Dependencies

The crate is used by:
- `avenger-scenegraph`: Image marks reference `RgbaImage` for their content
- `avenger-wgpu`: Converts `RgbaImage` to GPU textures for rendering
- `avenger-vega`: Processes Vega scenegraph image URLs into `RgbaImage` instances

## Architecture

### Core Types

- `RgbaImage`: Container for width, height, and RGBA pixel data as `Vec<u8>`
- `ImageFetcher`: Trait defining the interface for fetching remote images
- `ReqwestImageFetcher`: Default implementation using reqwest for HTTP(S) downloads

### Image Sources

The `RgbaImage::from_str` method handles:
- Data URIs: `data:image/png;base64,...` and `data:image/svg+xml,...`
- Remote URLs: `http://` and `https://` prefixes

### Feature Flags

- `svg`: Enables SVG rasterization via resvg/usvg/tiny-skia
- `reqwest`: Enables HTTP(S) image fetching via reqwest

## SVG Handling

When the `svg` feature is enabled, SVG documents are rasterized to PNG using:
1. `usvg` for parsing and normalizing the SVG DOM
2. `resvg` with `tiny-skia` backend for rendering to pixels
3. System fonts loaded via `fontdb` for text rendering