/// A structural or semantic error at a path in the specification.
#[derive(Debug, thiserror::Error)]
#[error("{path}: {message}")]
pub struct SpecError {
    path: String,
    message: String,
}

impl SpecError {
    pub(crate) fn new(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            message: message.into(),
        }
    }

    /// The property path, or `$` for a document-level error.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Describes the invalid shape or value.
    pub fn message(&self) -> &str {
        &self.message
    }
}
