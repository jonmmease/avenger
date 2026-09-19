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

pub use definitions::{
    IntervalPrecision, ProducerDefinition, Projection, Resolution, RowIdentity, SelectionKind,
};
pub use error::{Error, Result};
pub use identity::{
    FacetKey, ProducerAddress, ProducerId, ProjectionId, ScopeId, SelectionId, ViewAddress, ViewId,
};
pub use pixels::PixelGrid;
pub use resolve::{ConsumerFilter, EmptySelection, SelectionFilter, SelectionMode, SelectionUse};
pub use split::{PredicateSplit, SelectionPredicates, SplitReason};
pub use state::{Contribution, SelectionSet, SelectionUpdate};
pub use values::{RowIdSelection, SelectionValue, ValueTest};
