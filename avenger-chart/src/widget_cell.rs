//! Positionless widget hosts for concat coordinate systems.

use std::{any::Any, collections::HashMap, marker::PhantomData, sync::Arc};

use avenger_chart_core::{
    AvengerChartError, CanonicalJson, ChartWidget, CompileContext, CompiledDataContext,
    CompiledMark, CompiledMarkCore, CompiledMarkState, CompiledNativeWidgetSpec, CompiledWidget,
    CoordinateSystemCore, CoordinateSystemTransformCore, DataContext, FacetDataScope, IntoPlotMark,
    Mark, MarkDataMode, MarkRuntimeContext, MarkState, NativeWidget, PlotMark, WidgetSource,
    validate_structural_id,
};
use avenger_chart_scales::PlotScaleSpec as ScaleSpec;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::{arrow::record_batch::RecordBatch, prelude::SessionContext};
use serde::{Deserialize, Serialize};

use crate::{
    concat::{GridConcat, HConcat, VConcat, WrapConcat},
    plot::compile_composed_widget,
    tools::ToolCompileContext,
};

/// Coordinate systems that accept a positionless [`WidgetCell`] child.
pub trait WidgetCellCoordinate: CoordinateSystemCore {}

impl WidgetCellCoordinate for HConcat {}
impl WidgetCellCoordinate for VConcat {}
impl WidgetCellCoordinate for GridConcat {}
impl WidgetCellCoordinate for WrapConcat {}

/// A widget occupying one concat cell without an implicit child [`Plot`](crate::plot::Plot).
#[derive(Clone)]
pub struct WidgetCell<C: WidgetCellCoordinate> {
    state: MarkState,
    source: WidgetSource,
    name: Option<String>,
    grid_row: Option<usize>,
    grid_column: Option<usize>,
    grid_row_span: usize,
    grid_column_span: usize,
    _coord: PhantomData<fn() -> C>,
}

impl<C: WidgetCellCoordinate> WidgetCell<C> {
    fn from_source(source: WidgetSource) -> Self {
        Self {
            state: MarkState {
                id: None,
                public_aliases: Vec::new(),
                data: DataContext::default(),
                view: None,
                data_mode: MarkDataMode::Unit,
                facet_data_scope: FacetDataScope::FILTERED,
                visible: None,
                details: None,
                zindex: None,
                geometry_space: None,
                axis_configs: HashMap::new(),
            },
            source,
            name: None,
            grid_row: None,
            grid_column: None,
            grid_row_span: 1,
            grid_column_span: 1,
            _coord: PhantomData,
        }
    }

    /// Host a scene-graph composed widget in this cell.
    pub fn widget(widget: impl ChartWidget) -> Self {
        Self::from_source(WidgetSource::composed(widget))
    }

    /// Host a native widget descriptor in this cell.
    pub fn native_widget(widget: impl NativeWidget) -> Self {
        Self::from_source(WidgetSource::native(widget))
    }

    /// Set the stable structural prefix used by interaction targets.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Place this cell in an explicit `GridConcat` row and column.
    pub fn at(mut self, row: usize, column: usize) -> Self {
        self.grid_row = Some(row);
        self.grid_column = Some(column);
        self
    }

    /// Set the row span used by `GridConcat`.
    pub fn grid_row_span(mut self, span: usize) -> Self {
        self.grid_row_span = span;
        self
    }

    /// Set the column span used by `GridConcat`.
    pub fn grid_column_span(mut self, span: usize) -> Self {
        self.grid_column_span = span;
        self
    }

    /// Set both row and column spans used by `GridConcat`.
    pub fn span(mut self, row_span: usize, column_span: usize) -> Self {
        self.grid_row_span = row_span;
        self.grid_column_span = column_span;
        self
    }

    async fn compile_cell(
        &self,
        compiled_state: CompiledMarkState,
        session_context: &SessionContext,
        compile_context: Option<CompileContext<'_>>,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        let local_context;
        let tool_context =
            if let Some(context) = compile_context.and_then(ToolCompileContext::downcast) {
                context
            } else {
                local_context = ToolCompileContext::from_parent(None);
                &local_context
            };
        if tool_context.is_multiplied_host() {
            return Err(AvengerChartError::InvalidArgument(
                "WidgetCell cannot be instantiated by a facet or repeat template; use a one-shot concat cell"
                    .to_string(),
            ));
        }
        let name = self
            .name
            .clone()
            .unwrap_or_else(|| format!("widget_cell_{}", compiled_state.mark_index()));
        validate_structural_id("widget cell", &name)?;
        let grid_placement = WidgetCellGridPlacement {
            row: self.grid_row,
            column: self.grid_column,
            row_span: self.grid_row_span,
            column_span: self.grid_column_span,
        };

        let (widget, scale_specs) = if let Some(widget) = self.source.composed_widget() {
            let public_widget_path = format!("{name}.{}", widget.id());
            let target_prefix = tool_context.target_path_with_child(compiled_state.mark_index());
            let mut identity_allocator = avenger_chart_core::CompiledIdentityAllocator::new(
                compiled_state.identity.runtime_id.as_opaque_str(),
            );
            let widget_instance_id = identity_allocator.allocate_widget_instance();
            let output = compile_composed_widget(
                widget,
                &public_widget_path,
                session_context,
                tool_context,
                0,
                Some(&target_prefix),
                widget_instance_id,
                &mut identity_allocator,
            )
            .await?;
            (CompiledWidget::Composed(output.widget), output.scale_specs)
        } else if let Some(widget) = self.source.native_widget() {
            let id = widget.id().to_string();
            validate_structural_id("widget", &id)?;
            validate_structural_id("widget kind", widget.kind())?;
            let mut identity_allocator = avenger_chart_core::CompiledIdentityAllocator::new(
                compiled_state.identity.runtime_id.as_opaque_str(),
            );
            let widget_instance_id = identity_allocator.allocate_widget_instance();
            let state = widget.state().with_instance_identity(&widget_instance_id);
            let identity = widget as *const dyn NativeWidget as *const () as usize;
            tool_context.register_native_widget(&id, identity, &state)?;
            (
                CompiledWidget::Native(CompiledNativeWidgetSpec {
                    id,
                    kind: widget.kind().to_string(),
                    schema_version: widget.schema_version(),
                    payload: CanonicalJson::from_value(widget.payload())?,
                    measure: widget.measure(),
                    state,
                }),
                HashMap::new(),
            )
        } else {
            return Err(AvengerChartError::InternalError(
                "WidgetCell source has no authoring variant".to_string(),
            ));
        };

        Ok(Arc::new(CompiledWidgetCell {
            state: compiled_state,
            name,
            widget,
            scale_specs,
            grid_placement,
        }))
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
impl<C: WidgetCellCoordinate> Mark<C> for WidgetCell<C> {
    fn state(&self) -> &MarkState {
        &self.state
    }

    fn state_mut(&mut self) -> &mut MarkState {
        &mut self.state
    }

    fn data_context(&self) -> &DataContext {
        &self.state.data
    }

    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        session_context: &SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        self.compile_cell(compiled_state, session_context, None)
            .await
    }

    async fn compile_with_context(
        &self,
        compiled_state: CompiledMarkState,
        session_context: &SessionContext,
        compile_context: Option<CompileContext<'_>>,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        self.compile_cell(compiled_state, session_context, compile_context)
            .await
    }
}

impl<C: WidgetCellCoordinate> IntoPlotMark<C> for WidgetCell<C> {
    fn into_plot_marks(self) -> Vec<PlotMark<C>> {
        vec![PlotMark::from_mark(self)]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct WidgetCellGridPlacement {
    pub(crate) row: Option<usize>,
    pub(crate) column: Option<usize>,
    pub(crate) row_span: usize,
    pub(crate) column_span: usize,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledWidgetCell {
    state: CompiledMarkState,
    name: String,
    widget: CompiledWidget,
    scale_specs: HashMap<String, ScaleSpec>,
    grid_placement: WidgetCellGridPlacement,
}

impl CompiledWidgetCell {
    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn widget(&self) -> &CompiledWidget {
        &self.widget
    }

    pub(crate) fn scale_specs(&self) -> &HashMap<String, ScaleSpec> {
        &self.scale_specs
    }

    pub(crate) fn child_index(&self) -> usize {
        self.state.mark_index()
    }

    pub(crate) fn grid_placement(&self) -> WidgetCellGridPlacement {
        self.grid_placement
    }
}

pub(crate) fn compiled_widget_cell(mark: &dyn CompiledMark) -> Option<&CompiledWidgetCell> {
    (mark.mark_type() == "widget_cell")
        .then(|| mark.as_any().downcast_ref::<CompiledWidgetCell>())
        .flatten()
}

impl CompiledMarkCore for CompiledWidgetCell {
    avenger_chart_core::impl_mark_with_data_context!();

    fn state(&self) -> &CompiledMarkState {
        &self.state
    }

    fn state_mut(&mut self) -> &mut CompiledMarkState {
        &mut self.state
    }

    fn data_context(&self) -> &CompiledDataContext {
        &self.state.data
    }

    fn mark_type(&self) -> &str {
        "widget_cell"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn supported_channels(&self) -> Vec<avenger_chart_core::ChannelDescriptor> {
        Vec::new()
    }
}

#[typetag::serde]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
impl CompiledMark for CompiledWidgetCell {
    async fn render_from_data(
        &self,
        _data: Option<&RecordBatch>,
        _scalars: &RecordBatch,
        _context: &dyn MarkRuntimeContext,
        _coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        Err(AvengerChartError::InternalError(
            "WidgetCell marks require the concat layout render dispatcher".to_string(),
        ))
    }
}
