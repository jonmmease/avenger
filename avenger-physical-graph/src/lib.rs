//! The physical side of avenger's queries, which runs natively and in WebAssembly pages: the
//! DataFusion functions that build charts in physical plans, and the leaves through which a node's
//! physical plan reads the results of the nodes below it.
//!
//! The crate depends on DataFusion's sub-crates and not on the `datafusion` crate, so its
//! dependencies name the parts of DataFusion that a page needs.

pub mod cut;
pub mod functions;
