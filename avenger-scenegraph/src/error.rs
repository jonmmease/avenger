use thiserror::Error;

#[cfg(feature = "pyo3")]
use pyo3::{exceptions::PyValueError, PyErr};

#[derive(Error, Debug)]
pub enum AvengerError {
    #[error("Internal error: `{0}`")]
    InternalError(String),

    // ParseError doesn't implement std::Error, so #[from] doesn't seem to work
    #[error("Error parsing SVG path")]
    InvalidSvgPath(lyon_extra::parser::ParseError),
}

// Conversion to PyO3 error
#[cfg(feature = "pyo3")]
impl From<AvengerError> for PyErr {
    fn from(err: AvengerError) -> PyErr {
        PyValueError::new_err(err.to_string())
    }
}

impl From<lyon_extra::parser::ParseError> for AvengerError {
    fn from(value: lyon_extra::parser::ParseError) -> Self {
        Self::InvalidSvgPath(value)
    }
}
