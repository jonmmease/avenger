//! SVG string export for Avenger scene graphs.
//!
//! `SvgRenderer` renders a `SceneGraph` directly to an SVG string without
//! creating a WGPU device. The output dimensions and `viewBox` use the
//! scenegraph's logical pixel width and height. By default the renderer emits a
//! white background, native SVG text, native gradients and clip paths, embedded
//! PNG images, and embedded font faces. Fonts retain their shaping tables.
//! Math is outlined, and color emoji use embedded images by default.
//!
//! ```rust
//! use avenger_svg::SvgRenderer;
//!
//! # let scene_graph = avenger_scenegraph::scene_graph::SceneGraph { width: 100.0, height: 50.0, origin: [0.0, 0.0], marks: vec![] };
//! let engine = avenger_typst_label::bundled_label_engine();
//! let svg = SvgRenderer::new().render_scene_graph(&scene_graph, &engine)?;
//! assert!(svg.starts_with("<svg"));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

mod error;
mod fonts;
mod options;
mod path;
mod renderer;
mod style;

pub use error::AvengerSvgError;
pub use options::SvgRenderOptions;
pub use renderer::SvgRenderer;
