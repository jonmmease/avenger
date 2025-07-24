use avenger_app::error::AvengerAppError;
use avenger_guides::error::AvengerGuidesError;
use avenger_scales::error::AvengerScaleError;
use avenger_scenegraph::error::AvengerSceneGraphError;
use avenger_wgpu::error::AvengerWgpuError;
use datafusion::arrow::error::ArrowError;
use datafusion::error::DataFusionError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AvengerChartError {
    #[error("Internal error: `{0}`")]
    InternalError(String),

    #[error("Invalid argument: `{0}`")]
    InvalidArgument(String),

    #[error("Serialization error: `{0}`")]
    SerializationError(String),

    #[error("No scale compiler registered for scale type: `{0}`")]
    ScaleKindLookupError(String),

    #[error("No mark compiler registered for mark type: `{0}`")]
    MarkTypeLookupError(String),

    #[error("Dataset not found: `{0}`")]
    DatasetLookupError(String),

    #[error("Missing required channel: `{0}`")]
    MissingChannelError(String),

    #[error("SceneGraph error: `{0}`")]
    SceneGraphError(#[from] AvengerSceneGraphError),

    #[error("Scale error: `{0}`")]
    ScaleError(#[from] AvengerScaleError),

    #[error("App error: `{0}`")]
    AppError(#[from] AvengerAppError),

    #[error("Guide error: `{0}`")]
    GuideError(#[from] AvengerGuidesError),

    #[error("DataFusion error: `{0}`")]
    DataFusionError(#[from] DataFusionError),

    #[error("Arrow error: `{0}`")]
    ArrowError(#[from] ArrowError),

    #[error("Image error: `{0}`")]
    ImageError(#[from] image::ImageError),

    #[error("Wgpu error: `{0}`")]
    WgpuError(#[from] AvengerWgpuError),
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

impl From<AvengerChartError> for AvengerAppError {
    fn from(value: AvengerChartError) -> Self {
        match value {
            AvengerChartError::AppError(e) => e,
            e => AvengerAppError::InternalError(e.to_string()),
        }
    }
}