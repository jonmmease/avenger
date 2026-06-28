//! Label diagnostics.
//!
//! This mirrors the diagnostic role of upstream `typst-syntax` and Typst's
//! compile diagnostics, but keeps only the label-engine error and warning types
//! that callers need at the `LabelEngine` boundary.

mod error;
mod warnings;

pub use error::{LabelError, LabelInitError};
pub use warnings::LabelWarning;
