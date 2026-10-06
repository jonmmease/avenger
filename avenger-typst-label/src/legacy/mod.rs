//! The current label pipeline, which the mirror-upstream rewrite replaces.
//!
//! The public API is re-exported from here until the new pipeline takes over, and then this
//! module is deleted.

pub(crate) mod label;
pub(crate) mod typst_eval;
pub(crate) mod typst_layout;
pub(crate) mod typst_library;
pub(crate) mod typst_realize;
pub(crate) mod typst_render;
pub(crate) mod typst_svg;
