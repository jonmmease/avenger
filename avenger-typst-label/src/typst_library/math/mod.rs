//! Ported from crates/typst-library/src/math/mod.rs @ v0.15.1, modified for Avenger.
//!
//! Mathematical formulas.
//!
//! avenger: a partial port so far; the math library adds the elements.

mod equation;
mod style;

pub use self::equation::*;
pub use self::style::*;
