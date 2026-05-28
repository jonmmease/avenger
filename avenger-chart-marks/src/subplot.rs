use std::{collections::HashMap, marker::PhantomData, sync::Arc};

use datafusion::prelude::SessionContext;
use datafusion_proto::protobuf::LogicalExprNode;

use avenger_chart_core::{
    AvengerChartError, ChannelValue, ColumnDimensionConfig, CompiledMark, CompiledMarkState,
    CompiledSubplotChildPlot, CoordinateSystemCore, DataContext, FacetDataScope,
    FacetDimensionConfig, FacetEmptyCellPolicy, FacetWrapColumnMode, Mark, MarkState,
    RowDimensionConfig, Sharing, SubplotChildPlotSpec, SubplotContainerCoordinateSystem,
    SubplotMarkCore,
};

#[derive(Clone, Default)]
pub(crate) struct SubplotConfig {
    pub(crate) label: Option<String>,
    pub(crate) key: Option<String>,
    pub(crate) plot_width: Option<f32>,
    pub(crate) plot_height: Option<f32>,
    pub(crate) facet_row_title: Option<String>,
    pub(crate) facet_col_title: Option<String>,
    pub(crate) facet_row_slot_sharing: Option<Sharing>,
    pub(crate) facet_col_slot_sharing: Option<Sharing>,
    pub(crate) facet_row_position: Option<String>,
    pub(crate) facet_col_position: Option<String>,
    pub(crate) facet_row_guide_visible: Option<bool>,
    pub(crate) facet_col_guide_visible: Option<bool>,
    pub(crate) facet_row_empty_cell_policy: Option<FacetEmptyCellPolicy>,
    pub(crate) facet_col_empty_cell_policy: Option<FacetEmptyCellPolicy>,
    pub(crate) facet_row_order_expr: Option<LogicalExprNode>,
    pub(crate) facet_col_order_expr: Option<LogicalExprNode>,
    pub(crate) facet_row_order_descending: bool,
    pub(crate) facet_col_order_descending: bool,
    pub(crate) facet_wrap_title: Option<String>,
    pub(crate) facet_wrap_slot_sharing: Option<Sharing>,
    pub(crate) facet_wrap_position: Option<String>,
    pub(crate) facet_wrap_guide_visible: Option<bool>,
    pub(crate) facet_wrap_empty_cell_policy: Option<FacetEmptyCellPolicy>,
    pub(crate) facet_wrap_order_expr: Option<LogicalExprNode>,
    pub(crate) facet_wrap_order_descending: bool,
    pub(crate) facet_wrap_column_mode: FacetWrapColumnMode,
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
                data: DataContext::default(),
                facet_data_scope: FacetDataScope::FILTERED,
                details: None,
                zindex: None,
                axis_configs: HashMap::new(),
            },
            subplot: Box::new(subplot),
            config: SubplotConfig::default(),
            _outer: PhantomData,
        }
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
        slot_sharing: Option<Sharing>,
        position: Option<String>,
        guide_visible: Option<bool>,
        empty_cell_policy: Option<FacetEmptyCellPolicy>,
        order_expr: Option<LogicalExprNode>,
        order_descending: bool,
    ) {
        self.config.facet_row_title = title;
        self.config.facet_row_slot_sharing = slot_sharing;
        self.config.facet_row_position = position;
        self.config.facet_row_guide_visible = guide_visible;
        self.config.facet_row_empty_cell_policy = empty_cell_policy;
        self.config.facet_row_order_expr = order_expr;
        self.config.facet_row_order_descending = order_descending;
    }

    #[doc(hidden)]
    pub fn facet_row_title_config(&self) -> Option<&str> {
        self.config.facet_row_title.as_deref()
    }

    #[doc(hidden)]
    pub fn facet_row_slot_sharing_config(&self) -> Option<Sharing> {
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
        slot_sharing: Option<Sharing>,
        position: Option<String>,
        guide_visible: Option<bool>,
        empty_cell_policy: Option<FacetEmptyCellPolicy>,
        order_expr: Option<LogicalExprNode>,
        order_descending: bool,
    ) {
        self.config.facet_col_title = title;
        self.config.facet_col_slot_sharing = slot_sharing;
        self.config.facet_col_position = position;
        self.config.facet_col_guide_visible = guide_visible;
        self.config.facet_col_empty_cell_policy = empty_cell_policy;
        self.config.facet_col_order_expr = order_expr;
        self.config.facet_col_order_descending = order_descending;
    }

    #[doc(hidden)]
    pub fn facet_col_title_config(&self) -> Option<&str> {
        self.config.facet_col_title.as_deref()
    }

    #[doc(hidden)]
    pub fn facet_col_slot_sharing_config(&self) -> Option<Sharing> {
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
        slot_sharing: Option<Sharing>,
        position: Option<String>,
        guide_visible: Option<bool>,
        empty_cell_policy: Option<FacetEmptyCellPolicy>,
        order_expr: Option<LogicalExprNode>,
        order_descending: bool,
        column_mode: FacetWrapColumnMode,
    ) {
        self.config.facet_wrap_title = title;
        self.config.facet_wrap_slot_sharing = slot_sharing;
        self.config.facet_wrap_position = position;
        self.config.facet_wrap_guide_visible = guide_visible;
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
    pub fn facet_wrap_slot_sharing_config(&self) -> Option<Sharing> {
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

    fn plot_width_config(&self) -> Option<f32> {
        self.config.plot_width
    }

    fn plot_height_config(&self) -> Option<f32> {
        self.config.plot_height
    }

    fn facet_row_title_config(&self) -> Option<&str> {
        self.config.facet_row_title.as_deref()
    }

    fn facet_row_slot_sharing_config(&self) -> Option<Sharing> {
        self.config.facet_row_slot_sharing
    }

    fn facet_row_position_config(&self) -> Option<&str> {
        self.config.facet_row_position.as_deref()
    }

    fn facet_row_guide_visible_config(&self) -> Option<bool> {
        self.config.facet_row_guide_visible
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

    fn facet_col_slot_sharing_config(&self) -> Option<Sharing> {
        self.config.facet_col_slot_sharing
    }

    fn facet_col_position_config(&self) -> Option<&str> {
        self.config.facet_col_position.as_deref()
    }

    fn facet_col_guide_visible_config(&self) -> Option<bool> {
        self.config.facet_col_guide_visible
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

    fn facet_wrap_slot_sharing_config(&self) -> Option<Sharing> {
        self.config.facet_wrap_slot_sharing
    }

    fn facet_wrap_position_config(&self) -> Option<&str> {
        self.config.facet_wrap_position.as_deref()
    }

    fn facet_wrap_guide_visible_config(&self) -> Option<bool> {
        self.config.facet_wrap_guide_visible
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
}
