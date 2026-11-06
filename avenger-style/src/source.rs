//! Sources, spans, and the diagnostics that checks report.

use std::fmt;

/// The id of a stylesheet's text, which the caller chooses, such as the position of a file in the
/// span table of a document.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SourceId(pub u32);

/// A range of bytes in the text of one source.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Span {
    pub source: SourceId,
    /// The offset of the first byte.
    pub start: usize,
    /// The offset after the last byte.
    pub end: usize,
}

impl Span {
    pub fn new(source: SourceId, start: usize, end: usize) -> Self {
        Self { source, start, end }
    }

    /// The empty span at `offset`, for a diagnostic that points at a position.
    pub fn at(source: SourceId, offset: usize) -> Self {
        Self::new(source, offset, offset)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Severity {
    Error,
    Warning,
}

/// An error or a warning at a span of a stylesheet. A caller maps the span to the line and the
/// column of its file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub span: Span,
    pub message: String,
    pub help: Option<String>,
}

impl Diagnostic {
    pub fn error(span: Span, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            span,
            message: message.into(),
            help: None,
        }
    }

    pub fn warning(span: Span, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            span,
            message: message.into(),
            help: None,
        }
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let severity = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        write!(f, "{severity}: {}", self.message)?;
        if let Some(help) = &self.help {
            write!(f, " (help: {help})")?;
        }
        Ok(())
    }
}

/// A problem that a check finds, before the caller gives it a span.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Problem {
    pub message: String,
    pub help: Option<String>,
}

impl Problem {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Problem {
            message: message.into(),
            help: None,
        }
    }

    pub(crate) fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    pub(crate) fn at(self, span: Span) -> Diagnostic {
        Diagnostic {
            severity: Severity::Error,
            span,
            message: self.message,
            help: self.help,
        }
    }
}
