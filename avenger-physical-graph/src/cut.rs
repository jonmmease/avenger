//! The leaves of a node's physical plan that read the results of the nodes below it. A host
//! that plans a node once gives each leaf a scan of the computation's result before each
//! computation, so the same plan runs over each computation's inputs.

use std::{fmt, sync::Arc};

use arrow::datatypes::SchemaRef;
use datafusion_common::{internal_err, tree_node::TreeNodeRecursion, Result};
use datafusion_execution::{SendableRecordBatchStream, TaskContext};
use datafusion_physical_expr::PhysicalExpr;
use datafusion_physical_plan::{DisplayAs, DisplayFormatType, ExecutionPlan, PlanProperties};

/// How a cut's leaf reads its input's result: the schema of the input's columns, named by
/// position, and the projection and the limit of the scan.
#[derive(Clone, Debug)]
pub struct CutScan {
    /// The position of the node below the cut among the inputs of the node above it.
    pub input: usize,
    pub schema: SchemaRef,
    pub projection: Option<Vec<usize>>,
    pub limit: Option<usize>,
}

/// A leaf that reads the result of the node below a cut through `scan`, the plan that scans that
/// result. A run replaces `scan` with a scan of each computation's result, which holds the
/// partitioning and the ordering that the plan was planned with. The leaf has no children, so the
/// optimizers, the walks of a plan, and its codec stop at it, and its scan never travels with the
/// plan.
#[derive(Debug)]
pub struct CutInputExec {
    cut: CutScan,
    scan: Arc<dyn ExecutionPlan>,
    properties: Arc<PlanProperties>,
}

impl CutInputExec {
    pub fn new(cut: CutScan, scan: Arc<dyn ExecutionPlan>) -> Self {
        let properties = Arc::new(scan.properties().as_ref().clone());
        Self {
            cut,
            scan,
            properties,
        }
    }

    /// How the leaf reads its input's result.
    pub fn cut(&self) -> &CutScan {
        &self.cut
    }

    /// The plan that scans the input's result.
    pub fn scan(&self) -> &Arc<dyn ExecutionPlan> {
        &self.scan
    }
}

impl DisplayAs for CutInputExec {
    fn fmt_as(&self, _: DisplayFormatType, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "CutInputExec: input={}", self.cut.input)
    }
}

impl ExecutionPlan for CutInputExec {
    fn name(&self) -> &'static str {
        "CutInputExec"
    }

    fn properties(&self) -> &Arc<PlanProperties> {
        &self.properties
    }

    fn children(&self) -> Vec<&Arc<dyn ExecutionPlan>> {
        vec![]
    }

    fn apply_expressions(
        &self,
        _f: &mut dyn FnMut(&Arc<dyn PhysicalExpr>) -> Result<TreeNodeRecursion>,
    ) -> Result<TreeNodeRecursion> {
        Ok(TreeNodeRecursion::Continue)
    }

    fn with_new_children(
        self: Arc<Self>,
        children: Vec<Arc<dyn ExecutionPlan>>,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        match children.is_empty() {
            true => Ok(self),
            false => internal_err!("a cut's leaf has no children"),
        }
    }

    fn execute(
        &self,
        partition: usize,
        context: Arc<TaskContext>,
    ) -> Result<SendableRecordBatchStream> {
        self.scan.execute(partition, context)
    }
}
