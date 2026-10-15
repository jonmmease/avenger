//! PDF export for Avenger scene graphs, written with `krilla`.
//!
//! `PdfRenderer` renders a `SceneGraph` to a one-page PDF document without creating a WGPU
//! device. A scene unit is a point. Marks, gradients, clip paths, and pattern masks stay vector
//! geometry. Fonts are embedded, and text, including math, stays selectable.
//!
//! ```rust
//! use avenger_pdf::PdfRenderer;
//!
//! # let scene_graph = avenger_scenegraph::scene_graph::SceneGraph { width: 100.0, height: 50.0, origin: [0.0, 0.0], marks: vec![] };
//! let engine = avenger_typst_label::bundled_label_engine();
//! let pdf = PdfRenderer::new().render_scene_graph(&scene_graph, &engine)?;
//! assert!(pdf.starts_with(b"%PDF-"));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

mod error;
mod options;
mod renderer;

pub use error::AvengerPdfError;
pub use options::PdfRenderOptions;
pub use renderer::PdfRenderer;
