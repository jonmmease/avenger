use std::fmt;

use chrono::{DateTime, Utc};
use datafusion::logical_expr::Volatility;

/// Eligibility for future cross-query result reuse. This evaluator retains no results.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReuseScope {
    Reusable,
    EvaluationLocal,
}

#[derive(Clone, Debug)]
pub struct NodeReport {
    pub name: String,
    pub dependencies: Vec<String>,
    pub inputs: Vec<String>,
    pub direct_volatility: Volatility,
    pub reuse_scope: ReuseScope,
}

#[derive(Clone, Debug)]
pub struct PrepareReport {
    pub nodes: Vec<NodeReport>,
    pub outputs: Vec<String>,
    /// True for the phase 2 evaluator, which plans each demanded node per query.
    pub replans_on_query: bool,
}

impl fmt::Display for PrepareReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "Execution: plan each demanded node per query, no retained results"
        )?;
        for node in &self.nodes {
            writeln!(
                f,
                "{}: {:?}, dependencies={:?}, inputs={:?}",
                node.name, node.reuse_scope, node.dependencies, node.inputs
            )?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct EvaluationReport {
    pub evaluation_id: u64,
    pub query_start_time: DateTime<Utc>,
    /// In dependency order, with each demanded node present once.
    pub executed_nodes: Vec<String>,
    pub physical_plans: usize,
    /// Conservative charge for completed batches and scalar values in this query.
    pub materialized_bytes: usize,
}
