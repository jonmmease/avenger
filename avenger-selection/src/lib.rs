#![doc = include_str!("../README.md")]

mod clause;
mod condition;
mod error;
mod grid;
mod matching;
mod state;

pub use clause::Clause;
pub use condition::Condition;
pub use error::SelectionError;
pub use grid::{CellSpan, PixelGrid, PolygonCells};
pub use matching::{matching, Matching, Predicate, Resolution, Selected};
pub use state::SelectionState;

/// The result of the crate's fallible functions.
pub type Result<T> = std::result::Result<T, SelectionError>;
