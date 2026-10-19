use avenger_geo::AvengerGeoError;
use avenger_typst_label::LabelError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AvengerGuidesGeoError {
    #[error("Geo error: {0}")]
    Geo(#[from] AvengerGeoError),

    #[error("Text error: {0}")]
    Text(#[from] LabelError),

    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),
}
