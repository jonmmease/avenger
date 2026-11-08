//! Charts of one plot evaluated at sizes with the base stylesheet, and compared with image
//! baselines.
//!
//! A changed image is written to `tests/output/`; accept it by copying it to `tests/baselines/`.

mod bands;
mod compositions;
mod domains;
#[cfg(feature = "geo")]
mod geo;
mod guides;
mod marks;
mod overlays;
mod polar;
mod selections;
mod state;
mod support;
mod widgets;
