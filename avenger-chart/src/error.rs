use datafusion::error::DataFusionError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AvengerChartError {
    #[error("Internal error: `{0}`")]
    InternalError(String),

    #[error("DataFusion error: `{0}`")]
    DataFusionError(#[from] DataFusionError),
}
