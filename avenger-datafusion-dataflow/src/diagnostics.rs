use std::fmt;

use chrono::{DateTime, Utc};
use datafusion::{arrow::datatypes::SchemaRef, logical_expr::Volatility};

/// Eligibility for retaining completed results across queries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReuseScope {
    Reusable,
    EvaluationLocal,
}

#[derive(Clone, Debug)]
pub struct NodeReport {
    pub scope: String,
    pub name: String,
    pub dependencies: Vec<String>,
    pub inputs: Vec<String>,
    /// Complete schema at this named materialization boundary.
    pub schema: SchemaRef,
    pub has_external_source: bool,
    pub direct_volatility: Volatility,
    pub reuse_scope: ReuseScope,
}

#[derive(Clone, Debug)]
pub struct PrepareReport {
    pub scopes: Vec<ScopeReport>,
    pub nodes: Vec<NodeReport>,
    pub outputs: Vec<String>,
    /// True while physical plans are created per demanded computation and instance.
    pub replans_on_query: bool,
    pub cache_enabled: bool,
}

impl fmt::Display for PrepareReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "Execution: plan cache misses, retention={}",
            self.cache_enabled
        )?;
        for scope in &self.scopes {
            writeln!(
                f,
                "Scope {}: parent={:?}, captures={:?}",
                scope.name, scope.parent, scope.captures
            )?;
        }
        for node in &self.nodes {
            writeln!(
                f,
                "{}::{}: {:?}, dependencies={:?}, inputs={:?}",
                node.scope, node.name, node.reuse_scope, node.dependencies, node.inputs
            )?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct EvaluationReport {
    pub scopes: Vec<ScopeEvaluationReport>,
    pub evaluation_id: u64,
    pub query_start_time: DateTime<Utc>,
    /// Definition-qualified names in execution order, without data-derived keys.
    pub executed_nodes: Vec<String>,
    pub physical_plans: usize,
    pub cache_hits: usize,
    pub cache_misses: usize,
    pub cache_bypasses: usize,
    /// Executed named nodes that contain external scans (not a file-read counter).
    pub source_executions: usize,
    /// Runtime-wide retained charge when this evaluation completed.
    pub retained_bytes: usize,
    /// Conservative cumulative charge for values, partitions, and frame/result metadata.
    pub materialized_bytes: usize,
}

/// One definition's partition schema and captured ancestor dependencies.
#[derive(Clone, Debug)]
pub struct ScopeReport {
    pub name: String,
    pub parent: Option<String>,
    pub key_schema: Option<SchemaRef>,
    pub captures: Vec<String>,
}

/// Aggregate work for a definition, without parameter or key values.
#[derive(Clone, Debug)]
pub struct ScopeEvaluationReport {
    pub name: String,
    pub instances: usize,
    pub executed_nodes: usize,
    pub partitioned_rows: usize,
}
