//! Repeat container authoring support.
//!
//! Repeat containers are semantic authoring sugar. During compilation they
//! lower to ordinary concat-family containers with generated `Subplot` marks.

use std::sync::Arc;

use avenger_chart_core::{
    AvengerChartError, AxisGuideVisibilityConfig, AxisGuideVisibilityPolicy, ChildPlotFurnishings,
    ChildPlotSizeSpec, CompileContext, CompiledSubplotChildPlot, CoordinateSystem,
    CoordinateSystemCore, CoordinateSystemTransform, CoordinationScope, DefaultLogicalExprNodeExt,
    FacetWrapColumnMode, IntoExpr, RepeatContext as CoreRepeatContext,
    RepeatVariable as CoreRepeatVariable, SubplotChildPlotSpec, TitleSpec,
};
use datafusion::prelude::SessionContext;
use datafusion_proto::protobuf::LogicalExprNode;

use crate::{
    concat::{ConcatGuide, ConcatOrigin, GridConcat, HConcat, VConcat, WrapConcat},
    plot::Plot,
    tools::ToolCompileContext,
};

pub use avenger_chart_core::repeat::*;

/// A repeated child Plot together with its position-aware furnishings.
#[derive(Clone)]
pub struct RepeatCell<C: CoordinateSystem> {
    plot: Plot<C>,
    furnishings: ChildPlotFurnishings,
}

impl<C: CoordinateSystem> RepeatCell<C> {
    pub fn new(plot: Plot<C>) -> Self {
        Self {
            plot,
            furnishings: ChildPlotFurnishings::default(),
        }
    }

    pub fn caption(mut self, text: impl IntoExpr) -> Self {
        self.furnishings.caption = Some(TitleSpec::new(text));
        self
    }

    pub fn configure_caption<F>(mut self, text: impl IntoExpr, f: F) -> Self
    where
        F: FnOnce(TitleSpec) -> TitleSpec,
    {
        self.furnishings.caption = Some(f(TitleSpec::new(text)));
        self
    }

    pub fn size(mut self, width: impl IntoExpr, height: impl IntoExpr) -> Self {
        self.furnishings.size = ChildPlotSizeSpec::default().width(width).height(height);
        self
    }

    pub fn configure_size<F>(mut self, f: F) -> Self
    where
        F: FnOnce(ChildPlotSizeSpec) -> ChildPlotSizeSpec,
    {
        self.furnishings.size = f(std::mem::take(&mut self.furnishings.size));
        self
    }

    fn into_erased(self) -> ErasedRepeatCell {
        ErasedRepeatCell {
            plot: Box::new(self.plot),
            furnishings: self.furnishings,
        }
    }
}

impl<C: CoordinateSystem> From<Plot<C>> for RepeatCell<C> {
    fn from(plot: Plot<C>) -> Self {
        Self::new(plot)
    }
}

#[derive(Clone)]
pub(crate) struct ErasedRepeatCell {
    pub(crate) plot: Box<dyn SubplotChildPlotSpec>,
    pub(crate) furnishings: ChildPlotFurnishings,
}

#[derive(Clone)]
pub(crate) struct RepeatCellBranch {
    predicate: LogicalExprNode,
    cell: ErasedRepeatCell,
}

#[derive(Clone, Default)]
pub(crate) struct RepeatCellTemplates {
    default: Option<ErasedRepeatCell>,
    branches: Vec<RepeatCellBranch>,
}

impl RepeatCellTemplates {
    fn set_default(&mut self, cell: ErasedRepeatCell) {
        self.default = Some(cell);
    }

    pub(crate) fn add_branch(&mut self, predicate: impl IntoExpr, cell: ErasedRepeatCell) {
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
    ) -> Result<Option<ErasedRepeatCell>, AvengerChartError> {
        if self.default.is_none() && self.branches.is_empty() {
            return Err(AvengerChartError::InvalidArgument(format!(
                "{kind} requires a default repeated child plot or at least one guarded cell"
            )));
        }

        for (branch_index, branch) in self.branches.iter().enumerate() {
            let predicate = branch.predicate.to_default_expr(session_context)?;
            let matches =
                avenger_chart_core::evaluate_repeat_predicate(predicate, repeat_context).map_err(|err| {
                    AvengerChartError::InvalidArgument(format!(
                        "{kind} repeat cell '{cell_key}' branch {branch_index} predicate failed: {err}"
                    ))
                })?;
            if matches {
                return Ok(Some(branch.cell.clone()));
            }
        }

        Ok(self.default.clone())
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

    pub fn columns(mut self, columns: impl IntoIterator<Item = RepeatVariable>) -> Self {
        self.columns = columns.into_iter().collect();
        self
    }

    pub fn cell<C, P>(mut self, cell: P) -> Self
    where
        C: CoordinateSystem,
        P: Into<RepeatCell<C>>,
    {
        self.cells.set_default(cell.into().into_erased());
        self
    }

    pub fn cell_when<C, P>(mut self, predicate: impl IntoExpr, cell: P) -> Self
    where
        C: CoordinateSystem,
        P: Into<RepeatCell<C>>,
    {
        self.cells.add_branch(predicate, cell.into().into_erased());
        self
    }

    #[doc(hidden)]
    pub fn cell_erased(
        mut self,
        plot: Box<dyn SubplotChildPlotSpec>,
        furnishings: ChildPlotFurnishings,
    ) -> Self {
        self.cells
            .set_default(ErasedRepeatCell { plot, furnishings });
        self
    }

    #[doc(hidden)]
    pub fn cell_when_erased(
        mut self,
        predicate: impl IntoExpr,
        plot: Box<dyn SubplotChildPlotSpec>,
        furnishings: ChildPlotFurnishings,
    ) -> Self {
        self.cells
            .add_branch(predicate, ErasedRepeatCell { plot, furnishings });
        self
    }

    pub fn with_repeat_domain_coordination(mut self, mode: RepeatDomainCoordination) -> Self {
        self.domain_coordination = mode;
        self
    }

    pub(crate) fn columns_config(&self) -> &[CoreRepeatVariable] {
        &self.columns
    }

    pub(crate) fn cell_templates(&self) -> &RepeatCellTemplates {
        &self.cells
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

    pub fn rows(mut self, rows: impl IntoIterator<Item = RepeatVariable>) -> Self {
        self.rows = rows.into_iter().collect();
        self
    }

    pub fn cell<C, P>(mut self, cell: P) -> Self
    where
        C: CoordinateSystem,
        P: Into<RepeatCell<C>>,
    {
        self.cells.set_default(cell.into().into_erased());
        self
    }

    pub fn cell_when<C, P>(mut self, predicate: impl IntoExpr, cell: P) -> Self
    where
        C: CoordinateSystem,
        P: Into<RepeatCell<C>>,
    {
        self.cells.add_branch(predicate, cell.into().into_erased());
        self
    }

    #[doc(hidden)]
    pub fn cell_erased(
        mut self,
        plot: Box<dyn SubplotChildPlotSpec>,
        furnishings: ChildPlotFurnishings,
    ) -> Self {
        self.cells
            .set_default(ErasedRepeatCell { plot, furnishings });
        self
    }

    #[doc(hidden)]
    pub fn cell_when_erased(
        mut self,
        predicate: impl IntoExpr,
        plot: Box<dyn SubplotChildPlotSpec>,
        furnishings: ChildPlotFurnishings,
    ) -> Self {
        self.cells
            .add_branch(predicate, ErasedRepeatCell { plot, furnishings });
        self
    }

    pub fn with_repeat_domain_coordination(mut self, mode: RepeatDomainCoordination) -> Self {
        self.domain_coordination = mode;
        self
    }

    pub(crate) fn rows_config(&self) -> &[CoreRepeatVariable] {
        &self.rows
    }

    pub(crate) fn cell_templates(&self) -> &RepeatCellTemplates {
        &self.cells
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

    pub fn rows(mut self, rows: impl IntoIterator<Item = RepeatVariable>) -> Self {
        self.rows = rows.into_iter().collect();
        self
    }

    pub fn columns(mut self, columns: impl IntoIterator<Item = RepeatVariable>) -> Self {
        self.columns = columns.into_iter().collect();
        self
    }

    pub fn cell<C, P>(mut self, cell: P) -> Self
    where
        C: CoordinateSystem,
        P: Into<RepeatCell<C>>,
    {
        self.cells.set_default(cell.into().into_erased());
        self
    }

    pub fn cell_when<C, P>(mut self, predicate: impl IntoExpr, cell: P) -> Self
    where
        C: CoordinateSystem,
        P: Into<RepeatCell<C>>,
    {
        self.cells.add_branch(predicate, cell.into().into_erased());
        self
    }

    #[doc(hidden)]
    pub fn cell_erased(
        mut self,
        plot: Box<dyn SubplotChildPlotSpec>,
        furnishings: ChildPlotFurnishings,
    ) -> Self {
        self.cells
            .set_default(ErasedRepeatCell { plot, furnishings });
        self
    }

    #[doc(hidden)]
    pub fn cell_when_erased(
        mut self,
        predicate: impl IntoExpr,
        plot: Box<dyn SubplotChildPlotSpec>,
        furnishings: ChildPlotFurnishings,
    ) -> Self {
        self.cells
            .add_branch(predicate, ErasedRepeatCell { plot, furnishings });
        self
    }

    pub fn matrix_domains(mut self) -> Self {
        self.domain_coordination = RepeatDomainCoordination::by_variable(CoordinationScope::Shared);
        self
    }

    pub fn matrix_domains_with_scope(mut self, scope: CoordinationScope) -> Self {
        self.domain_coordination = RepeatDomainCoordination::by_variable(scope);
        self
    }

    pub fn axis_guide_visibility(mut self, policy: AxisGuideVisibilityPolicy) -> Self {
        self.axis_guide_visibility = AxisGuideVisibilityConfig::same(policy);
        self.matrix_axis_defaults = false;
        self
    }

    pub fn matrix_axes(mut self) -> Self {
        self.axis_guide_visibility = AxisGuideVisibilityConfig::same(
            AxisGuideVisibilityPolicy::OuterForEquivalentDomainGroups,
        );
        self.matrix_axis_defaults = true;
        self
    }

    pub fn with_repeat_domain_coordination(mut self, mode: RepeatDomainCoordination) -> Self {
        self.domain_coordination = mode;
        self
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

    pub(crate) fn domain_coordination_config(&self) -> &RepeatDomainCoordination {
        &self.domain_coordination
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

    pub fn items(mut self, items: impl IntoIterator<Item = RepeatVariable>) -> Self {
        self.items = items.into_iter().collect();
        self
    }

    pub fn columns(mut self, expr: impl IntoExpr) -> Self {
        self.column_mode = FacetWrapColumnMode::Fixed(
            LogicalExprNode::from_default_expr(expr.into_expr())
                .expect("Failed to serialize repeat wrap columns expression"),
        );
        self
    }

    pub fn responsive_columns(mut self, width: impl IntoExpr) -> Self {
        self.column_mode = FacetWrapColumnMode::ResponsiveWidth(
            LogicalExprNode::from_default_expr(width.into_expr())
                .expect("Failed to serialize repeat wrap responsive column width"),
        );
        self
    }

    pub fn cell<C, P>(mut self, cell: P) -> Self
    where
        C: CoordinateSystem,
        P: Into<RepeatCell<C>>,
    {
        self.cells.set_default(cell.into().into_erased());
        self
    }

    pub fn cell_when<C, P>(mut self, predicate: impl IntoExpr, cell: P) -> Self
    where
        C: CoordinateSystem,
        P: Into<RepeatCell<C>>,
    {
        self.cells.add_branch(predicate, cell.into().into_erased());
        self
    }

    #[doc(hidden)]
    pub fn cell_erased(
        mut self,
        plot: Box<dyn SubplotChildPlotSpec>,
        furnishings: ChildPlotFurnishings,
    ) -> Self {
        self.cells
            .set_default(ErasedRepeatCell { plot, furnishings });
        self
    }

    #[doc(hidden)]
    pub fn cell_when_erased(
        mut self,
        predicate: impl IntoExpr,
        plot: Box<dyn SubplotChildPlotSpec>,
        furnishings: ChildPlotFurnishings,
    ) -> Self {
        self.cells
            .add_branch(predicate, ErasedRepeatCell { plot, furnishings });
        self
    }

    pub fn item_domains(mut self) -> Self {
        self.domain_coordination = RepeatDomainCoordination::by_variable(CoordinationScope::Shared);
        self
    }

    pub fn item_domains_with_scope(mut self, scope: CoordinationScope) -> Self {
        self.domain_coordination = RepeatDomainCoordination::by_variable(scope);
        self
    }

    pub fn with_repeat_domain_coordination(mut self, mode: RepeatDomainCoordination) -> Self {
        self.domain_coordination = mode;
        self
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
        Box::new(HConcat::new().with_origin(ConcatOrigin::RepeatColumns))
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
        Box::new(VConcat::new().with_origin(ConcatOrigin::RepeatRows))
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
                .columns(self.columns.len().max(1))
                .with_origin(ConcatOrigin::RepeatGrid),
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
        Box::new(
            WrapConcat::new()
                .with_column_mode(self.column_mode.clone())
                .with_origin(ConcatOrigin::RepeatWrap),
        )
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

#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
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
        furnishings: &ChildPlotFurnishings,
    ) -> Result<Arc<dyn CompiledSubplotChildPlot>, AvengerChartError> {
        let tool_context =
            ToolCompileContext::from_parent(None).with_repeat_context(self.repeat_context.clone());
        self.compile_boxed_with_context(
            session_context,
            Some(&tool_context as CompileContext<'_>),
            furnishings,
        )
        .await
    }

    async fn compile_boxed_with_context(
        &self,
        session_context: &SessionContext,
        compile_context: Option<CompileContext<'_>>,
        furnishings: &ChildPlotFurnishings,
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
            .compile_boxed_with_context(session_context, compile_context, furnishings)
            .await
    }
}
