use std::{collections::HashMap, marker::PhantomData, sync::Arc};

use datafusion::prelude::SessionContext;
use datafusion_proto::protobuf::LogicalExprNode;

use avenger_chart_core::{
    AvengerChartError, AxisGuideVisibilityConfig, ChannelValue, ColumnDimensionConfig,
    CompileContext, CompiledMark, CompiledMarkState, CompiledSubplotChildPlot,
    CoordinateSystemCore, CoordinationScope, DataContext, FacetDataScope, FacetDimensionConfig,
    FacetEmptyCellPolicy, FacetWrapColumnMode, IntoPlotMark, Mark, MarkDataMode, MarkState,
    PlotMark, RowDimensionConfig, SubplotChildPlotSpec, SubplotContainerCoordinateSystem,
    SubplotMarkCore,
};

#[derive(Clone)]
pub(crate) struct SubplotConfig {
    pub(crate) label: Option<String>,
    pub(crate) key: Option<String>,
    pub(crate) plot_width: Option<f32>,
    pub(crate) plot_height: Option<f32>,
    pub(crate) facet_row_title: Option<String>,
    pub(crate) facet_col_title: Option<String>,
    pub(crate) facet_row_slot_sharing: Option<CoordinationScope>,
    pub(crate) facet_col_slot_sharing: Option<CoordinationScope>,
    pub(crate) facet_row_position: Option<String>,
    pub(crate) facet_col_position: Option<String>,
    pub(crate) facet_row_guide_visible: Option<bool>,
    pub(crate) facet_col_guide_visible: Option<bool>,
    pub(crate) facet_row_axis_guide_visibility: Option<AxisGuideVisibilityConfig>,
    pub(crate) facet_col_axis_guide_visibility: Option<AxisGuideVisibilityConfig>,
    pub(crate) facet_row_empty_cell_policy: Option<FacetEmptyCellPolicy>,
    pub(crate) facet_col_empty_cell_policy: Option<FacetEmptyCellPolicy>,
    pub(crate) facet_row_order_expr: Option<LogicalExprNode>,
    pub(crate) facet_col_order_expr: Option<LogicalExprNode>,
    pub(crate) facet_row_order_descending: bool,
    pub(crate) facet_col_order_descending: bool,
    pub(crate) facet_wrap_title: Option<String>,
    pub(crate) facet_wrap_slot_sharing: Option<CoordinationScope>,
    pub(crate) facet_wrap_position: Option<String>,
    pub(crate) facet_wrap_guide_visible: Option<bool>,
    pub(crate) facet_wrap_axis_guide_visibility: Option<AxisGuideVisibilityConfig>,
    pub(crate) facet_wrap_empty_cell_policy: Option<FacetEmptyCellPolicy>,
    pub(crate) facet_wrap_order_expr: Option<LogicalExprNode>,
    pub(crate) facet_wrap_order_descending: bool,
    pub(crate) facet_wrap_column_mode: FacetWrapColumnMode,
    pub(crate) grid_row: Option<usize>,
    pub(crate) grid_column: Option<usize>,
    pub(crate) grid_row_span: usize,
    pub(crate) grid_column_span: usize,
}

impl Default for SubplotConfig {
    fn default() -> Self {
        Self {
            label: None,
            key: None,
            plot_width: None,
            plot_height: None,
            facet_row_title: None,
            facet_col_title: None,
            facet_row_slot_sharing: None,
            facet_col_slot_sharing: None,
            facet_row_position: None,
            facet_col_position: None,
            facet_row_guide_visible: None,
            facet_col_guide_visible: None,
            facet_row_axis_guide_visibility: None,
            facet_col_axis_guide_visibility: None,
            facet_row_empty_cell_policy: None,
            facet_col_empty_cell_policy: None,
            facet_row_order_expr: None,
            facet_col_order_expr: None,
            facet_row_order_descending: false,
            facet_col_order_descending: false,
            facet_wrap_title: None,
            facet_wrap_slot_sharing: None,
            facet_wrap_position: None,
            facet_wrap_guide_visible: None,
            facet_wrap_axis_guide_visibility: None,
            facet_wrap_empty_cell_policy: None,
            facet_wrap_order_expr: None,
            facet_wrap_order_descending: false,
            facet_wrap_column_mode: FacetWrapColumnMode::Auto,
            grid_row: None,
            grid_column: None,
            grid_row_span: 1,
            grid_column_span: 1,
        }
    }
}

/// Mark that owns one child plot inside an outer coordinate system.
///
/// `Subplot<OuterC>` is parameterized by the coordinate system that positions
/// the child plot, not by the child plot's own coordinate system. This keeps
/// concat, facet, and coordinate-positioned subplots under one public mark
/// concept while still allowing mixed child coordinate systems.
#[derive(Clone)]
pub struct Subplot<OuterC: CoordinateSystemCore> {
    state: MarkState,
    subplot: Box<dyn SubplotChildPlotSpec>,
    config: SubplotConfig,
    _outer: PhantomData<fn() -> OuterC>,
}

impl<OuterC: CoordinateSystemCore> Subplot<OuterC> {
    pub fn new<P>(subplot: P) -> Self
    where
        P: SubplotChildPlotSpec + 'static,
    {
        Self {
            state: MarkState {
                id: None,
                data: DataContext::default(),
                view: None,
                data_mode: MarkDataMode::Inherit,
                facet_data_scope: FacetDataScope::FILTERED,
                exclude_from_scale_domains: false,
                visible: None,
                details: None,
                zindex: None,
                geometry_space: None,
                axis_configs: HashMap::new(),
            },
            subplot: Box::new(subplot),
            config: SubplotConfig::default(),
            _outer: PhantomData,
        }
    }

    /// Set a structural id used by chart interaction targeting.
    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.state.id = Some(id.into());
        self
    }

    /// Set explicit data for this subplot mark.
    pub fn data(mut self, dataframe: datafusion::dataframe::DataFrame) -> Self {
        self.state.data = DataContext::new(dataframe);
        self
    }

    /// Set zindex.
    pub fn zindex(mut self, zindex: i32) -> Self {
        self.state.zindex = Some(zindex);
        self
    }

    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.config.label = Some(label.into());
        self
    }

    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.config.key = Some(key.into());
        self
    }

    /// Place this subplot in an explicit `GridConcat` row and column.
    pub fn grid_cell(mut self, row: usize, column: usize) -> Self {
        self.config.grid_row = Some(row);
        self.config.grid_column = Some(column);
        self
    }

    /// Set the row span used by `GridConcat`.
    pub fn grid_row_span(mut self, span: usize) -> Self {
        self.config.grid_row_span = span;
        self
    }

    /// Set the column span used by `GridConcat`.
    pub fn grid_column_span(mut self, span: usize) -> Self {
        self.config.grid_column_span = span;
        self
    }

    /// Set the row and column span used by `GridConcat`.
    pub fn grid_span(mut self, row_span: usize, column_span: usize) -> Self {
        self.config.grid_row_span = row_span;
        self.config.grid_column_span = column_span;
        self
    }

    #[doc(hidden)]
    pub fn data_context_ref(&self) -> &DataContext {
        &self.state.data
    }

    #[doc(hidden)]
    pub fn label_config(&self) -> Option<&str> {
        self.config.label.as_deref()
    }

    #[doc(hidden)]
    pub fn key_config(&self) -> Option<&str> {
        self.config.key.as_deref()
    }

    #[doc(hidden)]
    pub fn grid_row_config(&self) -> Option<usize> {
        self.config.grid_row
    }

    #[doc(hidden)]
    pub fn grid_column_config(&self) -> Option<usize> {
        self.config.grid_column
    }

    #[doc(hidden)]
    pub fn grid_row_span_config(&self) -> usize {
        self.config.grid_row_span
    }

    #[doc(hidden)]
    pub fn grid_column_span_config(&self) -> usize {
        self.config.grid_column_span
    }

    #[doc(hidden)]
    pub fn plot_width_config(&self) -> Option<f32> {
        self.config.plot_width
    }

    #[doc(hidden)]
    pub fn plot_height_config(&self) -> Option<f32> {
        self.config.plot_height
    }

    #[doc(hidden)]
    pub fn set_plot_width_config(&mut self, width: Option<f32>) {
        self.config.plot_width = width;
    }

    #[doc(hidden)]
    pub fn set_plot_height_config(&mut self, height: Option<f32>) {
        self.config.plot_height = height;
    }

    #[doc(hidden)]
    #[allow(clippy::too_many_arguments)]
    pub fn set_facet_row_options(
        &mut self,
        title: Option<String>,
        slot_sharing: Option<CoordinationScope>,
        position: Option<String>,
        guide_visible: Option<bool>,
        axis_guide_visibility: Option<AxisGuideVisibilityConfig>,
        empty_cell_policy: Option<FacetEmptyCellPolicy>,
        order_expr: Option<LogicalExprNode>,
        order_descending: bool,
    ) {
        self.config.facet_row_title = title;
        self.config.facet_row_slot_sharing = slot_sharing;
        self.config.facet_row_position = position;
        self.config.facet_row_guide_visible = guide_visible;
        self.config.facet_row_axis_guide_visibility = axis_guide_visibility;
        self.config.facet_row_empty_cell_policy = empty_cell_policy;
        self.config.facet_row_order_expr = order_expr;
        self.config.facet_row_order_descending = order_descending;
    }

    #[doc(hidden)]
    pub fn facet_row_title_config(&self) -> Option<&str> {
        self.config.facet_row_title.as_deref()
    }

    #[doc(hidden)]
    pub fn facet_row_slot_sharing_config(&self) -> Option<CoordinationScope> {
        self.config.facet_row_slot_sharing
    }

    #[doc(hidden)]
    pub fn facet_row_position_config(&self) -> Option<&str> {
        self.config.facet_row_position.as_deref()
    }

    #[doc(hidden)]
    pub fn facet_row_guide_visible_config(&self) -> Option<bool> {
        self.config.facet_row_guide_visible
    }

    #[doc(hidden)]
    pub fn facet_row_axis_guide_visibility_config(&self) -> Option<AxisGuideVisibilityConfig> {
        self.config.facet_row_axis_guide_visibility
    }

    #[doc(hidden)]
    pub fn facet_row_empty_cell_policy_config(&self) -> Option<FacetEmptyCellPolicy> {
        self.config.facet_row_empty_cell_policy
    }

    #[doc(hidden)]
    pub fn facet_row_order_expr_config(&self) -> Option<&LogicalExprNode> {
        self.config.facet_row_order_expr.as_ref()
    }

    #[doc(hidden)]
    pub fn facet_row_order_descending_config(&self) -> bool {
        self.config.facet_row_order_descending
    }

    #[doc(hidden)]
    #[allow(clippy::too_many_arguments)]
    pub fn set_facet_col_options(
        &mut self,
        title: Option<String>,
        slot_sharing: Option<CoordinationScope>,
        position: Option<String>,
        guide_visible: Option<bool>,
        axis_guide_visibility: Option<AxisGuideVisibilityConfig>,
        empty_cell_policy: Option<FacetEmptyCellPolicy>,
        order_expr: Option<LogicalExprNode>,
        order_descending: bool,
    ) {
        self.config.facet_col_title = title;
        self.config.facet_col_slot_sharing = slot_sharing;
        self.config.facet_col_position = position;
        self.config.facet_col_guide_visible = guide_visible;
        self.config.facet_col_axis_guide_visibility = axis_guide_visibility;
        self.config.facet_col_empty_cell_policy = empty_cell_policy;
        self.config.facet_col_order_expr = order_expr;
        self.config.facet_col_order_descending = order_descending;
    }

    #[doc(hidden)]
    pub fn facet_col_title_config(&self) -> Option<&str> {
        self.config.facet_col_title.as_deref()
    }

    #[doc(hidden)]
    pub fn facet_col_slot_sharing_config(&self) -> Option<CoordinationScope> {
        self.config.facet_col_slot_sharing
    }

    #[doc(hidden)]
    pub fn facet_col_position_config(&self) -> Option<&str> {
        self.config.facet_col_position.as_deref()
    }

    #[doc(hidden)]
    pub fn facet_col_guide_visible_config(&self) -> Option<bool> {
        self.config.facet_col_guide_visible
    }

    #[doc(hidden)]
    pub fn facet_col_axis_guide_visibility_config(&self) -> Option<AxisGuideVisibilityConfig> {
        self.config.facet_col_axis_guide_visibility
    }

    #[doc(hidden)]
    pub fn facet_col_empty_cell_policy_config(&self) -> Option<FacetEmptyCellPolicy> {
        self.config.facet_col_empty_cell_policy
    }

    #[doc(hidden)]
    pub fn facet_col_order_expr_config(&self) -> Option<&LogicalExprNode> {
        self.config.facet_col_order_expr.as_ref()
    }

    #[doc(hidden)]
    pub fn facet_col_order_descending_config(&self) -> bool {
        self.config.facet_col_order_descending
    }

    #[doc(hidden)]
    #[allow(clippy::too_many_arguments)]
    pub fn set_facet_wrap_options(
        &mut self,
        title: Option<String>,
        slot_sharing: Option<CoordinationScope>,
        position: Option<String>,
        guide_visible: Option<bool>,
        axis_guide_visibility: Option<AxisGuideVisibilityConfig>,
        empty_cell_policy: Option<FacetEmptyCellPolicy>,
        order_expr: Option<LogicalExprNode>,
        order_descending: bool,
        column_mode: FacetWrapColumnMode,
    ) {
        self.config.facet_wrap_title = title;
        self.config.facet_wrap_slot_sharing = slot_sharing;
        self.config.facet_wrap_position = position;
        self.config.facet_wrap_guide_visible = guide_visible;
        self.config.facet_wrap_axis_guide_visibility = axis_guide_visibility;
        self.config.facet_wrap_empty_cell_policy = empty_cell_policy;
        self.config.facet_wrap_order_expr = order_expr;
        self.config.facet_wrap_order_descending = order_descending;
        self.config.facet_wrap_column_mode = column_mode;
    }

    #[doc(hidden)]
    pub fn facet_wrap_title_config(&self) -> Option<&str> {
        self.config.facet_wrap_title.as_deref()
    }

    #[doc(hidden)]
    pub fn facet_wrap_slot_sharing_config(&self) -> Option<CoordinationScope> {
        self.config.facet_wrap_slot_sharing
    }

    #[doc(hidden)]
    pub fn facet_wrap_position_config(&self) -> Option<&str> {
        self.config.facet_wrap_position.as_deref()
    }

    #[doc(hidden)]
    pub fn facet_wrap_guide_visible_config(&self) -> Option<bool> {
        self.config.facet_wrap_guide_visible
    }

    #[doc(hidden)]
    pub fn facet_wrap_axis_guide_visibility_config(&self) -> Option<AxisGuideVisibilityConfig> {
        self.config.facet_wrap_axis_guide_visibility
    }

    #[doc(hidden)]
    pub fn facet_wrap_empty_cell_policy_config(&self) -> Option<FacetEmptyCellPolicy> {
        self.config.facet_wrap_empty_cell_policy
    }

    #[doc(hidden)]
    pub fn facet_wrap_order_expr_config(&self) -> Option<&LogicalExprNode> {
        self.config.facet_wrap_order_expr.as_ref()
    }

    #[doc(hidden)]
    pub fn facet_wrap_order_descending_config(&self) -> bool {
        self.config.facet_wrap_order_descending
    }

    #[doc(hidden)]
    pub fn facet_wrap_column_mode_config(&self) -> FacetWrapColumnMode {
        self.config.facet_wrap_column_mode.clone()
    }

    pub fn has_plot_level_data(&self) -> bool {
        self.subplot.has_plot_level_data()
    }

    pub async fn compile_child_plot(
        &self,
        session_context: &SessionContext,
    ) -> Result<Arc<dyn CompiledSubplotChildPlot>, AvengerChartError> {
        self.subplot.compile_boxed(session_context).await
    }

    pub async fn compile_child_plot_with_context(
        &self,
        session_context: &SessionContext,
        compile_context: Option<CompileContext<'_>>,
    ) -> Result<Arc<dyn CompiledSubplotChildPlot>, AvengerChartError> {
        self.subplot
            .compile_boxed_with_context(session_context, compile_context)
            .await
    }

    #[doc(hidden)]
    pub fn with_channel_value(mut self, channel_name: &'static str, value: ChannelValue) -> Self {
        self.state.data = self.state.data.with_channel_value(channel_name, value);
        self
    }

    #[doc(hidden)]
    pub fn validate_no_facet_channels(&self, outer_label: &str) -> Result<(), AvengerChartError> {
        let channels = self.state.data.channels();
        for channel_name in [
            RowDimensionConfig::channel_name(),
            ColumnDimensionConfig::channel_name(),
        ] {
            if channels.contains_key(channel_name) {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "{outer_label} subplots do not support facet channel `{channel_name}`"
                )));
            }
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl<OuterC: CoordinateSystemCore> SubplotMarkCore for Subplot<OuterC> {
    fn data_context_ref(&self) -> &DataContext {
        &self.state.data
    }

    fn label_config(&self) -> Option<&str> {
        self.config.label.as_deref()
    }

    fn key_config(&self) -> Option<&str> {
        self.config.key.as_deref()
    }

    fn grid_row_config(&self) -> Option<usize> {
        self.config.grid_row
    }

    fn grid_column_config(&self) -> Option<usize> {
        self.config.grid_column
    }

    fn grid_row_span_config(&self) -> usize {
        self.config.grid_row_span
    }

    fn grid_column_span_config(&self) -> usize {
        self.config.grid_column_span
    }

    fn plot_width_config(&self) -> Option<f32> {
        self.config.plot_width
    }

    fn plot_height_config(&self) -> Option<f32> {
        self.config.plot_height
    }

    fn facet_row_title_config(&self) -> Option<&str> {
        self.config.facet_row_title.as_deref()
    }

    fn facet_row_slot_sharing_config(&self) -> Option<CoordinationScope> {
        self.config.facet_row_slot_sharing
    }

    fn facet_row_position_config(&self) -> Option<&str> {
        self.config.facet_row_position.as_deref()
    }

    fn facet_row_guide_visible_config(&self) -> Option<bool> {
        self.config.facet_row_guide_visible
    }

    fn facet_row_axis_guide_visibility_config(&self) -> Option<AxisGuideVisibilityConfig> {
        self.config.facet_row_axis_guide_visibility
    }

    fn facet_row_empty_cell_policy_config(&self) -> Option<FacetEmptyCellPolicy> {
        self.config.facet_row_empty_cell_policy
    }

    fn facet_row_order_expr_config(&self) -> Option<&LogicalExprNode> {
        self.config.facet_row_order_expr.as_ref()
    }

    fn facet_row_order_descending_config(&self) -> bool {
        self.config.facet_row_order_descending
    }

    fn facet_col_title_config(&self) -> Option<&str> {
        self.config.facet_col_title.as_deref()
    }

    fn facet_col_slot_sharing_config(&self) -> Option<CoordinationScope> {
        self.config.facet_col_slot_sharing
    }

    fn facet_col_position_config(&self) -> Option<&str> {
        self.config.facet_col_position.as_deref()
    }

    fn facet_col_guide_visible_config(&self) -> Option<bool> {
        self.config.facet_col_guide_visible
    }

    fn facet_col_axis_guide_visibility_config(&self) -> Option<AxisGuideVisibilityConfig> {
        self.config.facet_col_axis_guide_visibility
    }

    fn facet_col_empty_cell_policy_config(&self) -> Option<FacetEmptyCellPolicy> {
        self.config.facet_col_empty_cell_policy
    }

    fn facet_col_order_expr_config(&self) -> Option<&LogicalExprNode> {
        self.config.facet_col_order_expr.as_ref()
    }

    fn facet_col_order_descending_config(&self) -> bool {
        self.config.facet_col_order_descending
    }

    fn facet_wrap_title_config(&self) -> Option<&str> {
        self.config.facet_wrap_title.as_deref()
    }

    fn facet_wrap_slot_sharing_config(&self) -> Option<CoordinationScope> {
        self.config.facet_wrap_slot_sharing
    }

    fn facet_wrap_position_config(&self) -> Option<&str> {
        self.config.facet_wrap_position.as_deref()
    }

    fn facet_wrap_guide_visible_config(&self) -> Option<bool> {
        self.config.facet_wrap_guide_visible
    }

    fn facet_wrap_axis_guide_visibility_config(&self) -> Option<AxisGuideVisibilityConfig> {
        self.config.facet_wrap_axis_guide_visibility
    }

    fn facet_wrap_empty_cell_policy_config(&self) -> Option<FacetEmptyCellPolicy> {
        self.config.facet_wrap_empty_cell_policy
    }

    fn facet_wrap_order_expr_config(&self) -> Option<&LogicalExprNode> {
        self.config.facet_wrap_order_expr.as_ref()
    }

    fn facet_wrap_order_descending_config(&self) -> bool {
        self.config.facet_wrap_order_descending
    }

    fn facet_wrap_column_mode_config(&self) -> FacetWrapColumnMode {
        self.config.facet_wrap_column_mode.clone()
    }

    fn has_plot_level_data(&self) -> bool {
        self.subplot.has_plot_level_data()
    }

    async fn compile_child_plot(
        &self,
        session_context: &SessionContext,
    ) -> Result<Arc<dyn CompiledSubplotChildPlot>, AvengerChartError> {
        self.subplot.compile_boxed(session_context).await
    }

    async fn compile_child_plot_with_context(
        &self,
        session_context: &SessionContext,
        compile_context: Option<CompileContext<'_>>,
    ) -> Result<Arc<dyn CompiledSubplotChildPlot>, AvengerChartError> {
        self.subplot
            .compile_boxed_with_context(session_context, compile_context)
            .await
    }
}

#[async_trait::async_trait]
impl<C> Mark<C> for Subplot<C>
where
    C: SubplotContainerCoordinateSystem,
{
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
        C::compile_subplot_mark(self, compiled_state, session_context).await
    }

    async fn compile_with_context(
        &self,
        compiled_state: CompiledMarkState,
        session_context: &SessionContext,
        compile_context: Option<CompileContext<'_>>,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        C::compile_subplot_mark_with_context(self, compiled_state, session_context, compile_context)
            .await
    }
}

impl<C> IntoPlotMark<C> for Subplot<C>
where
    C: SubplotContainerCoordinateSystem,
{
    fn into_plot_marks(self) -> Vec<PlotMark<C>> {
        vec![PlotMark::from_mark(self)]
    }
}
