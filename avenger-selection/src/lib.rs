#![doc = include_str!("../README.md")]

#[cfg(feature = "dataflow")]
mod dataflow;
mod definitions;
mod error;
mod identity;
mod pixels;
mod predicate;
mod query;
mod resolve;
mod state;
mod values;

#[cfg(feature = "dataflow")]
pub use dataflow::{InstalledSelectionQuery, SelectionQueryBinding};
pub use definitions::{
    IntervalPrecision, ProducerDefinition, Projection, Resolution, RowIdentity,
    SelectionDefinition, SelectionKind,
};
pub use error::{Error, Result};
pub use identity::{
    FacetKey, ProducerAddress, ProducerId, ProjectionId, ScopeId, SelectionId, ViewAddress, ViewId,
};
pub use pixels::PixelGrid;
pub use query::{
    BoundQuery, DirectReason, QueryDiagnostics, QueryFamily, QueryFamilyBuilder, QueryPolicy,
    QueryStrategy, SelectionQuery,
};
pub use resolve::{
    ConsumerFilter, EmptySelection, ResolvedContribution, ResolvedFilter, ResolvedProjection,
    ResolvedSelection, SelectionCompiler, SelectionConsumer, SelectionFilter, SelectionMode,
    SelectionStatus, SelectionUse,
};
pub use state::{Contribution, SelectionSet, SelectionSnapshot, SelectionUpdate};
pub use values::{RowIdSelection, SelectionTerm, SelectionTuple, SelectionValue, ValueTest};
