//! Interactive controls for `avenger-chart`.
//!
//! This crate owns the built-in composed and native widgets. The shared
//! coordinate, artifact, evaluation, and hosting contracts live in the lower
//! chart crates so downstream widget crates can use the same infrastructure.

#![forbid(unsafe_code)]

pub mod button;
pub mod checkbox;
pub mod list;
pub mod prelude;
pub mod slider;
pub mod style;
pub mod text_input;

pub use button::{Button, ButtonVariant};
pub use checkbox::Checkbox;
pub use list::{CheckboxList, RadioButtonList};
