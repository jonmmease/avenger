//! Direct PDF export for Avenger scene graphs using `krilla`.
//!
//! This crate is a staging ground for the future `avenger-pdf`
//! implementation. It writes PDF pages directly instead of converting through
//! SVG, which will let regular text, math glyphs, and emoji flow through
//! `krilla`'s font embedding machinery.

pub mod error;
pub mod options;
pub mod renderer;

pub use error::AvengerPdfError;
pub use options::{PdfBackground, PdfRenderOptions};
pub use renderer::PdfRenderer;
