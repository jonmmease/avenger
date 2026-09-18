#![doc = include_str!("../README.md")]

#[cfg(test)]
extern crate self as avenger_datafusion_dataflow;

pub use datafusion;
pub use datafusion::arrow;

mod cache;
mod diagnostics;
mod error;
mod execution;
mod expr_input;
mod graph;
mod in_flight;
mod inputs;
mod interface;
mod partition;
mod result;
mod runtime;
mod semantics;
mod serialization;
mod table;

pub use cache::{CacheConfig, CachePolicy, CacheStats};
pub use datafusion_proto::logical_plan::{DefaultLogicalExtensionCodec, LogicalExtensionCodec};
pub use diagnostics::{
    EvaluationReport, ImportReport, NodeReport, PrepareReport, ReuseScope, ScopeEvaluationReport,
    ScopeReport,
};
pub use error::{Error, Result};
pub use graph::{
    Dataflow, DataflowBuilder, ExprInput, PlanNode, ScalarInput, ScalarNode, ScalarOutput,
    ScopeBuilder, TableInput, TableOutput,
};
pub use inputs::{Inputs, InputsBuilder, ScopedBindingsBuilder};
pub use interface::{DataflowInterface, InputHandle, ScopeInterface};
pub use partition::{PartitionKey, ScopeHandle, ScopeInstance};
pub use result::{DataflowResult, ScopeResult, ScopeResults};
pub use runtime::{ExecutionConfig, PreparedDataflow, PreparedExtension, Runtime, RuntimeConfig};
pub use semantics::SemanticConfig;
pub use table::{SnapshotId, TableSnapshot, TableStore};

use std::sync::atomic::{AtomicU64, Ordering};

fn fresh_id() -> u64 {
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    NEXT_ID
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
        .expect("dataflow identity space exhausted")
}

#[cfg(feature = "json")]
pub mod json;
