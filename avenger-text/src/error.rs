use thiserror::Error;

#[derive(Error, Debug)]
pub enum AvengerTextError {
    #[error("Internal error: `{0}`")]
    TextMeasurementError(String),

    #[error("Failed to allocate image")]
    ImageAllocationError(String),
}
