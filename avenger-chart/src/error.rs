use arrow::error::ArrowError;

/// An error of reading, writing, or checking a chart.
#[derive(Debug, thiserror::Error)]
pub enum ChartError {
    /// The bytes are not a chart message.
    #[error("the bytes are not an avenger chart: {0}")]
    NotAChart(String),
    /// A chart of another format version than this reader's.
    #[error("the chart has format version {0}, and this reader reads version 1")]
    UnsupportedVersion(u32),
    /// A part of the chart that breaks a rule of the format, or two parts that conflict, at its
    /// path from the chart, such as `root.plot.cartesian.layers[0].mark.symbol.x`.
    #[error("{path}: {reason}")]
    Invalid { path: String, reason: String },
    /// A combination of charts that breaks a rule of its own, such as an overlay of plots of two
    /// coordinate systems, with the position of the chart that breaks it among the charts that
    /// combine, or `None` for the node of the combination.
    #[error("{}{reason}", child_prefix(.child))]
    Combine {
        child: Option<usize>,
        reason: String,
    },
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

    pub(crate) fn combine(child: Option<usize>, reason: impl Into<String>) -> Self {
        Self::Combine {
            child,
            reason: reason.into(),
        }
    }
}

fn child_prefix(child: &Option<usize>) -> String {
    child.map_or_else(String::new, |child| {
        format!("chart {child} of the combination: ")
    })
}
