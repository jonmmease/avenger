use thiserror::Error;

#[cfg(feature = "pyo3")]
use pyo3::{exceptions::PyValueError, PyErr};

#[derive(Error, Debug)]
pub enum AvengerError {
    #[error("Internal error: `{0}`")]
    InternalError(String),
}

// Conversion to PyO3 error
#[cfg(feature = "pyo3")]
impl From<AvengerError> for PyErr {
    fn from(err: AvengerError) -> PyErr {
        PyValueError::new_err(err.to_string())
    }
}
