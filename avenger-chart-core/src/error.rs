use avenger_color::ColorParseError;
use avenger_guides::error::AvengerGuidesError;
use avenger_scales::error::AvengerScaleError;
use avenger_scenegraph::error::AvengerSceneGraphError;
use avenger_text::error::AvengerTextError;
use datafusion::{arrow::error::ArrowError, error::DataFusionError};
use thiserror::Error;

use crate::ChannelResolutionError;

#[derive(Error, Debug)]
pub enum AvengerChartError {
    #[error("Internal error: `{0}`")]
    InternalError(String),

    #[error("Invalid argument: `{0}`")]
    InvalidArgument(String),

    #[error("Serialization error: `{0}`")]
    SerializationError(String),

    #[error("Deserialization error: `{0}`")]
    DeserializationError(String),

    #[error("No scale compiler registered for scale type: `{0}`")]
    ScaleKindLookupError(String),

    #[error("No mark compiler registered for mark type: `{0}`")]
    MarkTypeLookupError(String),

    #[error("Dataset not found: `{0}`")]
    DatasetLookupError(String),

    #[error("Missing required channel: `{0}`")]
    MissingChannelError(String),

    #[error("Scale not found: `{0}`")]
    ScaleNotFound(String),

    #[error("Non-constant channel: `{0}`")]
    NonConstantChannel(String),

    #[error("SceneGraph error: `{0}`")]
    SceneGraphError(#[from] AvengerSceneGraphError),

    #[error("Scale error: `{0}`")]
    ScaleError(#[from] AvengerScaleError),

    #[error("Coordinate guide error: `{0}`")]
    GuideError(#[from] AvengerGuidesError),

    #[error("Text error: `{0}`")]
    TextError(#[from] AvengerTextError),

    #[error("DataFusion error: `{0}`")]
    DataFusionError(#[from] DataFusionError),

    #[error("Arrow error: `{0}`")]
    ArrowError(#[from] ArrowError),

    #[error("Color parse error: `{0}`")]
    ColorParseError(#[from] ColorParseError),

    #[error("Layout error: `{0}`")]
    LayoutError(String),

    #[error("Coordinate system error: `{0}`")]
    CoordinateSystemError(String),

    #[error("Channel resolution error: `{0}`")]
    ChannelResolutionError(#[from] ChannelResolutionError),

    #[error(
        "Positional scale '{scale_name}' in {coord_system} coordinate system contains only literal values.\n\
             Found: {literal_value}\n\
             This would map all points to the same position.\n\n\
             {suggestion}"
    )]
    PositionalScaleLiteralError {
        scale_name: String,
        coord_system: String,
        literal_value: String,
        suggestion: String,
    },
}

impl From<AvengerChartError> for DataFusionError {
    fn from(value: AvengerChartError) -> Self {
        match value {
            AvengerChartError::DataFusionError(e) => e,
            AvengerChartError::ArrowError(e) => DataFusionError::ArrowError(e, None),
            e => DataFusionError::Execution(e.to_string()),
        }
    }
}
