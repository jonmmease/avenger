use std::{collections::HashSet, sync::Arc, time::SystemTime};

use arrow::{datatypes::SchemaRef, record_batch::RecordBatch};
use datafusion::datasource::MemTable;
use datafusion_common::ScalarValue;

const DEFAULT_MAX_BAKED_BYTES_PER_SUBTREE: usize = 64 * 1024 * 1024;
const DEFAULT_MAX_BAKED_BYTES_TOTAL: usize = 256 * 1024 * 1024;

/// Controls which subtrees may be baked and how much data may be embedded.
#[derive(Clone, Debug, PartialEq)]
pub struct PartialEvalPolicy {
    /// Maximum bytes that may be baked for one subtree.
    pub max_baked_bytes_per_subtree: usize,
    /// Maximum bytes that may be baked across one evaluation call.
    pub max_baked_bytes_total: usize,
    /// Placeholder values to bind before folding.
    pub fixed_params: Vec<(String, ScalarValue)>,
    /// Resolved table names that must stay symbolic.
    pub unfoldable_tables: HashSet<String>,
}

impl Default for PartialEvalPolicy {
    fn default() -> Self {
        Self {
            max_baked_bytes_per_subtree: DEFAULT_MAX_BAKED_BYTES_PER_SUBTREE,
            max_baked_bytes_total: DEFAULT_MAX_BAKED_BYTES_TOTAL,
            fixed_params: Vec::new(),
            unfoldable_tables: HashSet::new(),
        }
    }
}

/// Result of partially evaluating one logical plan.
pub struct PartialEvalOutput {
    /// Rewritten residual plan.
    pub residual: datafusion::logical_expr::LogicalPlan,
    /// Report describing folded and skipped subtrees.
    pub report: BakeReport,
}

/// Report describing a partial-evaluation pass.
#[derive(Clone)]
pub struct BakeReport {
    /// Subtrees that were materialized into in-memory tables.
    pub baked: Vec<BakedSubtree>,
    /// Candidate subtrees that were left symbolic.
    pub skipped: Vec<SkippedSubtree>,
    /// Source table names folded away in v1.
    pub source_tables: Vec<String>,
    /// Fixed params that matched at least one placeholder.
    pub fixed_params_applied: Vec<(String, ScalarValue)>,
    /// Fixed params that matched no placeholder in the evaluated plan set.
    pub unused_fixed_params: Vec<String>,
    /// Placeholder ids still present in the residual plan set.
    pub remaining_params: Vec<String>,
    /// Time at which the report was produced.
    pub as_of: SystemTime,
}

impl BakeReport {
    /// Build an empty report stamped with the current system time.
    pub fn empty() -> Self {
        Self {
            baked: Vec::new(),
            skipped: Vec::new(),
            source_tables: Vec::new(),
            fixed_params_applied: Vec::new(),
            unused_fixed_params: Vec::new(),
            remaining_params: Vec::new(),
            as_of: SystemTime::now(),
        }
    }
}

/// A subtree that was materialized during partial evaluation.
#[derive(Clone)]
pub struct BakedSubtree {
    /// Name of the generated in-memory table scan.
    pub table_name: String,
    /// Source table names folded into this materialized subtree.
    pub source_tables: Vec<String>,
    /// Materialized table provider.
    pub mem_table: Arc<MemTable>,
    /// Arrow schema of the materialized batches.
    pub schema: SchemaRef,
    /// Materialized record batches.
    pub batches: Vec<RecordBatch>,
    /// Total row count across all batches.
    pub rows: usize,
    /// Approximate byte count across all batches.
    pub bytes: usize,
    /// Number of folded occurrences sharing this table.
    pub occurrences: usize,
    /// Human-readable display of the folded subtree.
    pub subtree_display: String,
}

/// A subtree that could not be baked.
#[derive(Clone, Debug, PartialEq)]
pub struct SkippedSubtree {
    /// Reason this subtree stayed symbolic.
    pub reason: SkipReason,
    /// Human-readable display of the skipped subtree.
    pub subtree_display: String,
}

/// Reason a candidate subtree stayed symbolic.
#[derive(Clone, Debug, PartialEq)]
pub enum SkipReason {
    /// The subtree contains at least one placeholder.
    ContainsPlaceholders,
    /// The subtree contains a volatile expression.
    Volatile,
    /// The subtree contains a now-family temporal function.
    TemporalFunction,
    /// The subtree scans a policy-excluded table.
    ExcludedTable,
    /// Baking this subtree exceeded the per-subtree budget.
    OverBudget {
        /// Bytes observed before aborting the bake.
        observed_bytes: usize,
    },
    /// The shared total budget was exhausted.
    TotalBudgetExhausted,
    /// Executing the subtree failed; the residual keeps the subtree symbolic.
    ExecutionError {
        /// Error text from execution.
        message: String,
    },
}
