#![doc = include_str!("../README.md")]

mod definitions;
mod error;
mod identity;
mod pixels;
mod predicate;
mod resolve;
mod split;
mod state;
mod values;

pub use definitions::{ProducerDefinition, Projection, Resolution};
pub use error::{Error, Result};
pub use identity::{ProducerId, ProjectionId, SelectionId, ViewId};
pub use pixels::PixelGrid;
pub use resolve::{ConsumerFilter, EmptySelection, SelectionFilter, SelectionMode};
pub use split::{PredicateSplit, SelectionPredicates, SplitReason};
pub use state::{Contribution, SelectionSet, SelectionUpdate};
pub use values::{SelectionValue, ValueTest};
