#![doc = include_str!("../README.md")]

pub mod chart_graph;
mod cut;
mod dot;
mod graph;
mod key;
pub mod proto {
    //! The prost messages of package `avenger.graph.v1`, the messages of the file of a baked
    //! graph, generated from `proto/avenger/graph/v1/graph.proto`, whose comments are their
    //! documentation.
    include!(concat!(env!("OUT_DIR"), "/avenger.graph.v1.rs"));
}
mod reads;
mod run;
mod sources;
mod split;
mod store;
mod task;

pub use graph::{QueryGraph, QueryNode};
pub use run::{Inputs, Outcome, Output, Run, RunKey};
pub use store::{ResultStore, SourceOptions, StoreConfig};
