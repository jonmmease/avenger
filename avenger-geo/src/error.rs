use thiserror::Error;

#[derive(Error, Debug)]
pub enum AvengerGeoError {
    #[error("Invalid projection configuration: {0}")]
    InvalidConfig(String),

    #[error("GeoJSON parse error: {0}")]
    GeoJson(#[from] geojson::Error),

    #[error("WKB error: {0}")]
    Wkb(String),

    #[error("Fit failed: {0}")]
    FitError(String),
}
