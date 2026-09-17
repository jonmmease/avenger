use std::{
    cmp::Ordering,
    fmt,
    hash::{Hash, Hasher},
    sync::Arc,
};

use async_trait::async_trait;
use datafusion::{
    common::{
        metadata::FieldMetadata,
        tree_node::{Transformed, TreeNode},
        DFSchemaRef, Result,
    },
    execution::{context::QueryPlanner, SessionState},
    logical_expr::{
        expr_rewriter::NamePreserver, Expr, Extension, LogicalPlan, UserDefinedLogicalNode,
        UserDefinedLogicalNodeCore,
    },
    physical_plan::ExecutionPlan,
    physical_planner::{DefaultPhysicalPlanner, ExtensionPlanner, PhysicalPlanner},
};
use datafusion_datasource::memory::MemorySourceConfig;

use crate::{
    graph::{
        reference::{GraphRead, ScalarRef, TableRef},
        GraphDef,
    },
    inputs::{InputBinding, MaterializedValue},
    TableSnapshot,
};

#[derive(Clone, Debug)]
pub(crate) struct BoundTable {
    read: GraphRead,
    snapshot: TableSnapshot,
}
impl PartialEq for BoundTable {
    fn eq(&self, other: &Self) -> bool {
        self.read == other.read && self.snapshot.id() == other.snapshot.id()
    }
}
impl Eq for BoundTable {}
impl PartialOrd for BoundTable {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        match self.read.partial_cmp(&other.read) {
            Some(Ordering::Equal) if self.snapshot.id() == other.snapshot.id() => {
                Some(Ordering::Equal)
            }
            Some(Ordering::Equal) => None,
            ordering => ordering,
        }
    }
}
impl Hash for BoundTable {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.read.hash(state);
        self.snapshot.id().hash(state);
    }
}
impl UserDefinedLogicalNodeCore for BoundTable {
    fn name(&self) -> &str {
        "SnapshotRead"
    }
    fn inputs(&self) -> Vec<&LogicalPlan> {
        vec![]
    }
    fn schema(&self) -> &DFSchemaRef {
        &self.read.schema
    }
    fn expressions(&self) -> Vec<Expr> {
        vec![]
    }
    fn fmt_for_explain(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "SnapshotRead: {}", self.read.label)
    }
    fn with_exprs_and_inputs(&self, exprs: Vec<Expr>, inputs: Vec<LogicalPlan>) -> Result<Self> {
        if !exprs.is_empty() || !inputs.is_empty() {
            return datafusion::common::internal_err!("SnapshotRead is a leaf without expressions");
        }
        Ok(self.clone())
    }
}

#[derive(Debug)]
pub(crate) struct GraphQueryPlanner;

#[async_trait]
impl QueryPlanner for GraphQueryPlanner {
    async fn create_physical_plan(
        &self,
        logical_plan: &LogicalPlan,
        session_state: &SessionState,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        DefaultPhysicalPlanner::with_extension_planners(vec![Arc::new(SnapshotPlanner)])
            .create_physical_plan(logical_plan, session_state)
            .await
    }
}

struct SnapshotPlanner;
#[async_trait]
impl ExtensionPlanner for SnapshotPlanner {
    async fn plan_extension(
        &self,
        _planner: &dyn PhysicalPlanner,
        node: &dyn UserDefinedLogicalNode,
        _logical_inputs: &[&LogicalPlan],
        _physical_inputs: &[Arc<dyn ExecutionPlan>],
        _session_state: &SessionState,
    ) -> Result<Option<Arc<dyn ExecutionPlan>>> {
        let Some(table) = node.as_any().downcast_ref::<BoundTable>() else {
            return Ok(None);
        };
        Ok(Some(MemorySourceConfig::try_new_exec(
            &[table.snapshot.batches().to_vec()],
            table.snapshot.schema().clone(),
            None,
        )?))
    }
}

/// Bind captured values into a fresh logical plan. Literal substitution requires
/// physical planning during the current query.
pub(crate) fn bind_plan<'a>(
    plan: LogicalPlan,
    graph: &GraphDef,
    input: impl Fn(usize) -> &'a InputBinding,
    value: impl Fn(usize) -> &'a MaterializedValue,
    base_input: impl Fn(usize) -> &'a InputBinding,
    imported: impl Fn(usize) -> &'a MaterializedValue,
) -> Result<LogicalPlan> {
    plan.transform_up_with_subqueries(|plan| {
        if let LogicalPlan::Extension(extension) = &plan {
            if let Some(read) = extension.node.as_any().downcast_ref::<GraphRead>() {
                let asset;
                let value = match read.source {
                    TableRef::Asset(index) => {
                        asset = MaterializedValue::Table(graph.assets[index].clone());
                        &asset
                    }
                    TableRef::Input(index) => {
                        let binding = if read.graph == graph.id {
                            input(index)
                        } else {
                            base_input(index)
                        };
                        let InputBinding::Value(value) = binding else {
                            unreachable!()
                        };
                        value
                    }
                    TableRef::Node(index) => value(index),
                    TableRef::Import(index) => imported(index),
                    TableRef::Rows(_) => {
                        return datafusion::common::internal_err!(
                            "local rows must be automatically bound before execution"
                        )
                    }
                };
                let MaterializedValue::Table(snapshot) = value else {
                    unreachable!()
                };
                return Ok(Transformed::yes(LogicalPlan::Extension(Extension {
                    node: Arc::new(BoundTable {
                        read: read.clone(),
                        snapshot: snapshot.clone(),
                    }),
                })));
            }
        }
        let names = NamePreserver::new(&plan);
        crate::expr_input::map_context_expressions(plan, |expr, schema| {
            let original_name = names.save(&expr);
            expr.transform_up(|expr| {
                let Expr::Placeholder(placeholder) = &expr else {
                    return Ok(Transformed::no(expr));
                };
                let (source, field) = &graph.placeholders[&placeholder.id];
                let binding = match source {
                    ScalarRef::Input(index) => Some(input(*index)),
                    ScalarRef::BaseInput(index) => Some(base_input(*index)),
                    _ => None,
                };
                let value = if let Some(binding) = binding {
                    match binding {
                        InputBinding::Value(value) => value,
                        InputBinding::Expr(expr) => {
                            let schema = schema.ok_or_else(|| {
                                datafusion::common::DataFusionError::Internal(
                                    "missing expression usage schema".into(),
                                )
                            })?;
                            return Ok(Transformed::yes(expr.at(schema)?));
                        }
                    }
                } else {
                    match source {
                        ScalarRef::Node(index) => value(*index),
                        ScalarRef::Import(index) => imported(*index),
                        _ => unreachable!(),
                    }
                };
                let MaterializedValue::Scalar(value) = value else {
                    unreachable!()
                };
                let metadata =
                    (!field.metadata().is_empty()).then(|| FieldMetadata::from(field.metadata()));
                Ok(Transformed::yes(Expr::Literal(value.clone(), metadata)))
            })
            .map(|transformed| transformed.update_data(|expr| original_name.restore(expr)))
        })
    })
    .map(|transformed| transformed.data)
}
