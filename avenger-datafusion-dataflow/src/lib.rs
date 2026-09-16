#![doc = include_str!("../README.md")]

pub use datafusion;
pub use datafusion::arrow;

mod diagnostics;
mod error;
mod execution;
mod graph;
mod inputs;
mod partition;
mod result;
mod runtime;
mod table;

pub use diagnostics::{
    EvaluationReport, NodeReport, PrepareReport, ReuseScope, ScopeEvaluationReport, ScopeReport,
};
pub use error::{Error, Result};
pub use graph::{
    ExprNode, Graph, GraphBuilder, PlanNode, ScalarInput, ScalarOutput, ScopeBuilder, TableInput,
    TableOutput,
};
pub use inputs::{Inputs, InputsBuilder, ScopedBindingsBuilder};
pub use partition::{PartitionKey, ScopeHandle, ScopeInstance};
pub use result::{GraphResult, ScopeResult, ScopeResults};
pub use runtime::{ExecutionConfig, PreparedGraph, Runtime, RuntimeConfig};
pub use table::{SnapshotId, TableSnapshot, TableStore};

use std::sync::atomic::{AtomicU64, Ordering};

fn fresh_id() -> u64 {
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    NEXT_ID
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
        .expect("dataflow identity space exhausted")
}
