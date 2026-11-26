//! The decoder of a page's physical plans. Each DataFusion operator decodes its own message, and
//! the decoder dispatches only the operators that the plans of charts hold, so a page links the
//! decoders of those operators and not of DataFusion's others, its sources of files, or its sinks.
//! The one scan it decodes is of rows that the plan holds, such as a `VALUES` list, which a graph
//! keeps in the node that reads it.
//! Expressions decode with datafusion-proto's parsers. An operator outside the set fails to decode
//! with its name, which the export of a page reports.

use std::sync::Arc;

use arrow::datatypes::Schema;
use datafusion_common::{internal_datafusion_err, not_impl_err, Result};
use datafusion_datasource::memory::MemorySourceConfig;
use datafusion_execution::TaskContext;
use datafusion_expr::{
    physical_planning_context::ScalarSubqueryResults, registry::FunctionRegistry, AggregateUDF,
    ScalarUDF, WindowUDF,
};
use datafusion_physical_expr::PhysicalExpr;
#[expect(
    deprecated,
    reason = "`CoalesceBatchesExec` remains in the plans that DataFusion encodes"
)]
use datafusion_physical_plan::coalesce_batches::CoalesceBatchesExec;
use datafusion_physical_plan::{
    aggregates::AggregateExec,
    coalesce_partitions::CoalescePartitionsExec,
    coop::CooperativeExec,
    empty::EmptyExec,
    filter::FilterExec,
    joins::{CrossJoinExec, HashJoinExec, NestedLoopJoinExec},
    limit::{GlobalLimitExec, LocalLimitExec},
    placeholder_row::PlaceholderRowExec,
    projection::ProjectionExec,
    proto::{ExecutionPlanDecode, ExecutionPlanDecodeCtx},
    scalar_subquery::ScalarSubqueryExec,
    sorts::{sort::SortExec, sort_preserving_merge::SortPreservingMergeExec},
    union::UnionExec,
    unnest::UnnestExec,
    windows::WindowAggExec,
    ExecutionPlan,
};
use datafusion_proto::{
    physical_plan::{
        PhysicalExtensionCodec, PhysicalPlanDecodeContext, PhysicalProtoConverterExtension,
    },
    protobuf::{physical_plan_node::PhysicalPlanType, PhysicalExprNode, PhysicalPlanNode},
};

use crate::codec::GraphCodec;

/// The physical plan of `node`, decoded in `ctx` with the graph's codec.
pub fn decode(node: &PhysicalPlanNode, ctx: &TaskContext) -> Result<Arc<dyn ExecutionPlan>> {
    let context = PhysicalPlanDecodeContext::new(ctx, &GraphCodec);
    Page.proto_to_execution_plan(node, &context)
}

/// The converter of a page: plans through [`plan`], and expressions through datafusion-proto's
/// parsers. It encodes nothing, so the trait's encoders and its decoder of every plan node never
/// link.
struct Page;

impl PhysicalProtoConverterExtension for Page {
    fn proto_to_execution_plan(
        &self,
        node: &PhysicalPlanNode,
        ctx: &PhysicalPlanDecodeContext<'_>,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        plan(node, ctx)
    }

    fn execution_plan_to_proto(
        &self,
        _plan: &Arc<dyn ExecutionPlan>,
        _codec: &dyn PhysicalExtensionCodec,
    ) -> Result<PhysicalPlanNode> {
        not_impl_err!("a page encodes no plan")
    }

    fn proto_to_physical_expr(
        &self,
        node: &PhysicalExprNode,
        input_schema: &Schema,
        ctx: &PhysicalPlanDecodeContext<'_>,
    ) -> Result<Arc<dyn PhysicalExpr>> {
        self.default_proto_to_physical_expr(node, input_schema, ctx)
    }

    fn physical_expr_to_proto(
        &self,
        _expr: &Arc<dyn PhysicalExpr>,
        _codec: &dyn PhysicalExtensionCodec,
    ) -> Result<PhysicalExprNode> {
        not_impl_err!("a page encodes no expression")
    }
}

/// The plan of `node`: an operator of the plans of charts, or an extension of the graph's codec.
fn plan(
    node: &PhysicalPlanNode,
    ctx: &PhysicalPlanDecodeContext<'_>,
) -> Result<Arc<dyn ExecutionPlan>> {
    let kind = node
        .physical_plan_type
        .as_ref()
        .ok_or_else(|| internal_datafusion_err!("a plan node without its kind"))?;
    let decoder = Decoder { ctx };
    let decode = ExecutionPlanDecodeCtx::new(&decoder);
    #[expect(
        deprecated,
        reason = "`CoalesceBatchesExec` remains in the plans that DataFusion encodes"
    )]
    match kind {
        PhysicalPlanType::Projection(_) => ProjectionExec::try_from_proto(node, &decode),
        PhysicalPlanType::Filter(_) => FilterExec::try_from_proto(node, &decode),
        PhysicalPlanType::Aggregate(_) => AggregateExec::try_from_proto(node, &decode),
        PhysicalPlanType::Window(_) => WindowAggExec::try_from_proto(node, &decode),
        PhysicalPlanType::Sort(_) => SortExec::try_from_proto(node, &decode),
        PhysicalPlanType::SortPreservingMerge(_) => {
            SortPreservingMergeExec::try_from_proto(node, &decode)
        }
        PhysicalPlanType::GlobalLimit(_) => GlobalLimitExec::try_from_proto(node, &decode),
        PhysicalPlanType::LocalLimit(_) => LocalLimitExec::try_from_proto(node, &decode),
        PhysicalPlanType::Union(_) => UnionExec::try_from_proto(node, &decode),
        PhysicalPlanType::CrossJoin(_) => CrossJoinExec::try_from_proto(node, &decode),
        PhysicalPlanType::HashJoin(_) => HashJoinExec::try_from_proto(node, &decode),
        PhysicalPlanType::NestedLoopJoin(_) => NestedLoopJoinExec::try_from_proto(node, &decode),
        PhysicalPlanType::Unnest(_) => UnnestExec::try_from_proto(node, &decode),
        PhysicalPlanType::PlaceholderRow(_) => PlaceholderRowExec::try_from_proto(node, &decode),
        PhysicalPlanType::Empty(_) => EmptyExec::try_from_proto(node, &decode),
        PhysicalPlanType::MemoryScan(_) => MemorySourceConfig::try_from_proto(node, &decode),
        PhysicalPlanType::ScalarSubquery(_) => ScalarSubqueryExec::try_from_proto(node, &decode),
        PhysicalPlanType::Merge(_) => CoalescePartitionsExec::try_from_proto(node, &decode),
        PhysicalPlanType::CoalesceBatches(_) => CoalesceBatchesExec::try_from_proto(node, &decode),
        PhysicalPlanType::Cooperative(_) => CooperativeExec::try_from_proto(node, &decode),
        PhysicalPlanType::Extension(extension) => {
            let inputs = (extension.inputs.iter())
                .map(|input| plan(input, ctx))
                .collect::<Result<Vec<_>>>()?;
            ctx.codec()
                .try_decode(&extension.node, &inputs, ctx.task_ctx(), &Page)
        }
        other => {
            let kind = format!("{other:?}");
            let name = kind.split('(').next().unwrap_or_default();
            not_impl_err!("a page does not run the plan node {name}")
        }
    }
}

/// The calls of an operator's decoder back into the page's: its children, its expressions, and its
/// functions, which resolve by name in the page's registry before the graph's codec.
struct Decoder<'a, 'ctx> {
    ctx: &'a PhysicalPlanDecodeContext<'ctx>,
}

impl ExecutionPlanDecode for Decoder<'_, '_> {
    fn decode_plan(&self, node: &PhysicalPlanNode) -> Result<Arc<dyn ExecutionPlan>> {
        plan(node, self.ctx)
    }

    fn decode_plan_with_scalar_subquery_results(
        &self,
        node: &PhysicalPlanNode,
        results: ScalarSubqueryResults,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        plan(node, &self.ctx.with_scalar_subquery_results(results))
    }

    fn decode_expr(
        &self,
        node: &PhysicalExprNode,
        input_schema: &Schema,
    ) -> Result<Arc<dyn PhysicalExpr>> {
        Page.proto_to_physical_expr(node, input_schema, self.ctx)
    }

    fn task_ctx(&self) -> &TaskContext {
        self.ctx.task_ctx()
    }

    fn decode_udf(&self, name: &str, payload: Option<&[u8]>) -> Result<Arc<ScalarUDF>> {
        let codec = self.ctx.codec();
        match payload {
            Some(bytes) => codec.try_decode_udf(name, bytes),
            None => (self.ctx.task_ctx().udf(name)).or_else(|_| codec.try_decode_udf(name, &[])),
        }
    }

    fn decode_udaf(&self, name: &str, payload: Option<&[u8]>) -> Result<Arc<AggregateUDF>> {
        let codec = self.ctx.codec();
        match payload {
            Some(bytes) => codec.try_decode_udaf(name, bytes),
            None => (self.ctx.task_ctx().udaf(name)).or_else(|_| codec.try_decode_udaf(name, &[])),
        }
    }

    fn decode_udwf(&self, name: &str, payload: Option<&[u8]>) -> Result<Arc<WindowUDF>> {
        let codec = self.ctx.codec();
        match payload {
            Some(bytes) => codec.try_decode_udwf(name, bytes),
            None => (self.ctx.task_ctx().udwf(name)).or_else(|_| codec.try_decode_udwf(name, &[])),
        }
    }
}
