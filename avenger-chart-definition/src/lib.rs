#![doc = include_str!("../README.md")]

mod builder;
mod model;
mod serialization;
mod validate;

pub use avenger_datafusion_dataflow as dataflow;
pub use avenger_layout::{CellAlign, Edges, GridSlot, Side, Size, TrackSize};
pub use avenger_panels::{LabelVisibility, Scope as PanelScope};
pub use builder::{ChartBuilder, PlotBuilder};
pub use model::*;
pub use serialization::protobuf;

/// Definition construction, validation, and artifact errors.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{path}: {message}")]
    Invalid { path: String, message: String },
    #[error(transparent)]
    Dataflow(#[from] dataflow::Error),
    #[error("chart artifact: {0}")]
    Artifact(String),
}
/// Result of chart-definition operations.
pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn invalid(path: impl Into<String>, message: impl Into<String>) -> Error {
    Error::Invalid {
        path: path.into(),
        message: message.into(),
    }
}
