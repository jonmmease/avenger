//! The physical side of avenger's query graphs, which runs natively and in WebAssembly pages: the
//! DataFusion functions that build charts in physical plans, the leaves and kept results through
//! which a node's plan reads the nodes below it, the parts of nodes' keys that selections give, the
//! codec of a graph's plans, and a graph's file and its runs in a page.
//!
//! The crate depends on DataFusion's sub-crates and not on the `datafusion` crate, so its
//! dependencies name the parts of DataFusion that a page needs.

pub mod codec;
pub mod cut;
pub mod functions;
pub mod kept;
pub mod keys;
pub mod page;
