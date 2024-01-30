use thiserror::Error;

#[cfg(feature = "pyo3")]
use pyo3::{exceptions::PyValueError, PyErr};

#[derive(Error, Debug)]
pub enum AvengerVegaError {
    #[error("Avenger error")]
    AvengerError(#[from] avenger::error::AvengerError),

    #[error("css color parse error")]
    InvalidColor(#[from] csscolorparser::ParseColorError),

    #[error("image error")]
    ImageError(#[from] image::ImageError),

    // ParseError doesn't implement std::Error, so #[from] doesn't seem to work
    #[error("Error parsing SVG path")]
    InvalidSvgPath(lyon_extra::parser::ParseError),

    #[error("Invalid dash string: {0}")]
    InvalidDashString(String),

    #[error("Image fetching is not enabled: {0}")]
    NoImageFetcherConfigured(String),

    #[cfg(feature = "image-request")]
    #[error("css color parse error")]
    ReqwestError(#[from] reqwest::Error),
}

impl From<lyon_extra::parser::ParseError> for AvengerVegaError {
    fn from(value: lyon_extra::parser::ParseError) -> Self {
        Self::InvalidSvgPath(value)
    }
}

#[cfg(feature = "pyo3")]
impl From<AvengerVegaError> for PyErr {
    fn from(err: AvengerVegaError) -> PyErr {
        PyValueError::new_err(err.to_string())
    }
}
