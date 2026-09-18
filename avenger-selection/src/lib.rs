#![doc = include_str!("../README.md")]

mod definitions;
mod error;
mod identity;
mod predicate;
mod resolve;
mod state;
mod values;

pub use definitions::{
    IntervalPrecision, ProducerDefinition, Projection, Resolution, RowIdentity,
    SelectionDefinition, SelectionKind,
};
pub use error::{Error, Result};
pub use identity::{
    FacetKey, ProducerAddress, ProducerId, ProjectionId, ScopeId, SelectionId, ViewAddress, ViewId,
};
pub use resolve::{
    ConsumerFilter, EmptySelection, ResolvedContribution, ResolvedFilter, ResolvedProjection,
    ResolvedSelection, SelectionCompiler, SelectionConsumer, SelectionFilter, SelectionMode,
    SelectionStatus, SelectionUse,
};
pub use state::{Contribution, SelectionSet, SelectionSnapshot, SelectionUpdate};
pub use values::{RowIdSelection, SelectionTerm, SelectionTuple, SelectionValue, ValueTest};
