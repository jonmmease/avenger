use arrow::error::ArrowError;
use avenger_scales::error::AvengerScaleError;
use avenger_scenegraph::error::AvengerSceneGraphError;
use datafusion::error::DataFusionError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AvengerChartError {
    #[error("Internal error: `{0}`")]
    InternalError(String),

    #[error("No scale compiler registered for scale type: `{0}`")]
    ScaleTypeLookupError(String),

    #[error("No mark compiler registered for mark type: `{0}`")]
    MarkTypeLookupError(String),

    #[error("Dataset not found: `{0}`")]
    DatasetLookupError(String),

    #[error("SceneGraph error: `{0}`")]
    SceneGraphError(#[from] AvengerSceneGraphError),

    #[error("Scale error: `{0}`")]
    ScaleError(#[from] AvengerScaleError),

    #[error("DataFusion error: `{0}`")]
    DataFusionError(#[from] DataFusionError),

    #[error("Arrow error: `{0}`")]
    ArrowError(#[from] ArrowError),
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
