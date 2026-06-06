//! Repeat container authoring support.
//!
//! Repeat containers are semantic authoring sugar. During compilation they
//! lower to ordinary concat-family containers with generated `Subplot` marks.

use std::sync::Arc;

use avenger_chart_core::{
    AvengerChartError, CompileContext, CompiledSubplotChildPlot, CoordinateSystem,
    CoordinateSystemCore, CoordinateSystemTransform, DefaultLogicalExprNodeExt,
    FacetWrapColumnMode, IntoExpr, RepeatContext as CoreRepeatContext,
    RepeatVariable as CoreRepeatVariable, SubplotChildPlotSpec,
};
use datafusion::prelude::SessionContext;
use datafusion_proto::protobuf::LogicalExprNode;

use crate::{
    concat::{ConcatGuide, GridConcat, HConcat, VConcat, WrapConcat},
    tools::ToolCompileContext,
};

pub use avenger_chart_core::repeat::*;

#[derive(Clone, Default)]
pub struct RepeatColumns {
    columns: Vec<CoreRepeatVariable>,
    cell: Option<Box<dyn SubplotChildPlotSpec>>,
}

impl RepeatColumns {
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn set_columns(&mut self, columns: Vec<CoreRepeatVariable>) {
        self.columns = columns;
    }

    pub(crate) fn set_cell(&mut self, cell: Box<dyn SubplotChildPlotSpec>) {
        self.cell = Some(cell);
    }

    pub(crate) fn columns_config(&self) -> &[CoreRepeatVariable] {
        &self.columns
    }

    pub(crate) fn cell_config(&self) -> Option<&dyn SubplotChildPlotSpec> {
        self.cell.as_deref()
    }
}

#[derive(Clone, Default)]
pub struct RepeatRows {
    rows: Vec<CoreRepeatVariable>,
    cell: Option<Box<dyn SubplotChildPlotSpec>>,
}

impl RepeatRows {
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn set_rows(&mut self, rows: Vec<CoreRepeatVariable>) {
        self.rows = rows;
    }

    pub(crate) fn set_cell(&mut self, cell: Box<dyn SubplotChildPlotSpec>) {
        self.cell = Some(cell);
    }

    pub(crate) fn rows_config(&self) -> &[CoreRepeatVariable] {
        &self.rows
    }

    pub(crate) fn cell_config(&self) -> Option<&dyn SubplotChildPlotSpec> {
        self.cell.as_deref()
    }
}

#[derive(Clone, Default)]
pub struct RepeatGrid {
    rows: Vec<CoreRepeatVariable>,
    columns: Vec<CoreRepeatVariable>,
    cell: Option<Box<dyn SubplotChildPlotSpec>>,
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
        self.cell = Some(cell);
    }

    pub(crate) fn rows_config(&self) -> &[CoreRepeatVariable] {
        &self.rows
    }

    pub(crate) fn columns_config(&self) -> &[CoreRepeatVariable] {
        &self.columns
    }

    pub(crate) fn cell_config(&self) -> Option<&dyn SubplotChildPlotSpec> {
        self.cell.as_deref()
    }
}

#[derive(Clone)]
pub struct RepeatWrap {
    items: Vec<CoreRepeatVariable>,
    column_mode: FacetWrapColumnMode,
    cell: Option<Box<dyn SubplotChildPlotSpec>>,
}

impl Default for RepeatWrap {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            column_mode: FacetWrapColumnMode::Auto,
            cell: None,
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
        self.cell = Some(cell);
    }

    pub(crate) fn items_config(&self) -> &[CoreRepeatVariable] {
        &self.items
    }

    pub(crate) fn column_mode_config(&self) -> &FacetWrapColumnMode {
        &self.column_mode
    }

    pub(crate) fn cell_config(&self) -> Option<&dyn SubplotChildPlotSpec> {
        self.cell.as_deref()
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
