use std::{any::Any, collections::HashMap, marker::PhantomData, sync::Arc};

use async_trait::async_trait;
use avenger_chart_core::{
    AvengerChartError, ChannelDescriptor, ChannelValue, ColorChannelConfig, CompiledDataContext,
    CompiledMark, CompiledMarkCore, CompiledMarkState, CoordinateSystemCore,
    CoordinateSystemTransformCore, CoordinationScope, DataContext, DataTransform,
    DataTransformCompileContext, DefaultLogicalExprNodeExt, EventDatumFieldSpec, FacetDataScope,
    IntoExpr, IntoPlotMark, LegendRendererKind, LegendRendererSelection, Mark, MarkDataMode,
    MarkRuntimeContext, MarkState, OpacityChannelConfig, PlotMark, RenderedMarkData,
    SizeChannelConfig, StoreData, StrokeWidthChannelConfig, TextMeasurementService,
    apply_opacity_to_color_channel, coerce_color_channel_with_renderer, coerce_font_style_channel,
    coerce_font_weight_channel, coerce_numeric_channel, coerce_numeric_channel_with_renderer,
    coerce_opacity_channel_with_renderer, coerce_text_align_channel, coerce_text_baseline_channel,
    coerce_text_channel, define_common_mark_channels, impl_mark_trait_common, is_continuous_scale,
};
use avenger_common::value::ScalarOrArray;
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::{mark::SceneMark, rect::SceneRectMark, text::SceneTextMark};
use avenger_text::{
    default_text_engine,
    measurement::{TextMeasurementConfig, truncate_text_to_limit_with},
    types::{FontStyle, FontWeight, FontWeightNameSpec, TextAlign, TextBaseline},
};
use datafusion::{
    arrow::{
        array::{ArrayRef, BooleanArray, Float64Array, Int64Array, StringArray},
        datatypes::{Field, Schema},
        record_batch::RecordBatch,
    },
    common::ScalarValue,
    dataframe::DataFrame,
    prelude::SessionContext,
};
use serde::{Deserialize, Serialize};

use crate::event::{
    HIERARCHY_CAN_ZOOM_FIELD, HIERARCHY_DEPTH_FIELD, HIERARCHY_DISPLAY_LEVELS_FIELD,
    HIERARCHY_HAS_HIDDEN_DESCENDANTS_FIELD, HIERARCHY_IS_DATA_LEAF_FIELD,
    HIERARCHY_IS_VISIBLE_LEAF_FIELD, HIERARCHY_LEVEL_NAME_FIELD, HIERARCHY_PARENT_PATH_ID_FIELD,
    HIERARCHY_PATH_ID_FIELD, HIERARCHY_SURFACE_KIND_COLLAPSED_RECT, HIERARCHY_SURFACE_KIND_FIELD,
    HIERARCHY_SURFACE_KIND_LEAF_RECT, HIERARCHY_SURFACE_KIND_NODE_HEADER,
    HIERARCHY_SURFACE_KIND_NODE_LABEL, HIERARCHY_SURFACE_KIND_NODE_RECT, HIERARCHY_TITLE_FIELD,
    HIERARCHY_VALUE_FIELD, HIERARCHY_VIEW_DEPTH_FIELD, RESERVED_GENERATED_EVENT_FIELDS,
    TREEMAP_RECT_HEIGHT_FIELD, TREEMAP_RECT_WIDTH_FIELD, TREEMAP_RECT_X_FIELD,
    TREEMAP_RECT_Y_FIELD, tree_rect_event_datum_field_specs,
};
use crate::layout::{path_id_for_components, scalar_is_null, scalar_label};
use crate::{ROOT_PATH_ID, Treemap, TreemapCoordMeasurement, VisibleTreemapNode};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum TreeRectNodeMode {
    #[default]
    VisibleLeaves,
    Depth(usize),
    AllVisible,
}

pub struct TreeRect<C = Treemap> {
    pub(crate) state: MarkState,
    pub(crate) node_mode: TreeRectNodeMode,
    pub(crate) _phantom: PhantomData<C>,
}

impl<C> Default for TreeRect<C> {
    fn default() -> Self {
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
                axis_configs: std::collections::HashMap::new(),
            },
            node_mode: TreeRectNodeMode::default(),
            _phantom: PhantomData,
        }
    }
}

impl<C> TreeRect<C> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.state.id = Some(id.into());
        self
    }

    pub fn data(mut self, dataframe: DataFrame) -> Self {
        self.state.data = DataContext::new(dataframe);
        self.state.data_mode = MarkDataMode::Inherit;
        self
    }

    pub fn data_store(mut self, data: StoreData) -> Self {
        self.state.data = DataContext::store_data(data);
        self.state.data_mode = MarkDataMode::Inherit;
        self
    }

    pub fn unit_data(mut self) -> Self {
        self.state.data_mode = MarkDataMode::Unit;
        self
    }

    pub fn exclude_from_scale_domains(mut self) -> Self {
        self.state.exclude_from_scale_domains = true;
        self
    }

    pub fn visible(mut self, visible: impl IntoExpr) -> Self {
        self.state.visible = Some(
            DefaultLogicalExprNodeExt::from_default_expr(visible.into_expr())
                .expect("Failed to serialize mark visible expr"),
        );
        self
    }

    pub fn transform<T, F>(self, transform: T, f: F) -> Self
    where
        T: DataTransform,
        F: FnOnce(Self, T::Output) -> Self,
    {
        self.transform_free(transform, f)
    }

    pub fn transform_free<T, F>(self, transform: T, f: F) -> Self
    where
        T: DataTransform,
        F: FnOnce(Self, T::Output) -> Self,
    {
        self.transform_with_scope(CoordinationScope::Free, transform, f)
    }

    pub fn transform_shared<T, F>(self, transform: T, f: F) -> Self
    where
        T: DataTransform,
        F: FnOnce(Self, T::Output) -> Self,
    {
        self.transform_with_scope(CoordinationScope::Shared, transform, f)
    }

    pub fn transform_level<T, F>(self, level: u8, transform: T, f: F) -> Self
    where
        T: DataTransform,
        F: FnOnce(Self, T::Output) -> Self,
    {
        self.transform_with_scope(CoordinationScope::Level(level), transform, f)
    }

    pub fn transform_with_scope<T, F>(
        mut self,
        scope: CoordinationScope,
        transform: T,
        f: F,
    ) -> Self
    where
        T: DataTransform,
        F: FnOnce(Self, T::Output) -> Self,
    {
        let scope = scope.to_normalized();
        let (compiled_transform, output) = transform
            .into_compiled_and_output(DataTransformCompileContext::new(scope))
            .expect("Failed to build data transform");
        self.state.data = self
            .state
            .data
            .with_transform_stage(scope, compiled_transform);
        f(self, output)
    }

    pub fn facet_data_scope(mut self, scope: FacetDataScope) -> Self {
        self.state.facet_data_scope = scope;
        self
    }

    pub fn facet_data_level(mut self, level: u8) -> Self {
        self.state.facet_data_scope = FacetDataScope::level(level);
        self
    }

    pub fn broadcast_to_facets(mut self) -> Self {
        self.state.facet_data_scope = FacetDataScope::BROADCAST;
        self
    }

    pub fn zindex(mut self, zindex: i32) -> Self {
        self.state.zindex = Some(zindex);
        self
    }

    pub fn state(&self) -> &MarkState {
        &self.state
    }

    pub fn state_mut(&mut self) -> &mut MarkState {
        &mut self.state
    }

    pub fn mark_state(&self) -> &MarkState {
        &self.state
    }

    pub fn mark_state_mut(&mut self) -> &mut MarkState {
        &mut self.state
    }

    pub fn get_data_context(&self) -> &DataContext {
        &self.state.data
    }

    #[doc(hidden)]
    pub fn with_channel_value(mut self, name: &str, value: ChannelValue) -> Self {
        self.state.data = self.state.data.with_channel_value(name, value);
        self
    }

    pub fn node_mode(mut self, mode: TreeRectNodeMode) -> Self {
        self.node_mode = mode;
        self
    }

    pub fn leaves(self) -> Self {
        self.node_mode(TreeRectNodeMode::VisibleLeaves)
    }

    pub fn depth(self, depth: usize) -> Self {
        self.node_mode(TreeRectNodeMode::Depth(depth))
    }

    pub fn all_visible(self) -> Self {
        self.node_mode(TreeRectNodeMode::AllVisible)
    }
}

impl<C> IntoPlotMark<C> for TreeRect<C>
where
    C: CoordinateSystemCore,
    TreeRect<C>: Mark<C> + Send + Sync + 'static,
{
    fn into_plot_marks(self) -> Vec<PlotMark<C>> {
        vec![PlotMark::from_mark(self)]
    }
}

define_common_mark_channels! {
    TreeRect {
        fill: {
            with_config: ColorChannelConfig,
        },
        stroke: {
            with_config: ColorChannelConfig,
        },
        stroke_width: {
            with_config: StrokeWidthChannelConfig,
        },
        opacity: {
            with_config: OpacityChannelConfig,
        },
        corner_radius: {},
        u: {},
        u2: {},
        v: {},
        v2: {},
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl Mark<Treemap> for TreeRect<Treemap> {
    impl_mark_trait_common!(TreeRect);

    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        _session_context: &SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        Ok(Arc::new(CompiledTreeRect {
            state: compiled_state,
            node_mode: self.node_mode.clone(),
        }))
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum TreeLabelFit {
    #[default]
    Ellipsis,
    Hide,
}

pub struct TreeLabel<C = Treemap> {
    pub(crate) state: MarkState,
    pub(crate) node_mode: TreeRectNodeMode,
    pub(crate) fit: TreeLabelFit,
    pub(crate) padding_px: f32,
    pub(crate) min_width_px: f32,
    pub(crate) min_height_px: f32,
    pub(crate) _phantom: PhantomData<C>,
}

impl<C> Default for TreeLabel<C> {
    fn default() -> Self {
        Self {
            state: MarkState {
                id: None,
                data: DataContext::default(),
                view: None,
                data_mode: MarkDataMode::Inherit,
                facet_data_scope: FacetDataScope::FILTERED,
                exclude_from_scale_domains: true,
                visible: None,
                details: None,
                zindex: None,
                geometry_space: None,
                axis_configs: std::collections::HashMap::new(),
            },
            node_mode: TreeRectNodeMode::VisibleLeaves,
            fit: TreeLabelFit::Ellipsis,
            padding_px: 4.0,
            min_width_px: 10.0,
            min_height_px: 8.0,
            _phantom: PhantomData,
        }
    }
}

impl<C> TreeLabel<C> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.state.id = Some(id.into());
        self
    }

    pub fn data(mut self, dataframe: DataFrame) -> Self {
        self.state.data = DataContext::new(dataframe);
        self.state.data_mode = MarkDataMode::Inherit;
        self
    }

    pub fn data_store(mut self, data: StoreData) -> Self {
        self.state.data = DataContext::store_data(data);
        self.state.data_mode = MarkDataMode::Inherit;
        self
    }

    pub fn unit_data(mut self) -> Self {
        self.state.data_mode = MarkDataMode::Unit;
        self
    }

    pub fn visible(mut self, visible: impl IntoExpr) -> Self {
        self.state.visible = Some(
            DefaultLogicalExprNodeExt::from_default_expr(visible.into_expr())
                .expect("Failed to serialize mark visible expr"),
        );
        self
    }

    pub fn transform<T, F>(self, transform: T, f: F) -> Self
    where
        T: DataTransform,
        F: FnOnce(Self, T::Output) -> Self,
    {
        self.transform_free(transform, f)
    }

    pub fn transform_free<T, F>(self, transform: T, f: F) -> Self
    where
        T: DataTransform,
        F: FnOnce(Self, T::Output) -> Self,
    {
        self.transform_with_scope(CoordinationScope::Free, transform, f)
    }

    pub fn transform_shared<T, F>(self, transform: T, f: F) -> Self
    where
        T: DataTransform,
        F: FnOnce(Self, T::Output) -> Self,
    {
        self.transform_with_scope(CoordinationScope::Shared, transform, f)
    }

    pub fn transform_level<T, F>(self, level: u8, transform: T, f: F) -> Self
    where
        T: DataTransform,
        F: FnOnce(Self, T::Output) -> Self,
    {
        self.transform_with_scope(CoordinationScope::Level(level), transform, f)
    }

    pub fn transform_with_scope<T, F>(
        mut self,
        scope: CoordinationScope,
        transform: T,
        f: F,
    ) -> Self
    where
        T: DataTransform,
        F: FnOnce(Self, T::Output) -> Self,
    {
        let scope = scope.to_normalized();
        let (compiled_transform, output) = transform
            .into_compiled_and_output(DataTransformCompileContext::new(scope))
            .expect("Failed to build data transform");
        self.state.data = self
            .state
            .data
            .with_transform_stage(scope, compiled_transform);
        f(self, output)
    }

    pub fn facet_data_scope(mut self, scope: FacetDataScope) -> Self {
        self.state.facet_data_scope = scope;
        self
    }

    pub fn facet_data_level(mut self, level: u8) -> Self {
        self.state.facet_data_scope = FacetDataScope::level(level);
        self
    }

    pub fn broadcast_to_facets(mut self) -> Self {
        self.state.facet_data_scope = FacetDataScope::BROADCAST;
        self
    }

    pub fn zindex(mut self, zindex: i32) -> Self {
        self.state.zindex = Some(zindex);
        self
    }

    pub fn state(&self) -> &MarkState {
        &self.state
    }

    pub fn state_mut(&mut self) -> &mut MarkState {
        &mut self.state
    }

    pub fn mark_state(&self) -> &MarkState {
        &self.state
    }

    pub fn mark_state_mut(&mut self) -> &mut MarkState {
        &mut self.state
    }

    pub fn get_data_context(&self) -> &DataContext {
        &self.state.data
    }

    #[doc(hidden)]
    pub fn with_channel_value(mut self, name: &str, value: ChannelValue) -> Self {
        self.state.data = self.state.data.with_channel_value(name, value);
        self
    }

    pub fn node_mode(mut self, mode: TreeRectNodeMode) -> Self {
        self.node_mode = mode;
        self
    }

    pub fn leaves(self) -> Self {
        self.node_mode(TreeRectNodeMode::VisibleLeaves)
    }

    pub fn depth(self, depth: usize) -> Self {
        self.node_mode(TreeRectNodeMode::Depth(depth))
    }

    pub fn all_visible(self) -> Self {
        self.node_mode(TreeRectNodeMode::AllVisible)
    }

    pub fn fit(mut self, fit: TreeLabelFit) -> Self {
        self.fit = fit;
        self
    }

    pub fn padding(mut self, padding_px: f32) -> Self {
        self.padding_px = padding_px;
        self
    }

    pub fn min_size_px(mut self, width: f32, height: f32) -> Self {
        self.min_width_px = width;
        self.min_height_px = height;
        self
    }

    pub fn text<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("text", value.into().no_scale())
    }

    pub fn align<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("align", value.into().no_scale())
    }

    pub fn baseline<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("baseline", value.into().no_scale())
    }

    pub fn font<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("font", value.into().no_scale())
    }

    pub fn font_weight<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("font_weight", value.into().no_scale())
    }

    pub fn font_style<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("font_style", value.into().no_scale())
    }
}

impl<C> IntoPlotMark<C> for TreeLabel<C>
where
    C: CoordinateSystemCore,
    TreeLabel<C>: Mark<C> + Send + Sync + 'static,
{
    fn into_plot_marks(self) -> Vec<PlotMark<C>> {
        vec![PlotMark::from_mark(self)]
    }
}

define_common_mark_channels! {
    TreeLabel {
        color: {
            allow_column: true,
            with_config: ColorChannelConfig,
        },
        font_size: {
            allow_column: true,
            with_config: SizeChannelConfig,
        },
        opacity: {
            allow_column: true,
            with_config: OpacityChannelConfig,
        },
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl Mark<Treemap> for TreeLabel<Treemap> {
    impl_mark_trait_common!(TreeLabel);

    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        _session_context: &SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        Ok(Arc::new(CompiledTreeLabel {
            state: compiled_state,
            node_mode: self.node_mode.clone(),
            fit: self.fit,
            padding_px: self.padding_px,
            min_width_px: self.min_width_px,
            min_height_px: self.min_height_px,
        }))
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledTreeLabel {
    pub(crate) state: CompiledMarkState,
    pub(crate) node_mode: TreeRectNodeMode,
    pub(crate) fit: TreeLabelFit,
    pub(crate) padding_px: f32,
    pub(crate) min_width_px: f32,
    pub(crate) min_height_px: f32,
}

impl CompiledMarkCore for CompiledTreeLabel {
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
        "tree_label"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn supported_channels(&self) -> Vec<ChannelDescriptor> {
        vec![
            ChannelDescriptor {
                name: "text",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "color",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "font_size",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "opacity",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "align",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "baseline",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "font",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "font_weight",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "font_style",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
        ]
    }

    fn wants_full_data_batch(&self) -> bool {
        true
    }

    fn event_datum_field_specs(&self) -> Vec<EventDatumFieldSpec> {
        tree_rect_event_datum_field_specs()
    }

    fn mark_specific_default(&self, channel: &str) -> Option<ScalarValue> {
        tree_label_channel_defaults(channel)
    }
}

#[typetag::serde]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl CompiledMark for CompiledTreeLabel {
    async fn render_from_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &dyn MarkRuntimeContext,
        coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        self.render_mark_data(data, scalars, context, coord)
            .await
            .map(|rendered| rendered.marks)
    }

    async fn render_mark_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &dyn MarkRuntimeContext,
        _coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<RenderedMarkData, AvengerChartError> {
        validate_no_reserved_event_field_collisions(data)?;
        let measurement = context
            .coord_measurement()
            .as_any()
            .downcast_ref::<TreemapCoordMeasurement>()
            .ok_or_else(|| {
                AvengerChartError::InternalError(
                    "TreeLabel requires TreemapCoordMeasurement".to_string(),
                )
            })?;
        let selected_nodes = select_nodes(measurement.visible_nodes(), &self.node_mode);
        let joined = if self.should_join_render_data(data) {
            nodes_for_render_data(data, measurement, &selected_nodes)?
        } else {
            let source_row_indices = source_row_indices_for_event_rows(data, &selected_nodes)?;
            RenderNodeSelection {
                nodes: selected_nodes,
                source_row_indices,
            }
        };
        let nodes = joined.nodes;
        let len = nodes.len();
        let source_row_indices = joined.source_row_indices;
        let channel_indices = if self.should_join_render_data(data) {
            Some(Arc::new(source_row_indices.clone()))
        } else {
            None
        };

        let mark_context = context.core_view();
        let text = if self.state.data.channels().contains_key("text") {
            gather_scalar_or_array(
                coerce_text_channel(data, scalars, "text", String::new())?,
                len,
                channel_indices.as_ref(),
            )
            .as_vec(len, None)
        } else {
            nodes
                .iter()
                .map(|node| node.node.label.clone())
                .collect::<Vec<_>>()
        };
        let color = coerce_color_channel_with_renderer(
            self,
            data,
            scalars,
            "color",
            &mark_context,
            [1.0, 1.0, 1.0, 1.0],
        )?;
        let opacity = coerce_opacity_channel_with_renderer(
            self,
            data,
            scalars,
            "opacity",
            &mark_context,
            1.0,
        )?;
        let color = gather_scalar_or_array(color, len, channel_indices.as_ref());
        let opacity = gather_scalar_or_array(opacity, len, channel_indices.as_ref());
        let color = apply_opacity_to_color_channel(color, &opacity, len);
        let font = gather_scalar_or_array(
            coerce_text_channel(data, scalars, "font", "sans-serif".to_string())?,
            len,
            channel_indices.as_ref(),
        )
        .as_vec(len, None);
        let font_size = gather_scalar_or_array(
            coerce_numeric_channel(data, scalars, "font_size", 11.0)?,
            len,
            channel_indices.as_ref(),
        )
        .as_vec(len, None);
        let font_weight = gather_scalar_or_array(
            coerce_font_weight_channel(data, scalars, "font_weight", FontWeight::Number(300.0))?,
            len,
            channel_indices.as_ref(),
        )
        .as_vec(len, None);
        let font_style = gather_scalar_or_array(
            coerce_font_style_channel(data, scalars, "font_style", FontStyle::Normal)?,
            len,
            channel_indices.as_ref(),
        )
        .as_vec(len, None);
        let align = gather_scalar_or_array(
            coerce_text_align_channel(data, scalars, "align", TextAlign::Center)?,
            len,
            channel_indices.as_ref(),
        );
        let baseline = gather_scalar_or_array(
            coerce_text_baseline_channel(data, scalars, "baseline", TextBaseline::Middle)?,
            len,
            channel_indices.as_ref(),
        );

        let mut fitted_text = Vec::with_capacity(len);
        let mut x = Vec::with_capacity(len);
        let mut y = Vec::with_capacity(len);
        let mut limit = Vec::with_capacity(len);
        for (index, node) in nodes.iter().enumerate() {
            let label_rect = node.label_rect.inset(self.padding_px);
            let text_limit = label_rect.width.max(0.0);
            let available_height = label_rect.height.max(0.0);
            let label = fit_tree_label(
                &text[index],
                text_limit,
                available_height,
                &font[index],
                font_size[index],
                &font_weight[index],
                &font_style[index],
                self.fit,
                self.min_width_px,
                self.min_height_px,
                context.text_measurement_service(),
            );
            fitted_text.push(label);
            x.push(label_rect.x + label_rect.width * 0.5);
            y.push(label_rect.y + label_rect.height * 0.5);
            limit.push(text_limit);
        }

        let event_rows = tree_node_event_datum_batch(&nodes, HIERARCHY_SURFACE_KIND_NODE_LABEL)?;

        Ok(
            RenderedMarkData::with_source_row_indices_and_event_datum_rows(
                vec![SceneMark::Text(Arc::new(SceneTextMark {
                    name: self
                        .state
                        .id
                        .clone()
                        .unwrap_or_else(|| "tree_label".to_string()),
                    interactive: true,
                    clip: true,
                    len: len as u32,
                    text: ScalarOrArray::from(fitted_text),
                    x: ScalarOrArray::from(x),
                    y: ScalarOrArray::from(y),
                    align,
                    baseline,
                    angle: ScalarOrArray::new_scalar(0.0),
                    color,
                    font: ScalarOrArray::from(font),
                    font_size: ScalarOrArray::from(font_size),
                    font_weight: ScalarOrArray::from(font_weight),
                    font_style: ScalarOrArray::from(font_style),
                    limit: ScalarOrArray::from(limit),
                    indices: None,
                    zindex: self.state.zindex,
                    ..SceneTextMark::default()
                }))],
                vec![source_row_indices],
                vec![event_rows],
            ),
        )
    }
}

impl CompiledTreeLabel {
    fn should_join_render_data(&self, data: Option<&RecordBatch>) -> bool {
        let Some(data) = data else {
            return false;
        };
        self.state
            .data
            .channels()
            .keys()
            .any(|channel| data.column_by_name(channel).is_some())
    }
}

pub struct TreeHeader<C = Treemap> {
    pub(crate) state: MarkState,
    pub(crate) min_relative_depth: usize,
    pub(crate) max_relative_depth: usize,
    pub(crate) padding_px: f32,
    pub(crate) _phantom: PhantomData<C>,
}

impl<C> Default for TreeHeader<C> {
    fn default() -> Self {
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
                zindex: Some(40),
                geometry_space: None,
                axis_configs: std::collections::HashMap::new(),
            },
            min_relative_depth: 1,
            max_relative_depth: 1,
            padding_px: 2.0,
            _phantom: PhantomData,
        }
    }
}

impl<C> TreeHeader<C> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.state.id = Some(id.into());
        self
    }

    pub fn data(mut self, dataframe: DataFrame) -> Self {
        self.state.data = DataContext::new(dataframe);
        self.state.data_mode = MarkDataMode::Inherit;
        self
    }

    pub fn data_store(mut self, data: StoreData) -> Self {
        self.state.data = DataContext::store_data(data);
        self.state.data_mode = MarkDataMode::Inherit;
        self
    }

    pub fn visible(mut self, visible: impl IntoExpr) -> Self {
        self.state.visible = Some(
            DefaultLogicalExprNodeExt::from_default_expr(visible.into_expr())
                .expect("Failed to serialize mark visible expr"),
        );
        self
    }

    pub fn transform<T, F>(self, transform: T, f: F) -> Self
    where
        T: DataTransform,
        F: FnOnce(Self, T::Output) -> Self,
    {
        self.transform_free(transform, f)
    }

    pub fn transform_free<T, F>(self, transform: T, f: F) -> Self
    where
        T: DataTransform,
        F: FnOnce(Self, T::Output) -> Self,
    {
        self.transform_with_scope(CoordinationScope::Free, transform, f)
    }

    pub fn transform_shared<T, F>(self, transform: T, f: F) -> Self
    where
        T: DataTransform,
        F: FnOnce(Self, T::Output) -> Self,
    {
        self.transform_with_scope(CoordinationScope::Shared, transform, f)
    }

    pub fn transform_level<T, F>(self, level: u8, transform: T, f: F) -> Self
    where
        T: DataTransform,
        F: FnOnce(Self, T::Output) -> Self,
    {
        self.transform_with_scope(CoordinationScope::Level(level), transform, f)
    }

    pub fn transform_with_scope<T, F>(
        mut self,
        scope: CoordinationScope,
        transform: T,
        f: F,
    ) -> Self
    where
        T: DataTransform,
        F: FnOnce(Self, T::Output) -> Self,
    {
        let scope = scope.to_normalized();
        let (compiled_transform, output) = transform
            .into_compiled_and_output(DataTransformCompileContext::new(scope))
            .expect("Failed to build data transform");
        self.state.data = self
            .state
            .data
            .with_transform_stage(scope, compiled_transform);
        f(self, output)
    }

    pub fn facet_data_scope(mut self, scope: FacetDataScope) -> Self {
        self.state.facet_data_scope = scope;
        self
    }

    pub fn facet_data_level(mut self, level: u8) -> Self {
        self.state.facet_data_scope = FacetDataScope::level(level);
        self
    }

    pub fn broadcast_to_facets(mut self) -> Self {
        self.state.facet_data_scope = FacetDataScope::BROADCAST;
        self
    }

    pub fn zindex(mut self, zindex: i32) -> Self {
        self.state.zindex = Some(zindex);
        self
    }

    pub fn state(&self) -> &MarkState {
        &self.state
    }

    pub fn state_mut(&mut self) -> &mut MarkState {
        &mut self.state
    }

    pub fn mark_state(&self) -> &MarkState {
        &self.state
    }

    pub fn mark_state_mut(&mut self) -> &mut MarkState {
        &mut self.state
    }

    pub fn get_data_context(&self) -> &DataContext {
        &self.state.data
    }

    #[doc(hidden)]
    pub fn with_channel_value(mut self, name: &str, value: ChannelValue) -> Self {
        self.state.data = self.state.data.with_channel_value(name, value);
        self
    }

    pub fn depth_range(mut self, range: std::ops::RangeInclusive<usize>) -> Self {
        self.min_relative_depth = *range.start();
        self.max_relative_depth = *range.end();
        self
    }

    pub fn padding(mut self, padding_px: f32) -> Self {
        self.padding_px = padding_px;
        self
    }

    pub fn text<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("text", value.into().no_scale())
    }

    pub fn font<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("font", value.into().no_scale())
    }

    pub fn font_weight<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("font_weight", value.into().no_scale())
    }

    pub fn font_style<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("font_style", value.into().no_scale())
    }
}

impl<C> IntoPlotMark<C> for TreeHeader<C>
where
    C: CoordinateSystemCore,
    TreeHeader<C>: Mark<C> + Send + Sync + 'static,
{
    fn into_plot_marks(self) -> Vec<PlotMark<C>> {
        vec![PlotMark::from_mark(self)]
    }
}

define_common_mark_channels! {
    TreeHeader {
        fill: {
            allow_column: true,
            with_config: ColorChannelConfig,
        },
        stroke: {
            allow_column: true,
            with_config: ColorChannelConfig,
        },
        stroke_width: {
            allow_column: true,
            with_config: StrokeWidthChannelConfig,
        },
        opacity: {
            allow_column: true,
            with_config: OpacityChannelConfig,
        },
        text_color: {
            allow_column: true,
            with_config: ColorChannelConfig,
        },
        font_size: {
            allow_column: true,
            with_config: SizeChannelConfig,
        },
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl Mark<Treemap> for TreeHeader<Treemap> {
    impl_mark_trait_common!(TreeHeader);

    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        _session_context: &SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        Ok(Arc::new(CompiledTreeHeader {
            state: compiled_state,
            min_relative_depth: self.min_relative_depth,
            max_relative_depth: self.max_relative_depth,
            padding_px: self.padding_px,
        }))
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledTreeHeader {
    pub(crate) state: CompiledMarkState,
    pub(crate) min_relative_depth: usize,
    pub(crate) max_relative_depth: usize,
    pub(crate) padding_px: f32,
}

impl CompiledMarkCore for CompiledTreeHeader {
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
        "tree_header"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn supported_channels(&self) -> Vec<ChannelDescriptor> {
        vec![
            ChannelDescriptor {
                name: "fill",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "stroke",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "stroke_width",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "opacity",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "text",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "text_color",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "font_size",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "font",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "font_weight",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "font_style",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
        ]
    }

    fn wants_full_data_batch(&self) -> bool {
        true
    }

    fn event_datum_field_specs(&self) -> Vec<EventDatumFieldSpec> {
        tree_rect_event_datum_field_specs()
    }

    fn mark_specific_default(&self, channel: &str) -> Option<ScalarValue> {
        tree_header_channel_defaults(channel)
    }

    fn preferred_legend_renderer(
        &self,
        channel: &str,
        scale: &ConfiguredScale,
    ) -> Option<LegendRendererSelection> {
        tree_rect_legend_renderer(&self.state, channel, scale)
    }
}

#[typetag::serde]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl CompiledMark for CompiledTreeHeader {
    async fn render_from_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &dyn MarkRuntimeContext,
        coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        self.render_mark_data(data, scalars, context, coord)
            .await
            .map(|rendered| rendered.marks)
    }

    async fn render_mark_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &dyn MarkRuntimeContext,
        _coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<RenderedMarkData, AvengerChartError> {
        validate_no_reserved_event_field_collisions(data)?;
        let measurement = context
            .coord_measurement()
            .as_any()
            .downcast_ref::<TreemapCoordMeasurement>()
            .ok_or_else(|| {
                AvengerChartError::InternalError(
                    "TreeHeader requires TreemapCoordMeasurement".to_string(),
                )
            })?;
        let selected_nodes = select_header_nodes(
            measurement.visible_nodes(),
            self.min_relative_depth,
            self.max_relative_depth,
        );
        let source_row_indices = source_row_indices_for_event_rows(data, &selected_nodes)?;
        let joined = RenderNodeSelection {
            nodes: selected_nodes,
            source_row_indices,
        };
        let nodes = joined.nodes;
        let len = nodes.len();
        let source_row_indices = joined.source_row_indices;
        let channel_indices = if self.should_join_render_data(data) {
            Some(Arc::new(source_row_indices.clone()))
        } else {
            None
        };

        let mark_context = context.core_view();
        let fill = coerce_color_channel_with_renderer(
            self,
            data,
            scalars,
            "fill",
            &mark_context,
            [70.0 / 255.0, 130.0 / 255.0, 180.0 / 255.0, 1.0],
        )?;
        let stroke = coerce_color_channel_with_renderer(
            self,
            data,
            scalars,
            "stroke",
            &mark_context,
            [1.0, 1.0, 1.0, 1.0],
        )?;
        let opacity = coerce_opacity_channel_with_renderer(
            self,
            data,
            scalars,
            "opacity",
            &mark_context,
            1.0,
        )?;
        let fill = gather_scalar_or_array(fill, len, channel_indices.as_ref());
        let stroke = gather_scalar_or_array(stroke, len, channel_indices.as_ref());
        let opacity = gather_scalar_or_array(opacity, len, channel_indices.as_ref());
        let fill = apply_opacity_to_color_channel(fill, &opacity, len);
        let stroke = apply_opacity_to_color_channel(stroke, &opacity, len);
        let stroke_width = gather_scalar_or_array(
            coerce_numeric_channel_with_renderer(
                self,
                data,
                scalars,
                "stroke_width",
                &mark_context,
                1.0,
            )?,
            len,
            channel_indices.as_ref(),
        );

        let text = if self.state.data.channels().contains_key("text") {
            gather_scalar_or_array(
                coerce_text_channel(data, scalars, "text", String::new())?,
                len,
                channel_indices.as_ref(),
            )
            .as_vec(len, None)
        } else {
            nodes
                .iter()
                .map(|node| node.node.label.clone())
                .collect::<Vec<_>>()
        };
        let text_color = gather_scalar_or_array(
            coerce_color_channel_with_renderer(
                self,
                data,
                scalars,
                "text_color",
                &mark_context,
                [1.0, 1.0, 1.0, 1.0],
            )?,
            len,
            channel_indices.as_ref(),
        );
        let font = gather_scalar_or_array(
            coerce_text_channel(data, scalars, "font", "sans-serif".to_string())?,
            len,
            channel_indices.as_ref(),
        )
        .as_vec(len, None);
        let font_size = gather_scalar_or_array(
            coerce_numeric_channel(data, scalars, "font_size", 14.0)?,
            len,
            channel_indices.as_ref(),
        )
        .as_vec(len, None);
        let font_weight = gather_scalar_or_array(
            coerce_font_weight_channel(
                data,
                scalars,
                "font_weight",
                FontWeight::Name(FontWeightNameSpec::Bold),
            )?,
            len,
            channel_indices.as_ref(),
        )
        .as_vec(len, None);
        let font_style = gather_scalar_or_array(
            coerce_font_style_channel(data, scalars, "font_style", FontStyle::Normal)?,
            len,
            channel_indices.as_ref(),
        )
        .as_vec(len, None);

        let mut rect_x = Vec::with_capacity(len);
        let mut rect_y = Vec::with_capacity(len);
        let mut rect_width = Vec::with_capacity(len);
        let mut rect_height = Vec::with_capacity(len);
        let mut fitted_text = Vec::with_capacity(len);
        let mut text_x = Vec::with_capacity(len);
        let mut text_y = Vec::with_capacity(len);
        let mut text_limit = Vec::with_capacity(len);
        for (index, node) in nodes.iter().enumerate() {
            let rect = node.header_rect.unwrap_or(node.outer_rect);
            rect_x.push(rect.x);
            rect_y.push(rect.y);
            rect_width.push(rect.width);
            rect_height.push(rect.height);
            let label_rect = rect.inset(self.padding_px);
            let limit = label_rect.width.max(0.0);
            fitted_text.push(fit_tree_label(
                &text[index],
                limit,
                label_rect.height.max(0.0),
                &font[index],
                font_size[index],
                &font_weight[index],
                &font_style[index],
                TreeLabelFit::Ellipsis,
                4.0,
                4.0,
                context.text_measurement_service(),
            ));
            text_x.push(label_rect.x + label_rect.width * 0.5);
            text_y.push(label_rect.y + label_rect.height * 0.5);
            text_limit.push(limit);
        }

        let rect_name = self
            .state
            .id
            .clone()
            .unwrap_or_else(|| "tree_header".to_string());
        let text_name = format!("{rect_name}_text");
        let event_rows = tree_header_event_datum_batch(&nodes)?;
        let text_event_rows = event_rows.clone();
        let text_zindex = self.state.zindex.map(|zindex| zindex + 1);

        Ok(
            RenderedMarkData::with_source_row_indices_and_event_datum_rows(
                vec![
                    SceneMark::Rect(SceneRectMark {
                        name: rect_name,
                        interactive: true,
                        clip: true,
                        len: len as u32,
                        gradients: Vec::new(),
                        x: ScalarOrArray::from(rect_x),
                        y: ScalarOrArray::from(rect_y),
                        width: Some(ScalarOrArray::from(rect_width)),
                        height: Some(ScalarOrArray::from(rect_height)),
                        x2: None,
                        y2: None,
                        fill,
                        fill_pattern: avenger_scenegraph::marks::pattern::default_no_fill_pattern(),
                        stroke,
                        stroke_width,
                        corner_radius: ScalarOrArray::new_scalar(0.0),
                        indices: None,
                        zindex: self.state.zindex,
                    }),
                    SceneMark::Text(Arc::new(SceneTextMark {
                        name: text_name,
                        interactive: true,
                        clip: true,
                        len: len as u32,
                        text: ScalarOrArray::from(fitted_text),
                        x: ScalarOrArray::from(text_x),
                        y: ScalarOrArray::from(text_y),
                        align: ScalarOrArray::new_scalar(TextAlign::Center),
                        baseline: ScalarOrArray::new_scalar(TextBaseline::Middle),
                        angle: ScalarOrArray::new_scalar(0.0),
                        color: text_color,
                        font: ScalarOrArray::from(font),
                        font_size: ScalarOrArray::from(font_size),
                        font_weight: ScalarOrArray::from(font_weight),
                        font_style: ScalarOrArray::from(font_style),
                        limit: ScalarOrArray::from(text_limit),
                        indices: None,
                        zindex: text_zindex,
                        ..SceneTextMark::default()
                    })),
                ],
                vec![source_row_indices.clone(), source_row_indices],
                vec![event_rows, text_event_rows],
            ),
        )
    }
}

impl CompiledTreeHeader {
    fn should_join_render_data(&self, data: Option<&RecordBatch>) -> bool {
        let Some(data) = data else {
            return false;
        };
        self.state
            .data
            .channels()
            .keys()
            .any(|channel| data.column_by_name(channel).is_some())
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledTreeRect {
    pub(crate) state: CompiledMarkState,
    pub(crate) node_mode: TreeRectNodeMode,
}

impl CompiledMarkCore for CompiledTreeRect {
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
        "tree_rect"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn supported_channels(&self) -> Vec<ChannelDescriptor> {
        vec![
            ChannelDescriptor {
                name: "fill",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "stroke",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "stroke_width",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "opacity",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "corner_radius",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "u",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "u2",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "v",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "v2",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
        ]
    }

    fn wants_full_data_batch(&self) -> bool {
        true
    }

    fn event_datum_field_specs(&self) -> Vec<EventDatumFieldSpec> {
        tree_rect_event_datum_field_specs()
    }

    fn mark_specific_default(&self, channel: &str) -> Option<ScalarValue> {
        tree_rect_channel_defaults(channel)
    }

    fn preferred_legend_renderer(
        &self,
        channel: &str,
        scale: &ConfiguredScale,
    ) -> Option<LegendRendererSelection> {
        tree_rect_legend_renderer(&self.state, channel, scale)
    }
}

#[typetag::serde]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl CompiledMark for CompiledTreeRect {
    async fn render_from_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &dyn MarkRuntimeContext,
        coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        self.render_mark_data(data, scalars, context, coord)
            .await
            .map(|rendered| rendered.marks)
    }

    async fn render_mark_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &dyn MarkRuntimeContext,
        _coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<RenderedMarkData, AvengerChartError> {
        validate_no_reserved_event_field_collisions(data)?;
        let measurement = context
            .coord_measurement()
            .as_any()
            .downcast_ref::<TreemapCoordMeasurement>()
            .ok_or_else(|| {
                AvengerChartError::InternalError(
                    "TreeRect requires TreemapCoordMeasurement".to_string(),
                )
            })?;
        let selected_nodes = select_nodes(measurement.visible_nodes(), &self.node_mode);
        let joined = if self.should_join_render_data(data) {
            nodes_for_render_data(data, measurement, &selected_nodes)?
        } else {
            let source_row_indices = source_row_indices_for_event_rows(data, &selected_nodes)?;
            RenderNodeSelection {
                nodes: selected_nodes,
                source_row_indices,
            }
        };
        let nodes = joined.nodes;
        let len = nodes.len();
        let source_row_indices = joined.source_row_indices;
        let channel_indices = if self.should_join_render_data(data) {
            Some(Arc::new(source_row_indices.clone()))
        } else {
            None
        };

        let mark_context = context.core_view();
        let fill = coerce_color_channel_with_renderer(
            self,
            data,
            scalars,
            "fill",
            &mark_context,
            [70.0 / 255.0, 130.0 / 255.0, 180.0 / 255.0, 1.0],
        )?;
        let stroke = coerce_color_channel_with_renderer(
            self,
            data,
            scalars,
            "stroke",
            &mark_context,
            [0.0, 0.0, 0.0, 1.0],
        )?;
        let opacity = coerce_opacity_channel_with_renderer(
            self,
            data,
            scalars,
            "opacity",
            &mark_context,
            1.0,
        )?;
        let fill = gather_scalar_or_array(fill, len, channel_indices.as_ref());
        let stroke = gather_scalar_or_array(stroke, len, channel_indices.as_ref());
        let opacity = gather_scalar_or_array(opacity, len, channel_indices.as_ref());
        let fill = apply_opacity_to_color_channel(fill, &opacity, len);
        let stroke = apply_opacity_to_color_channel(stroke, &opacity, len);
        let stroke_width = coerce_numeric_channel_with_renderer(
            self,
            data,
            scalars,
            "stroke_width",
            &mark_context,
            1.0,
        )?;
        let stroke_width = gather_scalar_or_array(stroke_width, len, channel_indices.as_ref());
        let corner_radius = coerce_numeric_channel_with_renderer(
            self,
            data,
            scalars,
            "corner_radius",
            &mark_context,
            0.0,
        )?;
        let corner_radius = gather_scalar_or_array(corner_radius, len, channel_indices.as_ref());
        let u = coerce_numeric_channel_with_renderer(self, data, scalars, "u", &mark_context, 0.0)?;
        let u2 =
            coerce_numeric_channel_with_renderer(self, data, scalars, "u2", &mark_context, 1.0)?;
        let v = coerce_numeric_channel_with_renderer(self, data, scalars, "v", &mark_context, 0.0)?;
        let v2 =
            coerce_numeric_channel_with_renderer(self, data, scalars, "v2", &mark_context, 1.0)?;
        let u = u.as_vec(len, channel_indices.as_ref());
        let u2 = u2.as_vec(len, channel_indices.as_ref());
        let v = v.as_vec(len, channel_indices.as_ref());
        let v2 = v2.as_vec(len, channel_indices.as_ref());
        let mut x = Vec::with_capacity(len);
        let mut y = Vec::with_capacity(len);
        let mut width = Vec::with_capacity(len);
        let mut height = Vec::with_capacity(len);
        for (index, node) in nodes.iter().enumerate() {
            let u0 = u[index].clamp(0.0, 1.0);
            let u1 = u2[index].clamp(0.0, 1.0);
            let v0 = v[index].clamp(0.0, 1.0);
            let v1 = v2[index].clamp(0.0, 1.0);
            let u_min = u0.min(u1);
            let u_max = u0.max(u1);
            let v_min = v0.min(v1);
            let v_max = v0.max(v1);
            x.push(node.rect.x + node.rect.width * u_min);
            y.push(node.rect.y + node.rect.height * v_min);
            width.push(node.rect.width * (u_max - u_min));
            height.push(node.rect.height * (v_max - v_min));
        }

        let event_rows = tree_rect_event_datum_batch(&nodes)?;

        Ok(
            RenderedMarkData::with_source_row_indices_and_event_datum_rows(
                vec![SceneMark::Rect(SceneRectMark {
                    name: self
                        .state
                        .id
                        .clone()
                        .unwrap_or_else(|| "tree_rect".to_string()),
                    interactive: true,
                    clip: true,
                    len: len as u32,
                    gradients: Vec::new(),
                    x: ScalarOrArray::from(x),
                    y: ScalarOrArray::from(y),
                    width: Some(ScalarOrArray::from(width)),
                    height: Some(ScalarOrArray::from(height)),
                    x2: None,
                    y2: None,
                    fill,
                    fill_pattern: avenger_scenegraph::marks::pattern::default_no_fill_pattern(),
                    stroke,
                    stroke_width,
                    corner_radius,
                    indices: None,
                    zindex: self.state.zindex,
                })],
                vec![source_row_indices],
                vec![event_rows],
            ),
        )
    }
}

impl CompiledTreeRect {
    fn should_join_render_data(&self, data: Option<&RecordBatch>) -> bool {
        let Some(data) = data else {
            return false;
        };
        self.state
            .data
            .channels()
            .keys()
            .any(|channel| data.column_by_name(channel).is_some())
    }
}

fn select_nodes<'a>(
    nodes: &'a [VisibleTreemapNode],
    mode: &TreeRectNodeMode,
) -> Vec<&'a VisibleTreemapNode> {
    nodes
        .iter()
        .filter(|node| match mode {
            TreeRectNodeMode::VisibleLeaves => node.is_visible_leaf,
            TreeRectNodeMode::Depth(depth) => {
                node.node.depth == *depth && node.node.path_id != ROOT_PATH_ID
            }
            TreeRectNodeMode::AllVisible => node.node.path_id != ROOT_PATH_ID,
        })
        .collect()
}

fn select_header_nodes(
    nodes: &[VisibleTreemapNode],
    min_relative_depth: usize,
    max_relative_depth: usize,
) -> Vec<&VisibleTreemapNode> {
    nodes
        .iter()
        .filter(|node| node.node.path_id != ROOT_PATH_ID)
        .filter(|node| node.header_rect.is_some())
        .filter(|node| {
            let relative_depth = node.node.depth.saturating_sub(node.view_depth);
            relative_depth >= min_relative_depth && relative_depth <= max_relative_depth
        })
        .collect()
}

struct RenderNodeSelection<'a> {
    nodes: Vec<&'a VisibleTreemapNode>,
    source_row_indices: Vec<usize>,
}

fn nodes_for_render_data<'a>(
    data: Option<&RecordBatch>,
    measurement: &'a TreemapCoordMeasurement,
    selected_nodes: &[&'a VisibleTreemapNode],
) -> Result<RenderNodeSelection<'a>, AvengerChartError> {
    let Some(data) = data else {
        return Ok(RenderNodeSelection {
            nodes: selected_nodes.to_vec(),
            source_row_indices: Vec::new(),
        });
    };
    if data.num_rows() == 0 {
        return Ok(RenderNodeSelection {
            nodes: Vec::new(),
            source_row_indices: Vec::new(),
        });
    }
    let Some(path_level_names) = path_level_names_for_selected_nodes(selected_nodes) else {
        return Ok(RenderNodeSelection {
            nodes: selected_nodes.to_vec(),
            source_row_indices: repeated_source_indices(selected_nodes.len(), data.num_rows()),
        });
    };
    let has_all_path_columns = path_level_names
        .iter()
        .all(|name| data.column_by_name(name).is_some());
    if !has_all_path_columns {
        if data.num_columns() == 1 && data.column_by_name("_dummy").is_some() {
            return Ok(RenderNodeSelection {
                nodes: selected_nodes.to_vec(),
                source_row_indices: repeated_source_indices(selected_nodes.len(), data.num_rows()),
            });
        }
        let available = data
            .schema()
            .fields()
            .iter()
            .map(|field| field.name().clone())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(AvengerChartError::InvalidArgument(format!(
            "TreeRect mark data must include treemap path column(s) [{}]; available columns are [{}]",
            path_level_names.join(", "),
            available
        )));
    }

    let node_by_path_id = selected_nodes
        .iter()
        .map(|node| (node.node.path_id.as_str(), *node))
        .collect::<HashMap<_, _>>();
    let path_columns = path_level_names
        .iter()
        .map(|name| {
            data.column_by_name(name).ok_or_else(|| {
                AvengerChartError::InvalidArgument(format!(
                    "TreeRect mark data is missing treemap path column '{name}'"
                ))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut nodes = Vec::with_capacity(data.num_rows());
    let mut source_row_indices = Vec::with_capacity(data.num_rows());
    for row_index in 0..data.num_rows() {
        let mut path = Vec::with_capacity(path_level_names.len());
        for (name, column) in path_level_names.iter().zip(path_columns.iter()) {
            let value = ScalarValue::try_from_array(column, row_index)
                .map_err(AvengerChartError::DataFusionError)?;
            if scalar_is_null(&value) {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "TreeRect path column '{name}' contains null at row {row_index}"
                )));
            }
            path.push(crate::TreemapPathComponent {
                name: name.clone(),
                label: scalar_label(&value),
                value,
            });
        }
        let path_id = path_id_for_components(&path);
        if let Some(node) = node_by_path_id.get(path_id.as_str()) {
            nodes.push(*node);
            source_row_indices.push(row_index);
        } else if measurement.node(&path_id).is_none() {
            return Err(AvengerChartError::InvalidArgument(format!(
                "TreeRect mark data row {row_index} resolved to path '{path_id}', which does not exist in the treemap hierarchy"
            )));
        }
    }
    Ok(RenderNodeSelection {
        nodes,
        source_row_indices,
    })
}

fn gather_scalar_or_array<T: Sync + Clone>(
    values: ScalarOrArray<T>,
    len: usize,
    indices: Option<&Arc<Vec<usize>>>,
) -> ScalarOrArray<T> {
    match indices {
        Some(indices) => ScalarOrArray::new_array(values.as_vec(len, Some(indices))),
        None => values,
    }
}

fn path_level_names_for_selected_nodes(nodes: &[&VisibleTreemapNode]) -> Option<Vec<String>> {
    let deepest = nodes
        .iter()
        .map(|node| node.node.path.as_slice())
        .max_by_key(|path| path.len())?;
    Some(
        deepest
            .iter()
            .map(|component| component.name.clone())
            .collect(),
    )
}

fn validate_no_reserved_event_field_collisions(
    data: Option<&RecordBatch>,
) -> Result<(), AvengerChartError> {
    let Some(data) = data else {
        return Ok(());
    };
    if let Some(field) = data.schema().fields().iter().find(|field| {
        RESERVED_GENERATED_EVENT_FIELDS
            .iter()
            .any(|reserved| field.name() == reserved)
    }) {
        return Err(AvengerChartError::InvalidArgument(format!(
            "TreeRect data column '{}' conflicts with a reserved generated treemap event datum field",
            field.name()
        )));
    }
    Ok(())
}

fn tree_rect_event_datum_batch(
    nodes: &[&VisibleTreemapNode],
) -> Result<RecordBatch, AvengerChartError> {
    let surface_kinds = nodes
        .iter()
        .map(|node| hierarchy_surface_kind_for_node(node).to_string())
        .collect::<Vec<_>>();
    let rects = nodes.iter().map(|node| node.outer_rect).collect::<Vec<_>>();
    tree_node_event_datum_batch_with_surface_kinds(nodes, surface_kinds, rects)
}

fn tree_node_event_datum_batch(
    nodes: &[&VisibleTreemapNode],
    surface_kind: &str,
) -> Result<RecordBatch, AvengerChartError> {
    tree_node_event_datum_batch_with_surface_kinds(
        nodes,
        vec![surface_kind.to_string(); nodes.len()],
        nodes.iter().map(|node| node.outer_rect).collect(),
    )
}

fn tree_header_event_datum_batch(
    nodes: &[&VisibleTreemapNode],
) -> Result<RecordBatch, AvengerChartError> {
    tree_node_event_datum_batch_with_surface_kinds(
        nodes,
        vec![HIERARCHY_SURFACE_KIND_NODE_HEADER.to_string(); nodes.len()],
        nodes
            .iter()
            .map(|node| node.header_rect.unwrap_or(node.outer_rect))
            .collect(),
    )
}

fn tree_node_event_datum_batch_with_surface_kinds(
    nodes: &[&VisibleTreemapNode],
    surface_kinds: Vec<String>,
    rects: Vec<crate::TreemapRect>,
) -> Result<RecordBatch, AvengerChartError> {
    let len = nodes.len();
    debug_assert_eq!(surface_kinds.len(), len);
    debug_assert_eq!(rects.len(), len);
    let mut fields = Vec::new();
    let mut columns: Vec<ArrayRef> = Vec::new();

    for (name, values) in path_event_columns(nodes)? {
        let array = ScalarValue::iter_to_array(values.into_iter())
            .map_err(AvengerChartError::DataFusionError)?;
        fields.push(Field::new(name, array.data_type().clone(), true));
        columns.push(array);
    }

    fields.extend([
        Field::new(
            HIERARCHY_SURFACE_KIND_FIELD,
            datafusion::arrow::datatypes::DataType::Utf8,
            false,
        ),
        Field::new(
            HIERARCHY_PATH_ID_FIELD,
            datafusion::arrow::datatypes::DataType::Utf8,
            false,
        ),
        Field::new(
            HIERARCHY_PARENT_PATH_ID_FIELD,
            datafusion::arrow::datatypes::DataType::Utf8,
            true,
        ),
        Field::new(
            HIERARCHY_DEPTH_FIELD,
            datafusion::arrow::datatypes::DataType::Int64,
            false,
        ),
        Field::new(
            HIERARCHY_VIEW_DEPTH_FIELD,
            datafusion::arrow::datatypes::DataType::Int64,
            false,
        ),
        Field::new(
            HIERARCHY_DISPLAY_LEVELS_FIELD,
            datafusion::arrow::datatypes::DataType::Int64,
            false,
        ),
        Field::new(
            HIERARCHY_IS_DATA_LEAF_FIELD,
            datafusion::arrow::datatypes::DataType::Boolean,
            false,
        ),
        Field::new(
            HIERARCHY_IS_VISIBLE_LEAF_FIELD,
            datafusion::arrow::datatypes::DataType::Boolean,
            false,
        ),
        Field::new(
            HIERARCHY_HAS_HIDDEN_DESCENDANTS_FIELD,
            datafusion::arrow::datatypes::DataType::Boolean,
            false,
        ),
        Field::new(
            HIERARCHY_CAN_ZOOM_FIELD,
            datafusion::arrow::datatypes::DataType::Boolean,
            false,
        ),
        Field::new(
            HIERARCHY_VALUE_FIELD,
            datafusion::arrow::datatypes::DataType::Float64,
            false,
        ),
        Field::new(
            HIERARCHY_TITLE_FIELD,
            datafusion::arrow::datatypes::DataType::Utf8,
            false,
        ),
        Field::new(
            HIERARCHY_LEVEL_NAME_FIELD,
            datafusion::arrow::datatypes::DataType::Utf8,
            false,
        ),
        Field::new(
            TREEMAP_RECT_X_FIELD,
            datafusion::arrow::datatypes::DataType::Float64,
            false,
        ),
        Field::new(
            TREEMAP_RECT_Y_FIELD,
            datafusion::arrow::datatypes::DataType::Float64,
            false,
        ),
        Field::new(
            TREEMAP_RECT_WIDTH_FIELD,
            datafusion::arrow::datatypes::DataType::Float64,
            false,
        ),
        Field::new(
            TREEMAP_RECT_HEIGHT_FIELD,
            datafusion::arrow::datatypes::DataType::Float64,
            false,
        ),
    ]);
    columns.extend([
        Arc::new(StringArray::from(surface_kinds)) as ArrayRef,
        Arc::new(StringArray::from(
            nodes
                .iter()
                .map(|node| node.node.path_id.clone())
                .collect::<Vec<_>>(),
        )),
        Arc::new(StringArray::from(
            nodes
                .iter()
                .map(|node| node.node.parent_path_id.clone())
                .collect::<Vec<_>>(),
        )),
        Arc::new(Int64Array::from(
            nodes
                .iter()
                .map(|node| node.node.depth as i64)
                .collect::<Vec<_>>(),
        )),
        Arc::new(Int64Array::from(
            nodes
                .iter()
                .map(|node| node.view_depth as i64)
                .collect::<Vec<_>>(),
        )),
        Arc::new(Int64Array::from(
            nodes
                .iter()
                .map(|node| node.display_levels as i64)
                .collect::<Vec<_>>(),
        )),
        Arc::new(BooleanArray::from(
            nodes
                .iter()
                .map(|node| node.node.child_path_ids.is_empty())
                .collect::<Vec<_>>(),
        )),
        Arc::new(BooleanArray::from(
            nodes
                .iter()
                .map(|node| node.is_visible_leaf)
                .collect::<Vec<_>>(),
        )),
        Arc::new(BooleanArray::from(
            nodes
                .iter()
                .map(|node| node.has_hidden_descendants)
                .collect::<Vec<_>>(),
        )),
        Arc::new(BooleanArray::from(
            nodes.iter().map(|node| node.can_zoom).collect::<Vec<_>>(),
        )),
        Arc::new(Float64Array::from(
            nodes.iter().map(|node| node.node.value).collect::<Vec<_>>(),
        )),
        Arc::new(StringArray::from(
            nodes
                .iter()
                .map(|node| node.node.label.clone())
                .collect::<Vec<_>>(),
        )),
        Arc::new(StringArray::from(
            nodes
                .iter()
                .map(|node| {
                    node.node
                        .path
                        .last()
                        .map(|component| component.name.clone())
                        .unwrap_or_default()
                })
                .collect::<Vec<_>>(),
        )),
        Arc::new(Float64Array::from(
            rects.iter().map(|rect| rect.x as f64).collect::<Vec<_>>(),
        )),
        Arc::new(Float64Array::from(
            rects.iter().map(|rect| rect.y as f64).collect::<Vec<_>>(),
        )),
        Arc::new(Float64Array::from(
            rects
                .iter()
                .map(|rect| rect.width as f64)
                .collect::<Vec<_>>(),
        )),
        Arc::new(Float64Array::from(
            rects
                .iter()
                .map(|rect| rect.height as f64)
                .collect::<Vec<_>>(),
        )),
    ]);

    debug_assert!(columns.iter().all(|column| column.len() == len));
    RecordBatch::try_new(Arc::new(Schema::new(fields)), columns)
        .map_err(AvengerChartError::ArrowError)
}

fn path_event_columns(
    nodes: &[&VisibleTreemapNode],
) -> Result<Vec<(String, Vec<ScalarValue>)>, AvengerChartError> {
    let mut levels: Vec<(String, datafusion::arrow::datatypes::DataType)> = Vec::new();
    for node in nodes {
        for component in &node.node.path {
            if RESERVED_GENERATED_EVENT_FIELDS
                .iter()
                .any(|reserved| component.name == *reserved)
            {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Treemap path level '{}' conflicts with a reserved generated event datum field",
                    component.name
                )));
            }
            if !levels.iter().any(|(name, _)| name == &component.name) {
                levels.push((component.name.clone(), component.value.data_type()));
            }
        }
    }

    let mut columns = Vec::with_capacity(levels.len());
    for (name, data_type) in levels {
        let mut values = Vec::with_capacity(nodes.len());
        for node in nodes {
            let value = if let Some(component) = node
                .node
                .path
                .iter()
                .find(|component| component.name == name)
            {
                component.value.clone()
            } else {
                ScalarValue::try_new_null(&data_type).map_err(AvengerChartError::DataFusionError)?
            };
            values.push(value);
        }
        columns.push((name, values));
    }
    Ok(columns)
}

fn hierarchy_surface_kind_for_node(node: &VisibleTreemapNode) -> &'static str {
    if node.has_hidden_descendants {
        HIERARCHY_SURFACE_KIND_COLLAPSED_RECT
    } else if node.is_visible_leaf {
        HIERARCHY_SURFACE_KIND_LEAF_RECT
    } else {
        HIERARCHY_SURFACE_KIND_NODE_RECT
    }
}

fn source_row_indices_for_event_rows(
    data: Option<&RecordBatch>,
    nodes: &[&VisibleTreemapNode],
) -> Result<Vec<usize>, AvengerChartError> {
    let Some(data) = data else {
        return Ok(Vec::new());
    };
    if nodes.is_empty() || data.num_rows() == 0 {
        return Ok(Vec::new());
    }
    let Some(path_level_names) = path_level_names_for_selected_nodes(nodes) else {
        return Ok(repeated_source_indices(nodes.len(), data.num_rows()));
    };
    if !path_level_names
        .iter()
        .all(|name| data.column_by_name(name).is_some())
    {
        return Ok(repeated_source_indices(nodes.len(), data.num_rows()));
    }

    let path_columns = path_level_names
        .iter()
        .map(|name| {
            data.column_by_name(name).ok_or_else(|| {
                AvengerChartError::InvalidArgument(format!(
                    "TreeRect mark data is missing treemap path column '{name}'"
                ))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut row_paths = Vec::with_capacity(data.num_rows());
    for row_index in 0..data.num_rows() {
        let mut path = Vec::with_capacity(path_level_names.len());
        for (name, column) in path_level_names.iter().zip(path_columns.iter()) {
            let value = ScalarValue::try_from_array(column, row_index)
                .map_err(AvengerChartError::DataFusionError)?;
            if scalar_is_null(&value) {
                break;
            }
            path.push(crate::TreemapPathComponent {
                name: name.clone(),
                label: scalar_label(&value),
                value,
            });
        }
        row_paths.push(path);
    }

    let mut indices = Vec::with_capacity(nodes.len());
    for node in nodes {
        let index = row_paths
            .iter()
            .position(|path| path_has_prefix(path, &node.node.path))
            .unwrap_or(0);
        indices.push(index);
    }
    Ok(indices)
}

fn repeated_source_indices(node_count: usize, row_count: usize) -> Vec<usize> {
    if row_count == 0 {
        return Vec::new();
    }
    (0..node_count)
        .map(|index| index.min(row_count - 1))
        .collect()
}

fn path_has_prefix(
    path: &[crate::TreemapPathComponent],
    prefix: &[crate::TreemapPathComponent],
) -> bool {
    if prefix.len() > path.len() {
        return false;
    }
    path.iter()
        .zip(prefix.iter())
        .all(|(left, right)| left.name == right.name && left.label == right.label)
}

fn fit_tree_label(
    label: &str,
    limit: f32,
    available_height: f32,
    font: &str,
    font_size: f32,
    font_weight: &FontWeight,
    font_style: &FontStyle,
    fit: TreeLabelFit,
    min_width: f32,
    min_height: f32,
    text_measurement_service: Option<&dyn TextMeasurementService>,
) -> String {
    if label.is_empty()
        || limit < min_width.max(0.0)
        || available_height < min_height.max(0.0)
        || available_height < font_size.max(0.0) * 0.8
    {
        return String::new();
    }
    let text_engine = text_measurement_service.is_none().then(default_text_engine);
    let measure_width = |candidate: &str| {
        let config = TextMeasurementConfig {
            text: candidate,
            font,
            font_size,
            font_weight: *font_weight,
            font_style: *font_style,
            syntax_mode: avenger_text::types::TextSyntaxMode::Plain,
            params: avenger_text::empty_label_params(),
            number_locale: None,
            number_locale_specs: None,
            datetime_locale: None,
            datetime_timezone: None,
            datetime_locale_specs: None,
        };
        if let Some(service) = text_measurement_service {
            service.measure_text_bounds(&config).width
        } else {
            text_engine
                .as_ref()
                .expect("default text engine fallback")
                .measure_bounds_with_plain_fallback_or_approx(&config)
                .width
        }
    };
    match fit {
        TreeLabelFit::Hide => {
            let width = measure_width(label);
            if width <= limit {
                label.to_string()
            } else {
                String::new()
            }
        }
        TreeLabelFit::Ellipsis => truncate_text_to_limit_with(label, limit, |candidate| {
            Ok::<_, std::convert::Infallible>(measure_width(candidate))
        })
        .unwrap_or_else(|_| label.to_string()),
    }
}

fn tree_label_channel_defaults(channel: &str) -> Option<ScalarValue> {
    match channel {
        "text" => Some(ScalarValue::Utf8(Some(String::new()))),
        "color" => Some(ScalarValue::Utf8(Some("#ffffff".to_string()))),
        "opacity" => Some(ScalarValue::Float32(Some(1.0))),
        "font_size" => Some(ScalarValue::Float32(Some(11.0))),
        "align" => Some(ScalarValue::Utf8(Some("center".to_string()))),
        "baseline" => Some(ScalarValue::Utf8(Some("middle".to_string()))),
        "font" => Some(ScalarValue::Utf8(Some("sans-serif".to_string()))),
        "font_weight" => Some(ScalarValue::Float32(Some(300.0))),
        "font_style" => Some(ScalarValue::Utf8(Some("normal".to_string()))),
        _ => None,
    }
}

fn tree_header_channel_defaults(channel: &str) -> Option<ScalarValue> {
    match channel {
        "fill" => Some(ScalarValue::Utf8(Some("#4682b4".to_string()))),
        "stroke" => Some(ScalarValue::Utf8(Some("#ffffff".to_string()))),
        "stroke_width" => Some(ScalarValue::Float32(Some(1.0))),
        "opacity" => Some(ScalarValue::Float32(Some(1.0))),
        "text" => Some(ScalarValue::Utf8(Some(String::new()))),
        "text_color" => Some(ScalarValue::Utf8(Some("#ffffff".to_string()))),
        "font_size" => Some(ScalarValue::Float32(Some(14.0))),
        "font" => Some(ScalarValue::Utf8(Some("sans-serif".to_string()))),
        "font_weight" => Some(ScalarValue::Utf8(Some("bold".to_string()))),
        "font_style" => Some(ScalarValue::Utf8(Some("normal".to_string()))),
        _ => None,
    }
}

fn tree_rect_legend_renderer(
    state: &CompiledMarkState,
    channel: &str,
    scale: &ConfiguredScale,
) -> Option<LegendRendererSelection> {
    state
        .data
        .channels()
        .get(channel)
        .and_then(|channel| channel.get_legend_config())?;
    match channel {
        "fill" | "stroke" | "color" if is_continuous_scale(scale.scale_impl.as_ref()) => Some(
            LegendRendererSelection::BuiltIn(LegendRendererKind::Colorbar),
        ),
        "fill" | "stroke" | "color" | "opacity" | "stroke_width" => {
            Some(LegendRendererSelection::BuiltIn(LegendRendererKind::Rect))
        }
        "corner_radius" | "u" | "u2" | "v" | "v2" | "defined" | "order" => None,
        _ => Some(LegendRendererSelection::BuiltIn(LegendRendererKind::Rect)),
    }
}

fn tree_rect_channel_defaults(channel: &str) -> Option<ScalarValue> {
    match channel {
        "fill" => Some(ScalarValue::Utf8(Some("#4682b4".to_string()))),
        "stroke" => Some(ScalarValue::Utf8(Some("#000000".to_string()))),
        "stroke_width" => Some(ScalarValue::Float32(Some(1.0))),
        "corner_radius" => Some(ScalarValue::Float32(Some(0.0))),
        "opacity" => Some(ScalarValue::Float32(Some(1.0))),
        "u" | "v" => Some(ScalarValue::Float32(Some(0.0))),
        "u2" | "v2" => Some(ScalarValue::Float32(Some(1.0))),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use arrow::{
        array::{Array, BooleanArray, Float64Array, Int64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    };
    use avenger_chart::plot::Plot;
    use avenger_chart::prelude::{ChartEventBinding, ChartEventType, Param};
    use avenger_chart_core::{
        SceneGeometryHitPolicy, SceneGeometryQuery, SceneQueryDatumField, Selection,
        SelectionClauseUpdate, SelectionPredicateUpdate, SelectionSceneQuery, SelectionUpdate,
        event as ev,
    };
    use datafusion::{
        common::ScalarValue,
        functions_aggregate::expr_fn::sum,
        logical_expr::{col, lit},
    };

    fn source_batch() -> RecordBatch {
        RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("region", DataType::Utf8, false),
                Field::new("product", DataType::Utf8, false),
                Field::new("sales", DataType::Float64, false),
            ])),
            vec![
                Arc::new(StringArray::from(vec!["East", "East", "West"])),
                Arc::new(StringArray::from(vec!["A", "B", "A"])),
                Arc::new(Float64Array::from(vec![2.0, 3.0, 5.0])),
            ],
        )
        .unwrap()
    }

    fn find_tree_rects(marks: &[SceneMark]) -> Vec<&SceneRectMark> {
        let mut out = Vec::new();
        for mark in marks {
            match mark {
                SceneMark::Rect(rect)
                    if matches!(
                        rect.name.as_str(),
                        "tree_rect" | "regions" | "base" | "overlay" | "colored" | "segments"
                    ) =>
                {
                    out.push(rect);
                }
                SceneMark::Group(group) => out.extend(find_tree_rects(&group.marks)),
                _ => {}
            }
        }
        out
    }

    fn find_tree_labels(marks: &[SceneMark]) -> Vec<&SceneTextMark> {
        let mut out = Vec::new();
        for mark in marks {
            match mark {
                SceneMark::Text(text) if text.name == "tree_label" || text.name == "labels" => {
                    out.push(text.as_ref());
                }
                SceneMark::Group(group) => out.extend(find_tree_labels(&group.marks)),
                _ => {}
            }
        }
        out
    }

    fn find_tree_headers(marks: &[SceneMark]) -> (Vec<&SceneRectMark>, Vec<&SceneTextMark>) {
        let mut rects = Vec::new();
        let mut texts = Vec::new();
        for mark in marks {
            match mark {
                SceneMark::Rect(rect) if rect.name == "tree_header" || rect.name == "headers" => {
                    rects.push(rect);
                }
                SceneMark::Text(text)
                    if text.name == "tree_header_text" || text.name == "headers_text" =>
                {
                    texts.push(text.as_ref());
                }
                SceneMark::Group(group) => {
                    let (child_rects, child_texts) = find_tree_headers(&group.marks);
                    rects.extend(child_rects);
                    texts.extend(child_texts);
                }
                _ => {}
            }
        }
        (rects, texts)
    }

    #[tokio::test]
    async fn tree_rect_renders_visible_leaf_cells_from_measurement() {
        let ctx = SessionContext::new();
        let df = ctx.read_batch(source_batch()).unwrap();
        let plot = Plot::with_coord(
            Treemap::new()
                .path_columns(["region", "product"])
                .value(sum(col("sales"))),
        )
        .data(df)
        .plot_size(200.0, 100.0)
        .mark(TreeRect::new().fill("#ff0000").stroke_width(0.0));

        let evaluated = plot
            .compile(&ctx)
            .await
            .unwrap()
            .evaluate(&ctx, None)
            .await
            .unwrap();
        let rects = find_tree_rects(&evaluated.scene_graph.marks);
        assert_eq!(rects.len(), 1);
        let rect = rects[0];
        assert_eq!(rect.len, 3);
        assert_eq!(rect.x.as_vec(3, None), vec![0.0, 0.0, 100.0]);
        assert_eq!(
            rect.width.as_ref().unwrap().as_vec(3, None),
            vec![100.0, 100.0, 100.0]
        );
    }

    #[tokio::test]
    async fn tree_rect_depth_mode_renders_internal_nodes() {
        let ctx = SessionContext::new();
        let df = ctx.read_batch(source_batch()).unwrap();
        let plot = Plot::with_coord(
            Treemap::new()
                .path_columns(["region", "product"])
                .value(sum(col("sales"))),
        )
        .data(df)
        .plot_size(200.0, 100.0)
        .mark(TreeRect::new().id("regions").depth(1).stroke_width(0.0));

        let evaluated = plot
            .compile(&ctx)
            .await
            .unwrap()
            .evaluate(&ctx, None)
            .await
            .unwrap();
        let rects = find_tree_rects(&evaluated.scene_graph.marks);
        assert_eq!(rects.len(), 1);
        let rect = rects[0];
        assert_eq!(rect.name, "regions");
        assert_eq!(rect.len, 2);
        assert_eq!(rect.x.as_vec(2, None), vec![0.0, 100.0]);
        assert_eq!(
            rect.width.as_ref().unwrap().as_vec(2, None),
            vec![100.0, 100.0]
        );
    }

    #[tokio::test]
    async fn tree_rect_all_visible_mode_renders_internal_and_leaf_nodes() {
        let ctx = SessionContext::new();
        let df = ctx.read_batch(source_batch()).unwrap();
        let plot = Plot::with_coord(
            Treemap::new()
                .path_columns(["region", "product"])
                .value(sum(col("sales"))),
        )
        .data(df)
        .plot_size(200.0, 100.0)
        .mark(TreeRect::new().all_visible().stroke_width(0.0));

        let evaluated = plot
            .compile(&ctx)
            .await
            .unwrap()
            .evaluate(&ctx, None)
            .await
            .unwrap();
        let rects = find_tree_rects(&evaluated.scene_graph.marks);
        assert_eq!(rects.len(), 1);
        assert_eq!(rects[0].len, 5);
    }

    #[tokio::test]
    async fn tree_label_defaults_to_visible_leaf_node_labels() {
        let ctx = SessionContext::new();
        let df = ctx.read_batch(source_batch()).unwrap();
        let plot = Plot::with_coord(
            Treemap::new()
                .path_columns(["region", "product"])
                .value(sum(col("sales"))),
        )
        .data(df)
        .plot_size(200.0, 100.0)
        .mark(TreeRect::new().stroke_width(0.0))
        .mark(TreeLabel::new());

        let evaluated = plot
            .compile(&ctx)
            .await
            .unwrap()
            .evaluate(&ctx, None)
            .await
            .unwrap();
        let labels = find_tree_labels(&evaluated.scene_graph.marks);
        assert_eq!(labels.len(), 1);
        assert_eq!(labels[0].len, 3);
        assert_eq!(
            labels[0].text.as_vec(3, None),
            vec!["A".to_string(), "B".to_string(), "A".to_string()]
        );
        assert_eq!(labels[0].font_size.as_vec(3, None), vec![11.0; 3]);
        assert_eq!(
            labels[0].font_weight.as_vec(3, None),
            vec![FontWeight::Number(300.0); 3]
        );

        let label_rows = evaluated
            .event_datums
            .rows
            .iter()
            .find(|rows| {
                let Some(column) = rows.rows.column_by_name(HIERARCHY_SURFACE_KIND_FIELD) else {
                    return false;
                };
                let values = column
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .expect("surface kind string");
                values.len() == 3 && values.value(0) == HIERARCHY_SURFACE_KIND_NODE_LABEL
            })
            .expect("label event datum rows");
        let path_ids = label_rows
            .rows
            .column_by_name(HIERARCHY_PATH_ID_FIELD)
            .expect("path ids")
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("path ids string");
        assert_eq!(path_ids.value(0), "region=East/product=A");
    }

    #[tokio::test]
    async fn tree_label_text_channel_joins_mark_data_by_path() {
        let ctx = SessionContext::new();
        let df = ctx.read_batch(source_batch()).unwrap();
        let plot = Plot::with_coord(
            Treemap::new()
                .path_columns(["region", "product"])
                .value(sum(col("sales"))),
        )
        .data(df)
        .plot_size(200.0, 100.0)
        .mark(TreeLabel::new().id("labels").text(col("region")));

        let evaluated = plot
            .compile(&ctx)
            .await
            .unwrap()
            .evaluate(&ctx, None)
            .await
            .unwrap();
        let labels = find_tree_labels(&evaluated.scene_graph.marks);
        assert_eq!(labels.len(), 1);
        assert_eq!(
            labels[0].text.as_vec(3, None),
            vec!["East".to_string(), "East".to_string(), "West".to_string()]
        );
    }

    #[tokio::test]
    async fn tree_header_renders_header_rects_and_matching_event_rows() {
        let ctx = SessionContext::new();
        let df = ctx.read_batch(source_batch()).unwrap();
        let plot = Plot::with_coord(
            Treemap::new()
                .path_columns(["region", "product"])
                .value(sum(col("sales")))
                .display_levels(2)
                .header_bars(crate::TreemapHeaderBars::enabled().height_px(18.0)),
        )
        .data(df)
        .plot_size(200.0, 100.0)
        .mark(TreeHeader::new().fill(col("region")));

        let evaluated = plot
            .compile(&ctx)
            .await
            .unwrap()
            .evaluate(&ctx, None)
            .await
            .unwrap();
        let (header_rects, header_texts) = find_tree_headers(&evaluated.scene_graph.marks);
        assert_eq!(header_rects.len(), 1);
        assert_eq!(header_texts.len(), 1);
        assert_eq!(header_rects[0].len, 2);
        assert_eq!(
            header_rects[0].height.as_ref().unwrap().as_vec(2, None),
            vec![18.0, 18.0]
        );
        assert_eq!(
            header_texts[0].text.as_vec(2, None),
            vec!["East".to_string(), "West".to_string()]
        );
        assert_eq!(header_texts[0].font_size.as_vec(2, None), vec![14.0; 2]);
        assert_eq!(
            header_texts[0].font_weight.as_vec(2, None),
            vec![FontWeight::Name(FontWeightNameSpec::Bold); 2]
        );

        let header_rows = evaluated
            .event_datums
            .rows
            .iter()
            .filter(|rows| {
                let Some(column) = rows.rows.column_by_name(HIERARCHY_SURFACE_KIND_FIELD) else {
                    return false;
                };
                let values = column
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .expect("surface kind string");
                values.len() == 2 && values.value(0) == HIERARCHY_SURFACE_KIND_NODE_HEADER
            })
            .collect::<Vec<_>>();
        assert_eq!(header_rows.len(), 2);
        for rows in header_rows {
            assert_eq!(rows.rows.num_rows(), 2);
            let path_ids = rows
                .rows
                .column_by_name(HIERARCHY_PATH_ID_FIELD)
                .expect("path ids")
                .as_any()
                .downcast_ref::<StringArray>()
                .expect("path ids string");
            assert_eq!(path_ids.value(0), "region=East");
            let titles = rows
                .rows
                .column_by_name(HIERARCHY_TITLE_FIELD)
                .expect("title")
                .as_any()
                .downcast_ref::<StringArray>()
                .expect("title string");
            assert_eq!(titles.value(0), "East");
            let level_names = rows
                .rows
                .column_by_name(HIERARCHY_LEVEL_NAME_FIELD)
                .expect("level name")
                .as_any()
                .downcast_ref::<StringArray>()
                .expect("level name string");
            assert_eq!(level_names.value(0), "region");
            let zoomable = rows
                .rows
                .column_by_name(HIERARCHY_CAN_ZOOM_FIELD)
                .expect("can zoom")
                .as_any()
                .downcast_ref::<BooleanArray>()
                .expect("can zoom bool");
            assert!(zoomable.value(0));
            let depths = rows
                .rows
                .column_by_name(HIERARCHY_DEPTH_FIELD)
                .expect("depth")
                .as_any()
                .downcast_ref::<Int64Array>()
                .expect("depth i64");
            assert_eq!(depths.value(0), 1);
            let view_depths = rows
                .rows
                .column_by_name(HIERARCHY_VIEW_DEPTH_FIELD)
                .expect("view depth")
                .as_any()
                .downcast_ref::<Int64Array>()
                .expect("view depth i64");
            assert_eq!(view_depths.value(0), 0);
            let display_levels = rows
                .rows
                .column_by_name(HIERARCHY_DISPLAY_LEVELS_FIELD)
                .expect("display levels")
                .as_any()
                .downcast_ref::<Int64Array>()
                .expect("display levels i64");
            assert_eq!(display_levels.value(0), 2);
            let header_heights = rows
                .rows
                .column_by_name(TREEMAP_RECT_HEIGHT_FIELD)
                .expect("header height")
                .as_any()
                .downcast_ref::<Float64Array>()
                .expect("height f64");
            assert_eq!(header_heights.value(0), 18.0);
        }
    }

    #[test]
    fn tree_label_fit_uses_measured_ellipsis_or_hides() {
        let font_weight = FontWeight::Name(FontWeightNameSpec::Normal);
        let font_style = FontStyle::Normal;
        let label = fit_tree_label(
            "A very long product label",
            36.0,
            18.0,
            "sans-serif",
            12.0,
            &font_weight,
            &font_style,
            TreeLabelFit::Ellipsis,
            4.0,
            4.0,
            None,
        );
        assert!(!label.is_empty());
        assert!(label.len() < "A very long product label".len());

        let hidden = fit_tree_label(
            "A",
            36.0,
            4.0,
            "sans-serif",
            12.0,
            &font_weight,
            &font_style,
            TreeLabelFit::Ellipsis,
            4.0,
            4.0,
            None,
        );
        assert_eq!(hidden, "");
    }

    #[tokio::test]
    async fn tree_rect_maps_local_unit_geometry_into_solved_cells() {
        let ctx = SessionContext::new();
        let df = ctx.read_batch(source_batch()).unwrap();
        let plot = Plot::with_coord(
            Treemap::new()
                .path_columns(["region", "product"])
                .value(sum(col("sales"))),
        )
        .data(df)
        .plot_size(200.0, 100.0)
        .mark(
            TreeRect::new()
                .u(0.25)
                .u2(0.75)
                .v(0.5)
                .v2(1.0)
                .stroke_width(0.0),
        );

        let evaluated = plot
            .compile(&ctx)
            .await
            .unwrap()
            .evaluate(&ctx, None)
            .await
            .unwrap();
        let rects = find_tree_rects(&evaluated.scene_graph.marks);
        assert_eq!(rects.len(), 1);
        let rect = rects[0];
        assert_eq!(rect.x.as_vec(3, None), vec![25.0, 25.0, 125.0]);
        assert_eq!(
            rect.width.as_ref().unwrap().as_vec(3, None),
            vec![50.0, 50.0, 50.0]
        );
        assert_eq!(rect.y.as_vec(3, None), vec![20.0, 70.0, 50.0]);
        assert_eq!(
            rect.height.as_ref().unwrap().as_vec(3, None),
            vec![20.0, 30.0, 50.0]
        );
    }

    #[tokio::test]
    async fn tree_rect_layers_reuse_coordinate_layout_geometry() {
        let ctx = SessionContext::new();
        let df = ctx.read_batch(source_batch()).unwrap();
        let plot = Plot::with_coord(
            Treemap::new()
                .path_columns(["region", "product"])
                .value(sum(col("sales"))),
        )
        .data(df)
        .plot_size(200.0, 100.0)
        .mark(TreeRect::new().id("base").fill("#d8dde3").stroke_width(0.0))
        .mark(
            TreeRect::new()
                .id("overlay")
                .fill("#e15759")
                .opacity(0.5)
                .stroke_width(0.0),
        );

        let evaluated = plot
            .compile(&ctx)
            .await
            .unwrap()
            .evaluate(&ctx, None)
            .await
            .unwrap();
        let rects = find_tree_rects(&evaluated.scene_graph.marks);
        assert_eq!(rects.len(), 2);
        let base = rects
            .iter()
            .find(|rect| rect.name == "base")
            .expect("base layer");
        let overlay = rects
            .iter()
            .find(|rect| rect.name == "overlay")
            .expect("overlay layer");
        assert_eq!(base.len, overlay.len);
        assert_eq!(
            base.x.as_vec(base.len as usize, None),
            overlay.x.as_vec(overlay.len as usize, None)
        );
        assert_eq!(
            base.y.as_vec(base.len as usize, None),
            overlay.y.as_vec(overlay.len as usize, None)
        );
        assert_eq!(
            base.width.as_ref().unwrap().as_vec(base.len as usize, None),
            overlay
                .width
                .as_ref()
                .unwrap()
                .as_vec(overlay.len as usize, None)
        );
        assert_eq!(
            base.height
                .as_ref()
                .unwrap()
                .as_vec(base.len as usize, None),
            overlay
                .height
                .as_ref()
                .unwrap()
                .as_vec(overlay.len as usize, None)
        );
    }

    fn color_source_batch() -> RecordBatch {
        RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("region", DataType::Utf8, false),
                Field::new("product", DataType::Utf8, false),
                Field::new("sales", DataType::Float64, false),
                Field::new("color", DataType::Utf8, false),
            ])),
            vec![
                Arc::new(StringArray::from(vec!["East", "East", "West"])),
                Arc::new(StringArray::from(vec!["A", "B", "A"])),
                Arc::new(Float64Array::from(vec![2.0, 3.0, 5.0])),
                Arc::new(StringArray::from(vec!["#ff0000", "#0000ff", "#00ff00"])),
            ],
        )
        .unwrap()
    }

    #[tokio::test]
    async fn tree_rect_uses_mark_data_style_columns_for_solved_cells() {
        let ctx = SessionContext::new();
        let df = ctx.read_batch(color_source_batch()).unwrap();
        let plot = Plot::with_coord(
            Treemap::new()
                .path_columns(["region", "product"])
                .value(sum(col("sales"))),
        )
        .data(df)
        .plot_size(200.0, 100.0)
        .mark(
            TreeRect::new()
                .id("colored")
                .fill(ChannelValue::from(col("color")).no_scale())
                .stroke_width(0.0),
        );

        let evaluated = plot
            .compile(&ctx)
            .await
            .unwrap()
            .evaluate(&ctx, None)
            .await
            .unwrap();
        let rects = find_tree_rects(&evaluated.scene_graph.marks);
        assert_eq!(rects.len(), 1);
        let rect = rects[0];
        assert_eq!(rect.len, 3);
        let fill = rect.fill.as_vec(3, None);
        assert_eq!(fill[0].color_or_transparent(), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(fill[1].color_or_transparent(), [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(fill[2].color_or_transparent(), [0.0, 1.0, 0.0, 1.0]);
    }

    #[tokio::test]
    async fn tree_rect_explicit_filtered_overlay_reuses_coordinate_layout() {
        let ctx = SessionContext::new();
        let base_df = ctx.read_batch(color_source_batch()).unwrap();
        let overlay_df = ctx
            .read_batch(color_source_batch())
            .unwrap()
            .filter(col("region").eq(lit("West")))
            .unwrap();
        let plot = Plot::with_coord(
            Treemap::new()
                .path_columns(["region", "product"])
                .value(sum(col("sales"))),
        )
        .data(base_df)
        .plot_size(200.0, 100.0)
        .mark(TreeRect::new().id("base").fill("#d8dde3").stroke_width(0.0))
        .mark(
            TreeRect::new()
                .id("overlay")
                .data(overlay_df)
                .fill(ChannelValue::from(col("color")).no_scale())
                .stroke_width(0.0),
        );

        let evaluated = plot
            .compile(&ctx)
            .await
            .unwrap()
            .evaluate(&ctx, None)
            .await
            .unwrap();
        let rects = find_tree_rects(&evaluated.scene_graph.marks);
        let base = rects
            .iter()
            .find(|rect| rect.name == "base")
            .expect("base layer");
        let overlay = rects
            .iter()
            .find(|rect| rect.name == "overlay")
            .expect("overlay layer");
        assert_eq!(base.len, 3);
        assert_eq!(overlay.len, 1);
        assert_eq!(overlay.x.as_vec(1, None), vec![100.0]);
        assert_eq!(overlay.y.as_vec(1, None), vec![0.0]);
        assert_eq!(overlay.width.as_ref().unwrap().as_vec(1, None), vec![100.0]);
        assert_eq!(
            overlay.height.as_ref().unwrap().as_vec(1, None),
            vec![100.0]
        );
    }

    fn stacked_segment_batch() -> RecordBatch {
        RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("region", DataType::Utf8, false),
                Field::new("product", DataType::Utf8, false),
                Field::new("segment", DataType::Utf8, false),
                Field::new("sales", DataType::Float64, false),
                Field::new("v0", DataType::Float64, false),
                Field::new("v1", DataType::Float64, false),
            ])),
            vec![
                Arc::new(StringArray::from(vec!["East", "East", "West"])),
                Arc::new(StringArray::from(vec!["A", "A", "A"])),
                Arc::new(StringArray::from(vec!["Small", "Large", "Small"])),
                Arc::new(Float64Array::from(vec![2.0, 3.0, 5.0])),
                Arc::new(Float64Array::from(vec![0.0, 0.4, 0.0])),
                Arc::new(Float64Array::from(vec![0.4, 1.0, 1.0])),
            ],
        )
        .unwrap()
    }

    #[tokio::test]
    async fn tree_rect_uses_data_driven_local_geometry_for_segments() {
        let ctx = SessionContext::new();
        let df = ctx.read_batch(stacked_segment_batch()).unwrap();
        let plot = Plot::with_coord(
            Treemap::new()
                .path_columns(["region", "product"])
                .value(sum(col("sales"))),
        )
        .data(df)
        .plot_size(200.0, 100.0)
        .mark(
            TreeRect::new()
                .id("segments")
                .fill(col("segment"))
                .v(ChannelValue::from(col("v0")).no_scale())
                .v2(ChannelValue::from(col("v1")).no_scale())
                .stroke_width(0.0),
        );

        let evaluated = plot
            .compile(&ctx)
            .await
            .unwrap()
            .evaluate(&ctx, None)
            .await
            .unwrap();
        let rects = find_tree_rects(&evaluated.scene_graph.marks);
        assert_eq!(rects.len(), 1);
        let rect = rects[0];
        assert_eq!(rect.len, 3);
        assert_eq!(rect.x.as_vec(3, None), vec![0.0, 0.0, 100.0]);
        assert_eq!(rect.width.as_ref().unwrap().as_vec(3, None), vec![100.0; 3]);
        assert_eq!(rect.y.as_vec(3, None), vec![0.0, 40.0, 0.0]);
        let heights = rect.height.as_ref().unwrap().as_vec(3, None);
        for (actual, expected) in heights.iter().zip([40.0, 60.0, 100.0]) {
            assert!(
                (actual - expected).abs() < 1e-4,
                "expected height {expected}, got {actual}"
            );
        }
    }

    #[tokio::test]
    async fn tree_rect_generated_event_rows_include_path_hierarchy_and_geometry_metadata() {
        let ctx = SessionContext::new();
        let df = ctx.read_batch(source_batch()).unwrap();
        let plot = Plot::with_coord(
            Treemap::new()
                .path_columns(["region", "product"])
                .value(sum(col("sales"))),
        )
        .data(df)
        .plot_size(200.0, 100.0)
        .mark(TreeRect::new().stroke_width(0.0));

        let evaluated = plot
            .compile(&ctx)
            .await
            .unwrap()
            .evaluate(&ctx, None)
            .await
            .unwrap();
        assert_eq!(evaluated.event_datums.rows.len(), 1);
        let rows = &evaluated.event_datums.rows[0].rows;
        assert_eq!(rows.num_rows(), 3);

        let regions = rows
            .column_by_name("region")
            .expect("region event column")
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("region string");
        assert_eq!(regions.value(0), "East");
        assert_eq!(regions.value(2), "West");

        let products = rows
            .column_by_name("product")
            .expect("product event column")
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("product string");
        assert_eq!(products.value(0), "A");
        assert_eq!(products.value(1), "B");

        let path_ids = rows
            .column_by_name(HIERARCHY_PATH_ID_FIELD)
            .expect("path id")
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("path id string");
        assert_eq!(path_ids.value(0), "region=East/product=A");
        assert_eq!(path_ids.value(2), "region=West/product=A");

        let surface_kinds = rows
            .column_by_name(HIERARCHY_SURFACE_KIND_FIELD)
            .expect("surface kind")
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("surface kind string");
        assert_eq!(surface_kinds.value(0), HIERARCHY_SURFACE_KIND_LEAF_RECT);

        let depths = rows
            .column_by_name(HIERARCHY_DEPTH_FIELD)
            .expect("depth")
            .as_any()
            .downcast_ref::<Int64Array>()
            .expect("depth i64");
        assert_eq!(depths.value(0), 2);

        let visible_leaf = rows
            .column_by_name(HIERARCHY_IS_VISIBLE_LEAF_FIELD)
            .expect("visible leaf")
            .as_any()
            .downcast_ref::<BooleanArray>()
            .expect("visible leaf bool");
        assert!(visible_leaf.value(0));

        let values = rows
            .column_by_name(HIERARCHY_VALUE_FIELD)
            .expect("hierarchy value")
            .as_any()
            .downcast_ref::<Float64Array>()
            .expect("value f64");
        assert_eq!(values.value(0), 2.0);
        assert_eq!(values.value(2), 5.0);

        let rect_widths = rows
            .column_by_name(TREEMAP_RECT_WIDTH_FIELD)
            .expect("rect width")
            .as_any()
            .downcast_ref::<Float64Array>()
            .expect("rect width f64");
        assert_eq!(rect_widths.value(0), 100.0);
        assert_eq!(rect_widths.value(2), 100.0);
    }

    #[tokio::test]
    async fn tree_rect_generated_event_rows_merge_with_source_event_rows_for_overlay_marks() {
        let ctx = SessionContext::new();
        let base_df = ctx.read_batch(color_source_batch()).unwrap();
        let overlay_df = ctx
            .read_batch(color_source_batch())
            .unwrap()
            .filter(col("region").eq(lit("West")))
            .unwrap();
        let plot = Plot::with_coord(
            Treemap::new()
                .path_columns(["region", "product"])
                .value(sum(col("sales"))),
        )
        .data(base_df)
        .plot_size(200.0, 100.0)
        .mark(TreeRect::new().id("base").fill("#d8dde3").stroke_width(0.0))
        .mark(
            TreeRect::new()
                .id("overlay")
                .data(overlay_df)
                .fill(ChannelValue::from(col("color")).no_scale())
                .stroke_width(0.0),
        )
        .event_binding(
            ChartEventBinding::on(ChartEventType::Click)
                .filter(ev::datum("color").is_not_null())
                .filter(crate::event::hierarchy_path_id().is_not_null()),
        );

        let evaluated = plot
            .compile(&ctx)
            .await
            .unwrap()
            .evaluate(&ctx, None)
            .await
            .unwrap();
        assert_eq!(evaluated.event_datums.rows.len(), 2);
        let overlay_rows = evaluated
            .event_datums
            .rows
            .iter()
            .find(|rows| rows.rows.num_rows() == 1)
            .expect("overlay event rows");

        let colors = overlay_rows
            .rows
            .column_by_name("color")
            .expect("source color")
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("color string");
        assert_eq!(colors.value(0), "#00ff00");

        let path_ids = overlay_rows
            .rows
            .column_by_name(HIERARCHY_PATH_ID_FIELD)
            .expect("path id")
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("path id string");
        assert_eq!(path_ids.value(0), "region=West/product=A");
    }

    #[tokio::test]
    async fn tree_rect_click_selection_can_use_generated_leaf_datums() {
        let ctx = SessionContext::new();
        let df = ctx.read_batch(source_batch()).unwrap();
        let plot = Plot::with_coord(
            Treemap::new()
                .path_columns(["region", "product"])
                .value(sum(col("sales"))),
        )
        .data(df)
        .plot_size(200.0, 100.0)
        .add_selection(Selection::new("picked").empty_selects_nothing())
        .mark(TreeRect::new().id("cells").stroke_width(0.0))
        .event_binding(
            ChartEventBinding::on(ChartEventType::Click)
                .filter(
                    crate::event::hierarchy_surface_kind()
                        .eq(lit(HIERARCHY_SURFACE_KIND_LEAF_RECT)),
                )
                .set_selection(
                    "picked",
                    SelectionUpdate::replace_clause(
                        SelectionClauseUpdate::equality(lit("active"))
                            .facet_scope(CoordinationScope::Shared)
                            .dimension_named("region", col("region"), ev::datum("region"))
                            .dimension_named("product", col("product"), ev::datum("product"))
                            .build(),
                    ),
                )
                .exact(),
        );

        let compiled = plot.compile(&ctx).await.unwrap();
        let binding = compiled.event_bindings().first().expect("event binding");
        let SelectionUpdate::ReplaceAllClauses { clauses } =
            &binding.selection_assignments[0].update
        else {
            panic!("expected replace-clause selection update");
        };
        assert_eq!(clauses[0].facet_scope, CoordinationScope::Shared);
        let SelectionPredicateUpdate::Equality { dimensions } = &clauses[0].predicate else {
            panic!("expected equality dimensions");
        };
        assert_eq!(
            dimensions
                .iter()
                .map(|dimension| dimension.id.as_str())
                .collect::<Vec<_>>(),
            vec!["region", "product"]
        );

        let evaluated = compiled.evaluate(&ctx, None).await.unwrap();
        let rows = &evaluated.event_datums.rows[0].rows;
        assert!(rows.column_by_name("region").is_some());
        assert!(rows.column_by_name("product").is_some());
        assert!(rows.column_by_name(HIERARCHY_PATH_ID_FIELD).is_some());
        assert!(rows.column_by_name(TREEMAP_RECT_X_FIELD).is_some());
    }

    #[tokio::test]
    async fn tree_rect_scene_query_selection_can_target_leaf_rect_datums() {
        let ctx = SessionContext::new();
        let df = ctx.read_batch(source_batch()).unwrap();
        let compiled = Plot::with_coord(
            Treemap::new()
                .path_columns(["region", "product"])
                .value(sum(col("sales"))),
        )
        .data(df)
        .plot_size(200.0, 100.0)
        .add_selection(Selection::new("picked").empty_selects_nothing())
        .mark(TreeRect::new().id("cells").stroke_width(0.0))
        .event_binding(
            ChartEventBinding::on(ChartEventType::Click).set_selection(
                "picked",
                SelectionUpdate::replace_all_from_scene_query(
                    SelectionSceneQuery::new(
                        SceneGeometryQuery::rect(lit(0.0), lit(0.0), lit(200.0), lit(100.0))
                            .hit_policy(SceneGeometryHitPolicy::GeometryIntersects)
                            .mark("cells")
                            .datum_fields([
                                SceneQueryDatumField::new("region"),
                                SceneQueryDatumField::new("product"),
                                SceneQueryDatumField::new("path_id")
                                    .datum(HIERARCHY_PATH_ID_FIELD)
                                    .field_expr(crate::event::hierarchy_path_id()),
                            ])
                            .unique_by([HIERARCHY_PATH_ID_FIELD]),
                    )
                    .sharing(CoordinationScope::Shared),
                ),
            ),
        )
        .compile(&ctx)
        .await
        .unwrap();

        let binding = compiled.event_bindings().first().expect("event binding");
        let SelectionUpdate::ReplaceAllFromSceneQuery { query } =
            &binding.selection_assignments[0].update
        else {
            panic!("expected scene query selection update");
        };
        assert_eq!(
            query.query.target.resolved_mark_paths(),
            Some(&[vec![0usize]][..])
        );
        assert_eq!(query.query.datum_fields.len(), 3);
        assert_eq!(query.query.unique_by, vec![HIERARCHY_PATH_ID_FIELD]);

        let evaluated = compiled.evaluate(&ctx, None).await.unwrap();
        let rows = &evaluated.event_datums.rows[0].rows;
        assert_eq!(rows.num_rows(), 3);
        assert!(rows.column_by_name(HIERARCHY_PATH_ID_FIELD).is_some());
        assert!(rows.column_by_name(TREEMAP_RECT_WIDTH_FIELD).is_some());
    }

    #[tokio::test]
    async fn tree_rect_collapsed_node_datums_can_drive_zoom_param() {
        let ctx = SessionContext::new();
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("region", DataType::Utf8, false),
                Field::new("product", DataType::Utf8, false),
                Field::new("sku", DataType::Utf8, false),
                Field::new("sales", DataType::Float64, false),
            ])),
            vec![
                Arc::new(StringArray::from(vec!["East", "East", "West"])),
                Arc::new(StringArray::from(vec!["A", "B", "C"])),
                Arc::new(StringArray::from(vec!["A1", "B1", "C1"])),
                Arc::new(Float64Array::from(vec![2.0, 3.0, 5.0])),
            ],
        )
        .unwrap();
        let df = ctx.read_batch(batch).unwrap();
        let root = Param::new("treemap_root", ScalarValue::Utf8(None));
        let plot = Plot::with_coord(
            Treemap::new()
                .path_columns(["region", "product", "sku"])
                .value(sum(col("sales")))
                .root_path_param(root.name.clone())
                .display_levels(1),
        )
        .data(df)
        .plot_size(200.0, 100.0)
        .add_param(root.clone())
        .mark(TreeRect::new().id("collapsed").stroke_width(0.0))
        .event_binding(
            ChartEventBinding::on(ChartEventType::Click)
                .filter(crate::event::hierarchy_can_zoom().eq(lit(true)))
                .set_param(&root, crate::event::hierarchy_path_id())
                .exact(),
        );

        let compiled = plot.compile(&ctx).await.unwrap();
        let binding = compiled.event_bindings().first().expect("event binding");
        assert_eq!(binding.assignments[0].param_name, root.name);

        let evaluated = compiled.evaluate(&ctx, None).await.unwrap();
        let rows = &evaluated.event_datums.rows[0].rows;
        let surface_kinds = rows
            .column_by_name(HIERARCHY_SURFACE_KIND_FIELD)
            .expect("surface kind")
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("surface kind string");
        assert_eq!(
            surface_kinds.value(0),
            HIERARCHY_SURFACE_KIND_COLLAPSED_RECT
        );

        let zoomable = rows
            .column_by_name(HIERARCHY_CAN_ZOOM_FIELD)
            .expect("can zoom")
            .as_any()
            .downcast_ref::<BooleanArray>()
            .expect("can zoom bool");
        assert!(zoomable.value(0));
    }

    #[tokio::test]
    async fn tree_rect_rejects_reserved_generated_event_metadata_columns() {
        let ctx = SessionContext::new();
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("region", DataType::Utf8, false),
                Field::new("product", DataType::Utf8, false),
                Field::new("sales", DataType::Float64, false),
                Field::new(HIERARCHY_PATH_ID_FIELD, DataType::Utf8, false),
            ])),
            vec![
                Arc::new(StringArray::from(vec!["East"])),
                Arc::new(StringArray::from(vec!["A"])),
                Arc::new(Float64Array::from(vec![2.0])),
                Arc::new(StringArray::from(vec!["user-owned"])),
            ],
        )
        .unwrap();
        let df = ctx.read_batch(batch).unwrap();
        let plot = Plot::with_coord(
            Treemap::new()
                .path_columns(["region", "product"])
                .value(sum(col("sales"))),
        )
        .data(df)
        .plot_size(200.0, 100.0)
        .mark(TreeRect::new().stroke_width(0.0));

        let err = match plot.compile(&ctx).await.unwrap().evaluate(&ctx, None).await {
            Ok(_) => panic!("expected reserved event metadata collision"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("reserved generated treemap"));
    }
}
