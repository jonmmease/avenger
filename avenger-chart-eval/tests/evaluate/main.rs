//! Charts of one plot evaluated at sizes with the base stylesheet, and compared with image
//! baselines.
//!
//! A changed image is written to `tests/output/`; accept it by copying it to `tests/baselines/`.

mod bands;
mod compositions;
mod domains;
mod guides;
#[cfg(feature = "geo")]
mod maps;
mod marks;
mod overlays;
mod polar;
mod selections;
mod support;
