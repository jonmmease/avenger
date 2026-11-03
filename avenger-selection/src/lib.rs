#![doc = include_str!("../README.md")]

mod clause;
mod condition;
mod error;
mod grid;
mod hash;
mod predicate;
mod selected;
mod set;
mod state;

pub use clause::Clause;
pub use condition::{Condition, PreparedCondition};
pub use error::SelectionError;
pub use grid::{CellSpan, PixelGrid, PolygonCells};
pub use predicate::{Predicate, Resolution, Selected};
pub use selected::{PreparedPredicate, SelectedOptions};
pub use state::SelectionState;

/// The result of the crate's fallible functions.
pub type Result<T> = std::result::Result<T, SelectionError>;
