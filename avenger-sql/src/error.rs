//! Errors at a span of the source text.

use datafusion::{
    common::{DataFusionError, Diagnostic, Span},
    sql::sqlparser::tokenizer,
};

/// A planning error with the message `message`, and a diagnostic at `span`, the source construct
/// of the error.
pub(crate) fn at(span: tokenizer::Span, message: impl Into<String>) -> DataFusionError {
    let message = message.into();
    DataFusionError::Plan(message.clone()).with_diagnostic(Diagnostic::new_error(
        message,
        Span::try_from_sqlparser_span(span),
    ))
}
