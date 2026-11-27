//! The physical side of avenger's queries, which runs natively and in WebAssembly pages: the
//! DataFusion functions that build charts in physical plans.
//!
//! The crate depends on DataFusion's sub-crates and not on the `datafusion` crate, so its
//! dependencies name the parts of DataFusion that a page needs.

pub mod functions;
