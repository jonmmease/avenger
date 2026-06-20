//! PDF export for Avenger scene graphs.
//!
//! `PdfRenderer` preserves text as PDF text and embeds the fonts that are used
//! by text marks. The default font set is Avenger's bundled fonts. If callers
//! enable system font loading or pass extra font directories, they are
//! responsible for ensuring those font licenses permit PDF embedding.

pub mod error;
pub mod options;
pub mod renderer;

pub use error::AvengerPdfError;
pub use options::{PdfBackground, PdfRenderOptions};
pub use renderer::PdfRenderer;
