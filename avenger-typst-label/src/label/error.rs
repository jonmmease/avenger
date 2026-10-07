//! Why a label fails to compile, and its warnings.

use std::ops::Range;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::typst_layout::math::MATH_TOO_COMPLEX;
use crate::typst_library::diag::SourceDiagnostic;
use typst_syntax::DiagSpanKind;

use super::label_file;

/// Why a label failed to compile.
#[derive(Debug, Clone, Error, PartialEq)]
pub enum LabelError {
    /// An error in the label's source, from parsing, evaluation or layout, in upstream's
    /// wording.
    #[error("{message} at {}..{}", range.start, range.end)]
    Source {
        /// The source's byte range that the error is about.
        range: Range<usize>,
        /// The error message.
        message: String,
        /// Hints for fixing the error.
        hints: Vec<String>,
    },

    /// The label's width is negative or not finite.
    #[error("label width {width} is negative or not finite")]
    InvalidWidth { width: f32 },

    /// The label's line height, a distance or a multiple, is negative or not finite.
    #[error("label line height {line_height} is negative or not finite")]
    InvalidLineHeight { line_height: f32 },

    /// The source is longer than the label's limit.
    #[error("source is {actual} bytes, exceeding max_source_bytes={limit}")]
    SourceTooLarge { actual: usize, limit: usize },

    /// The source has more equations than the label's limit.
    #[error("label has {actual} equations, exceeding max_math_spans={limit}")]
    TooManyMathSpans { actual: usize, limit: usize },

    /// Math nests deeper than the label's limit.
    #[error("math nesting depth is {actual}, exceeding max_math_depth={limit}")]
    MathDepthExceeded { actual: usize, limit: usize },

    /// An equation needs more layout work than any label may.
    #[error("the equation at byte {position} is too complex to lay out")]
    MathTooComplex {
        /// Where the equation starts in the source.
        position: usize,
    },

    /// None of the families of a font list is available, under the `Error` policy.
    #[error("none of the font families in {family:?} is available")]
    MissingFont {
        /// The font list.
        family: String,
    },
}

/// A non-fatal problem with a label.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum LabelWarning {
    /// A family of one of the label's font lists is not available, under the `Warn` policy.
    MissingFont { family: String },
    /// A warning from typesetting, in upstream's wording.
    Typst {
        /// The source's byte range that the warning is about.
        range: Range<usize>,
        /// The warning message.
        message: String,
        /// Hints about the warning.
        hints: Vec<String>,
    },
}

/// The byte range a diagnostic is about, or the whole source when it points nowhere in it.
fn range(source: &str, diagnostic: &SourceDiagnostic) -> Range<usize> {
    match diagnostic.span.get() {
        DiagSpanKind::Range { id, range } if id == label_file() => range,
        _ => 0..source.len(),
    }
}

fn hints(diagnostic: &SourceDiagnostic) -> Vec<String> {
    diagnostic.hints.iter().map(|hint| hint.v.to_string()).collect()
}

/// An error diagnostic as a label error.
pub(crate) fn source_error(source: &str, error: &SourceDiagnostic) -> LabelError {
    let range = range(source, error);
    if error.message == MATH_TOO_COMPLEX {
        return LabelError::MathTooComplex { position: range.start };
    }
    LabelError::Source {
        range,
        message: error.message.to_string(),
        hints: hints(error),
    }
}

/// A warning diagnostic as a label warning.
pub(crate) fn source_warning(source: &str, warning: &SourceDiagnostic) -> LabelWarning {
    LabelWarning::Typst {
        range: range(source, warning),
        message: warning.message.to_string(),
        hints: hints(warning),
    }
}
