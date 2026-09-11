//! PDF export for Avenger scene graphs using `krilla`.
//!
//! The renderer writes scene marks, images, and selectable text into PDF pages.
//! Regular text, math glyphs, and emoji flow through `krilla`'s font embedding
//! machinery.

pub mod error;
pub mod options;
pub mod renderer;

pub use error::AvengerPdfError;
pub use options::{PdfBackground, PdfRenderOptions};
pub use renderer::PdfRenderer;
