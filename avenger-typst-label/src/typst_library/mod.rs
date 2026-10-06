//! Ported from crates/typst-library/src/lib.rs @ v0.15.1, modified for Avenger.
//!
//! Typst's standard library: the subset a single label line uses.

pub mod diag;
pub mod foundations;
pub mod layout;
pub mod math;
pub mod text;
pub mod visualize;

#[cfg(test)]
mod tests;
