//! Repeat container authoring support.
//!
//! Repeat containers are semantic authoring sugar. During compilation they
//! lower to ordinary concat-family containers with generated `Subplot` marks.

use std::sync::Arc;

use avenger_chart_core::{
    AvengerChartError, AxisGuideVisibilityConfig, AxisGuideVisibilityPolicy, CompileContext,
    CompiledSubplotChildPlot, CoordinateSystem, CoordinateSystemCore, CoordinateSystemTransform,
    CoordinationScope, DefaultLogicalExprNodeExt, FacetWrapColumnMode, IntoExpr,
    RepeatContext as CoreRepeatContext, RepeatVariable as CoreRepeatVariable, SubplotChildPlotSpec,
};
use datafusion::prelude::SessionContext;
use datafusion_proto::protobuf::LogicalExprNode;

use crate::{
    concat::{ConcatGuide, GridConcat, HConcat, VConcat, WrapConcat},
    tools::ToolCompileContext,
};

pub use avenger_chart_core::repeat::*;

#[derive(Clone)]
pub(crate) struct RepeatCellBranch {
    predicate: LogicalExprNode,
    cell: Box<dyn SubplotChildPlotSpec>,
}

#[derive(Clone, Default)]
pub(crate) struct RepeatCellTemplates {
    default: Option<Box<dyn SubplotChildPlotSpec>>,
    branches: Vec<RepeatCellBranch>,
}

impl RepeatCellTemplates {
    pub(crate) fn set_default(&mut self, cell: Box<dyn SubplotChildPlotSpec>) {
        self.default = Some(cell);
    }

    pub(crate) fn add_branch(
        &mut self,
        predicate: impl IntoExpr,
        cell: Box<dyn SubplotChildPlotSpec>,
    ) {
        let predicate = LogicalExprNode::from_default_expr(predicate.into_expr())
            .expect("Failed to serialize repeat cell branch predicate");
        self.branches.push(RepeatCellBranch { predicate, cell });
    }

    pub(crate) fn select(
        &self,
        kind: &str,
        cell_key: &str,
        repeat_context: &CoreRepeatContext,
        session_context: &SessionContext,
    ) -> Result<Box<dyn SubplotChildPlotSpec>, AvengerChartError> {
        let Some(default) = self.default.as_ref() else {
            return Err(AvengerChartError::InvalidArgument(format!(
                "{kind} requires a default repeated child plot via `.cell(...)`"
            )));
        };

        for (branch_index, branch) in self.branches.iter().enumerate() {
            let predicate = branch.predicate.to_default_expr(session_context)?;
            let matches =
                avenger_chart_core::evaluate_repeat_predicate(predicate, repeat_context).map_err(|err| {
                    AvengerChartError::InvalidArgument(format!(
                        "{kind} repeat cell '{cell_key}' branch {branch_index} predicate failed: {err}"
                    ))
                })?;
            if matches {
                return Ok(branch.cell.clone());
            }
        }

        Ok(default.clone())
    }
}

#[derive(Clone, Default)]
pub struct RepeatColumns {
    columns: Vec<CoreRepeatVariable>,
    cells: RepeatCellTemplates,
    domain_coordination: RepeatDomainCoordination,
}

impl RepeatColumns {
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn set_columns(&mut self, columns: Vec<CoreRepeatVariable>) {
        self.columns = columns;
    }

    pub(crate) fn set_cell(&mut self, cell: Box<dyn SubplotChildPlotSpec>) {
        self.cells.set_default(cell);
    }

    pub(crate) fn add_cell_when(
        &mut self,
        predicate: impl IntoExpr,
        cell: Box<dyn SubplotChildPlotSpec>,
    ) {
        self.cells.add_branch(predicate, cell);
    }

    pub(crate) fn columns_config(&self) -> &[CoreRepeatVariable] {
        &self.columns
    }

    pub(crate) fn cell_templates(&self) -> &RepeatCellTemplates {
        &self.cells
    }

    pub(crate) fn set_domain_coordination(&mut self, mode: RepeatDomainCoordination) {
        self.domain_coordination = mode;
    }

    pub(crate) fn domain_coordination_config(&self) -> &RepeatDomainCoordination {
        &self.domain_coordination
    }
}

#[derive(Clone, Default)]
pub struct RepeatRows {
    rows: Vec<CoreRepeatVariable>,
    cells: RepeatCellTemplates,
    domain_coordination: RepeatDomainCoordination,
}

impl RepeatRows {
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn set_rows(&mut self, rows: Vec<CoreRepeatVariable>) {
        self.rows = rows;
    }

    pub(crate) fn set_cell(&mut self, cell: Box<dyn SubplotChildPlotSpec>) {
        self.cells.set_default(cell);
    }

    pub(crate) fn add_cell_when(
        &mut self,
        predicate: impl IntoExpr,
        cell: Box<dyn SubplotChildPlotSpec>,
    ) {
        self.cells.add_branch(predicate, cell);
    }

    pub(crate) fn rows_config(&self) -> &[CoreRepeatVariable] {
        &self.rows
    }

    pub(crate) fn cell_templates(&self) -> &RepeatCellTemplates {
        &self.cells
    }

    pub(crate) fn set_domain_coordination(&mut self, mode: RepeatDomainCoordination) {
        self.domain_coordination = mode;
    }

    pub(crate) fn domain_coordination_config(&self) -> &RepeatDomainCoordination {
        &self.domain_coordination
    }
}

#[derive(Clone, Default)]
pub struct RepeatGrid {
    rows: Vec<CoreRepeatVariable>,
    columns: Vec<CoreRepeatVariable>,
    cells: RepeatCellTemplates,
    domain_coordination: RepeatDomainCoordination,
    axis_guide_visibility: AxisGuideVisibilityConfig,
    matrix_axis_defaults: bool,
}

impl RepeatGrid {
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn set_rows(&mut self, rows: Vec<CoreRepeatVariable>) {
        self.rows = rows;
    }

    pub(crate) fn set_columns(&mut self, columns: Vec<CoreRepeatVariable>) {
        self.columns = columns;
    }

    pub(crate) fn set_cell(&mut self, cell: Box<dyn SubplotChildPlotSpec>) {
        self.cells.set_default(cell);
    }

    pub(crate) fn add_cell_when(
        &mut self,
        predicate: impl IntoExpr,
        cell: Box<dyn SubplotChildPlotSpec>,
    ) {
        self.cells.add_branch(predicate, cell);
    }

    pub(crate) fn rows_config(&self) -> &[CoreRepeatVariable] {
        &self.rows
    }

    pub(crate) fn columns_config(&self) -> &[CoreRepeatVariable] {
        &self.columns
    }

    pub(crate) fn cell_templates(&self) -> &RepeatCellTemplates {
        &self.cells
    }

    pub(crate) fn set_domain_coordination(&mut self, mode: RepeatDomainCoordination) {
        self.domain_coordination = mode;
    }

    pub(crate) fn matrix_domains(&mut self, scope: CoordinationScope) {
        self.domain_coordination = RepeatDomainCoordination::by_variable(scope);
    }

    pub(crate) fn domain_coordination_config(&self) -> &RepeatDomainCoordination {
        &self.domain_coordination
    }

    pub(crate) fn axis_guide_visibility(&mut self, policy: AxisGuideVisibilityPolicy) {
        self.axis_guide_visibility = AxisGuideVisibilityConfig::same(policy);
        self.matrix_axis_defaults = false;
    }

    pub(crate) fn matrix_axes(&mut self) {
        self.axis_guide_visibility = AxisGuideVisibilityConfig::same(
            AxisGuideVisibilityPolicy::OuterForEquivalentDomainGroups,
        );
        self.matrix_axis_defaults = true;
    }

    pub(crate) fn axis_guide_visibility_config(&self) -> AxisGuideVisibilityConfig {
        self.axis_guide_visibility
    }

    pub(crate) fn matrix_axis_defaults(&self) -> bool {
        self.matrix_axis_defaults
    }
}

#[derive(Clone)]
pub struct RepeatWrap {
    items: Vec<CoreRepeatVariable>,
    column_mode: FacetWrapColumnMode,
    cells: RepeatCellTemplates,
    domain_coordination: RepeatDomainCoordination,
}

impl Default for RepeatWrap {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            column_mode: FacetWrapColumnMode::Auto,
            cells: RepeatCellTemplates::default(),
            domain_coordination: RepeatDomainCoordination::Independent,
        }
    }
}

impl RepeatWrap {
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn set_items(&mut self, items: Vec<CoreRepeatVariable>) {
        self.items = items;
    }

    pub(crate) fn set_columns(&mut self, expr: impl IntoExpr) {
        self.column_mode = FacetWrapColumnMode::Fixed(
            LogicalExprNode::from_default_expr(expr.into_expr())
                .expect("Failed to serialize repeat wrap columns expression"),
        );
    }

    pub(crate) fn set_responsive_columns(&mut self, width: impl IntoExpr) {
        self.column_mode = FacetWrapColumnMode::ResponsiveWidth(
            LogicalExprNode::from_default_expr(width.into_expr())
                .expect("Failed to serialize repeat wrap responsive column width"),
        );
    }

    pub(crate) fn set_cell(&mut self, cell: Box<dyn SubplotChildPlotSpec>) {
        self.cells.set_default(cell);
    }

    pub(crate) fn add_cell_when(
        &mut self,
        predicate: impl IntoExpr,
        cell: Box<dyn SubplotChildPlotSpec>,
    ) {
        self.cells.add_branch(predicate, cell);
    }

    pub(crate) fn items_config(&self) -> &[CoreRepeatVariable] {
        &self.items
    }

    pub(crate) fn column_mode_config(&self) -> &FacetWrapColumnMode {
        &self.column_mode
    }

    pub(crate) fn cell_templates(&self) -> &RepeatCellTemplates {
        &self.cells
    }

    pub(crate) fn set_domain_coordination(&mut self, mode: RepeatDomainCoordination) {
        self.domain_coordination = mode;
    }

    pub(crate) fn item_domains(&mut self, scope: CoordinationScope) {
        self.domain_coordination = RepeatDomainCoordination::by_variable(scope);
    }

    pub(crate) fn domain_coordination_config(&self) -> &RepeatDomainCoordination {
        &self.domain_coordination
    }
}

impl CoordinateSystemCore for RepeatColumns {
    fn required_channels(&self) -> &'static [&'static str] {
        &[]
    }
}

impl CoordinateSystem for RepeatColumns {
    type Guide = ConcatGuide;

    fn create_transform(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(HConcat::new())
    }
}

impl CoordinateSystemCore for RepeatRows {
    fn required_channels(&self) -> &'static [&'static str] {
        &[]
    }
}

impl CoordinateSystem for RepeatRows {
    type Guide = ConcatGuide;

    fn create_transform(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(VConcat::new())
    }
}

impl CoordinateSystemCore for RepeatGrid {
    fn required_channels(&self) -> &'static [&'static str] {
        &[]
    }
}

impl CoordinateSystem for RepeatGrid {
    type Guide = ConcatGuide;

    fn create_transform(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(
            GridConcat::new()
                .rows(self.rows.len().max(1))
                .columns(self.columns.len().max(1)),
        )
    }
}

impl CoordinateSystemCore for RepeatWrap {
    fn required_channels(&self) -> &'static [&'static str] {
        &[]
    }
}

impl CoordinateSystem for RepeatWrap {
    type Guide = ConcatGuide;

    fn create_transform(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(WrapConcat::new().with_column_mode(self.column_mode.clone()))
    }
}

pub(crate) struct RepeatResolvedChildPlotSpec {
    inner: Box<dyn SubplotChildPlotSpec>,
    repeat_context: CoreRepeatContext,
}

impl RepeatResolvedChildPlotSpec {
    pub(crate) fn new(
        inner: Box<dyn SubplotChildPlotSpec>,
        repeat_context: CoreRepeatContext,
    ) -> Self {
        Self {
            inner,
            repeat_context,
        }
    }
}

#[async_trait::async_trait]
impl SubplotChildPlotSpec for RepeatResolvedChildPlotSpec {
    fn clone_box(&self) -> Box<dyn SubplotChildPlotSpec> {
        Box::new(Self {
            inner: self.inner.clone(),
            repeat_context: self.repeat_context.clone(),
        })
    }

    fn has_plot_level_data(&self) -> bool {
        self.inner.has_plot_level_data()
    }

    async fn compile_boxed(
        &self,
        session_context: &SessionContext,
    ) -> Result<Arc<dyn CompiledSubplotChildPlot>, AvengerChartError> {
        let tool_context =
            ToolCompileContext::from_parent(None).with_repeat_context(self.repeat_context.clone());
        self.compile_boxed_with_context(session_context, Some(&tool_context as CompileContext<'_>))
            .await
    }

    async fn compile_boxed_with_context(
        &self,
        session_context: &SessionContext,
        compile_context: Option<CompileContext<'_>>,
    ) -> Result<Arc<dyn CompiledSubplotChildPlot>, AvengerChartError> {
        let repeat_tool_context;
        let compile_context =
            if let Some(tool_context) = compile_context.and_then(ToolCompileContext::downcast) {
                repeat_tool_context = tool_context
                    .clone()
                    .with_repeat_context(self.repeat_context.clone());
                Some(&repeat_tool_context as CompileContext<'_>)
            } else {
                repeat_tool_context = ToolCompileContext::from_parent(None)
                    .with_repeat_context(self.repeat_context.clone());
                Some(&repeat_tool_context as CompileContext<'_>)
            };
        self.inner
            .compile_boxed_with_context(session_context, compile_context)
            .await
    }
}
