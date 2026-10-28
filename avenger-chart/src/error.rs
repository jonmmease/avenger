use arrow::error::ArrowError;

/// An error of decoding, encoding, or checking a chart.
#[derive(Debug, thiserror::Error)]
pub enum ChartError {
    /// The bytes are not a chart message.
    #[error("the bytes are not an avenger chart: {0}")]
    NotAChart(String),
    /// A chart of another format version than this reader's.
    #[error("the chart has format version {0}, and this reader reads version 1")]
    UnsupportedVersion(u32),
    /// A part of the chart that breaks a rule of the format, at its path from the chart, such as
    /// `root/children[1]/encodings[0]`.
    #[error("{path}: {reason}")]
    Invalid { path: String, reason: String },
    /// A one-row or one-column Arrow field of the message that does not decode to what its field
    /// holds.
    #[error("{0}")]
    Payload(String),
    #[error(transparent)]
    Arrow(#[from] ArrowError),
}

impl ChartError {
    pub(crate) fn invalid(path: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::Invalid {
            path: path.into(),
            reason: reason.into(),
        }
    }
}
