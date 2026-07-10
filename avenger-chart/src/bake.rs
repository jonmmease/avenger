//! Chart-level baking API.
//!
//! Baking partially evaluates a compiled plot's DataFusion data plans with
//! respect to the params that remain live at runtime. The baked plot embeds
//! materialized, param-independent tables and keeps residual param-bearing work
//! symbolic.

use std::{collections::HashSet, io::Cursor, sync::Arc, time::SystemTime};

use arrow::{
    datatypes::SchemaRef,
    ipc::{reader::StreamReader, writer::StreamWriter},
    record_batch::RecordBatch,
};
use datafusion::{datasource::MemTable, prelude::SessionContext};
use datafusion_common::ScalarValue;
use serde::{Deserialize, Serialize};

use crate::error::AvengerChartError;

/// Controls chart baking budgets and fixed parameter bindings.
#[derive(Clone, Debug, PartialEq)]
pub struct BakePolicy {
    /// Maximum bytes that may be embedded for one baked subtree.
    pub max_baked_bytes_per_subtree: usize,
    /// Maximum bytes that may be embedded across the whole plot bake.
    pub max_baked_bytes_total: usize,
    /// Parameter values to bind before baking.
    pub fixed_params: Vec<(String, ScalarValue)>,
}

impl Default for BakePolicy {
    fn default() -> Self {
        let policy = avenger_datafusion_partial_eval::PartialEvalPolicy::default();
        Self {
            max_baked_bytes_per_subtree: policy.max_baked_bytes_per_subtree,
            max_baked_bytes_total: policy.max_baked_bytes_total,
            fixed_params: Vec::new(),
        }
    }
}

impl BakePolicy {
    pub(crate) fn to_partial_eval_policy(
        &self,
        unfoldable_tables: HashSet<String>,
    ) -> avenger_datafusion_partial_eval::PartialEvalPolicy {
        avenger_datafusion_partial_eval::PartialEvalPolicy {
            max_baked_bytes_per_subtree: self.max_baked_bytes_per_subtree,
            max_baked_bytes_total: self.max_baked_bytes_total,
            fixed_params: self.fixed_params.clone(),
            unfoldable_tables,
        }
    }
}

/// A data context addressed by a plot-level bake report.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BakeContextId {
    /// The plot-level data plan.
    PlotData,
    /// A compiled mark-group data context.
    MarkGroup {
        /// Index into the compiled plot's mark-group list.
        index: usize,
        /// Optional author-provided group id.
        id: Option<String>,
    },
    /// The plot-level data plan of a child plot reached through subplot mark
    /// indices from the baked plot root.
    ChildPlotData {
        /// Mark-index path through nested subplot payloads.
        subplot_path: Vec<usize>,
    },
    /// A mark-group data context inside a child plot reached through subplot
    /// mark indices from the baked plot root.
    ChildMarkGroup {
        /// Mark-index path through nested subplot payloads.
        subplot_path: Vec<usize>,
        /// Index into the child plot's mark-group list.
        index: usize,
        /// Optional author-provided group id.
        id: Option<String>,
    },
}

/// How a baked context was emitted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EmitForm {
    /// Residual DataFusion logical plan serialized as protobuf.
    Proto,
}

/// Why a data context was not baked.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NotBakedReason {
    /// No data plan was available for this context. For mark groups this
    /// means the group inherits the parent plot's data (which is baked, or
    /// not, through the `PlotData` context).
    NoData,
    /// The context reads mutable store state, which must stay live.
    StoreData,
    /// A transform stage is not audited as plan-pure.
    PlanBreakStage {
        /// Stage index in the context's transform list.
        stage_index: usize,
        /// Serialized typetag name when available.
        stage_type: Option<String>,
    },
    /// The transform chain produced or consumed derived scalars.
    DerivedScalars,
    /// The group inherits facet-partitioned data and its transform chain must
    /// keep evaluating per facet scope at runtime (shared-scale domains
    /// evaluate the chain at the sharing-owner scope, which a pre-grouped
    /// table cannot reproduce). The chain's base data is served by the
    /// enclosing plot's `PlotData`/`ChildPlotData` bake.
    FacetScopedTransforms,
    /// Assembly failed before partial evaluation could run.
    AssemblyError {
        /// Error message.
        message: String,
    },
    /// The base scan for this context was not folded into exactly one table.
    BaseNotFolded {
        /// Subtree skip reasons reported by the partial-evaluation crate.
        skipped: Vec<String>,
    },
    /// More than one baked table contained the context's base scan.
    MultiplePrimaryTables {
        /// Matching baked table names.
        table_names: Vec<String>,
    },
}

/// Per-context bake status.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContextBakeStatus {
    /// The context was emitted in baked form.
    Baked {
        /// Context identifier.
        context_id: BakeContextId,
        /// Emission form used for this context.
        emit_form: EmitForm,
        /// Baked table that consumed this context's base scan.
        primary_table: String,
        /// Whether the residual references only baked tables or inline values.
        self_contained: bool,
    },
    /// The original context was preserved.
    NotBaked {
        /// Context identifier.
        context_id: BakeContextId,
        /// Reason the context stayed unchanged.
        reason: NotBakedReason,
    },
}

/// A fixed parameter that was applied during bake.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixedParamBinding {
    /// Parameter name, without adding or removing a `$` prefix.
    pub name: String,
    /// Debug rendering of the value applied before baking.
    pub value: String,
}

/// Report stamped onto a baked plot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlotBakeReport {
    /// Time at which the bake ran.
    pub as_of: SystemTime,
    /// Source table names folded away by the partial-evaluation pass.
    pub source_tables: Vec<String>,
    /// Fixed params that matched at least one placeholder.
    pub fixed_params_applied: Vec<FixedParamBinding>,
    /// Fixed params that matched no placeholder.
    pub unused_fixed_params: Vec<String>,
    /// Placeholder ids still present in baked residuals.
    pub remaining_params: Vec<String>,
    /// Per-context status entries.
    pub contexts: Vec<ContextBakeStatus>,
    /// Whether every baked residual is self-contained.
    pub self_contained: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct BakedTableManifestEntry {
    pub(crate) name: String,
    pub(crate) arrow_ipc_bytes: Vec<u8>,
    pub(crate) rows: usize,
    pub(crate) bytes: usize,
}

impl BakedTableManifestEntry {
    pub(crate) fn from_batches(
        name: String,
        schema: SchemaRef,
        batches: &[RecordBatch],
        rows: usize,
        bytes: usize,
    ) -> Result<Self, AvengerChartError> {
        let mut arrow_ipc_bytes = Vec::new();
        {
            let mut writer = StreamWriter::try_new(&mut arrow_ipc_bytes, &schema)?;
            for batch in batches {
                writer.write(batch)?;
            }
            writer.finish()?;
        }
        Ok(Self {
            name,
            arrow_ipc_bytes,
            rows,
            bytes,
        })
    }

    pub(crate) fn mem_table(&self) -> Result<Arc<MemTable>, AvengerChartError> {
        let cursor = Cursor::new(&self.arrow_ipc_bytes);
        let mut reader = StreamReader::try_new(cursor, None)?;
        let schema = reader.schema();
        let batches = reader
            .by_ref()
            .collect::<Result<Vec<_>, arrow::error::ArrowError>>()?;
        Ok(Arc::new(MemTable::try_new(schema, vec![batches])?))
    }
}

pub(crate) fn register_baked_tables(
    ctx: &SessionContext,
    entries: &[BakedTableManifestEntry],
) -> Result<(), AvengerChartError> {
    for entry in entries {
        let _ = ctx.deregister_table(&entry.name);
        ctx.register_table(&entry.name, entry.mem_table()?)?;
    }
    Ok(())
}
