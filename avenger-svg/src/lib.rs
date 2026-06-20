//! SVG string export for Avenger scene graphs.
//!
//! `SvgRenderer` renders a `SceneGraph` directly to an SVG string without
//! creating a WGPU device. The output dimensions and `viewBox` use the
//! scenegraph's logical pixel width and height. By default the renderer emits a
//! white background, native SVG text, native gradients and clip paths, embedded
//! PNG image data URIs, and subset WOFF2 `@font-face` rules for resolvable text
//! fonts.
//!
//! ```rust,ignore
//! use avenger_svg::SvgRenderer;
//!
//! let svg = SvgRenderer::new().render_scene_graph(&scene_graph)?;
//! std::fs::write("chart.svg", svg)?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

pub mod error;
mod fonts;
pub mod options;
pub mod path;
pub mod renderer;
pub mod style;

pub use error::AvengerSvgError;
pub use options::{SvgBackground, SvgFontEmbedding, SvgImageMode, SvgRenderOptions};
pub use renderer::SvgRenderer;
