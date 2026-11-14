//! Diagnostics: the errors of a parse, each with a code, the range of its text, a message, help,
//! and the edits that fix it.

use std::fmt;

use rowan::TextRange;

/// What a diagnostic is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DiagnosticCode {
    /// A token is not part of the grammar, or is malformed: an unterminated string or comment,
    /// a bad escape, an empty quoted name, or a number followed by letters.
    InvalidToken,
    /// The parser expected something else here.
    Expected,
    /// A reserved keyword where the grammar takes a name.
    ReservedName,
    /// A form of GoogleSQL that avenger-sql does not read.
    Unsupported,
    /// Comparisons chained without parentheses, as in `a < b < c`.
    ChainedComparison,
    /// A `NOT` in the operand of an operator that binds more tightly.
    UnparenthesizedNot,
    /// Adjacent string literals with no whitespace or comment between them.
    AdjacentLiterals,
    /// A query that starts with `SELECT`, or a standard `WITH` clause.
    NotPipeSyntax,
    /// A rule of the forms of a statement, such as the parts of a `CREATE`, or of a module.
    InvalidStatement,
    /// Parentheses nested too deeply to parse.
    TooDeep,
}

/// An edit of the source: the text that replaces a range.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextEdit {
    pub range: TextRange,
    pub replacement: String,
}

impl TextEdit {
    pub fn new(range: TextRange, replacement: impl Into<String>) -> Self {
        Self {
            range,
            replacement: replacement.into(),
        }
    }
}

/// A fix of a diagnostic: a description and its edits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fix {
    pub label: String,
    pub edits: Vec<TextEdit>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub code: DiagnosticCode,
    pub range: TextRange,
    pub message: String,
    pub help: Option<String>,
    pub fixes: Vec<Fix>,
}

impl Diagnostic {
    pub fn new(code: DiagnosticCode, range: TextRange, message: impl Into<String>) -> Self {
        Self {
            code,
            range,
            message: message.into(),
            help: None,
            fixes: Vec::new(),
        }
    }

    /// Sets the help of the diagnostic.
    pub fn help(&mut self, help: impl Into<String>) -> &mut Self {
        self.help = Some(help.into());
        self
    }

    /// Adds a fix: its label and its edits.
    pub fn fix(&mut self, label: impl Into<String>, edits: Vec<TextEdit>) -> &mut Self {
        self.fixes.push(Fix {
            label: label.into(),
            edits,
        });
        self
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)?;
        if let Some(help) = &self.help {
            write!(f, " (help: {help})")?;
        }
        Ok(())
    }
}

impl std::error::Error for Diagnostic {}
