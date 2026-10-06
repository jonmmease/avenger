//! Ported from crates/typst-library/src/model/mod.rs @ v0.15.1, modified for Avenger.
//!
//! Structuring elements that define the document model.
//!
//! avenger: only the emphasis elements and the paragraph properties inline layout reads;
//! labels have no document structure.

mod emph;
mod par;
mod strong;

#[allow(unused_imports, reason = "the built-in show rules use these")]
pub use self::emph::*;
pub use self::par::*;
#[allow(unused_imports, reason = "the built-in show rules use these")]
pub use self::strong::*;
