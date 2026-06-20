use std::{any::Any, collections::HashMap, marker::PhantomData, sync::Arc};

use async_trait::async_trait;
use avenger_chart_core::{
    AvengerChartError, ChannelDescriptor, ChannelValue, ColorChannelConfig, CompiledDataContext,
    CompiledMark, CompiledMarkCore, CompiledMarkState, CoordinateSystemCore,
    CoordinateSystemTransformCore, CoordinationScope, DataContext, DataTransform,
    DataTransformCompileContext, DefaultLogicalExprNodeExt, FacetDataScope, IntoExpr, IntoPlotMark,
    Mark, MarkDataMode, MarkRuntimeContext, MarkState, OpacityChannelConfig, PlotMark, StoreData,
    StrokeWidthChannelConfig, apply_opacity_to_color_channel, coerce_color_channel_with_renderer,
    coerce_numeric_channel_with_renderer, coerce_opacity_channel_with_renderer,
    define_common_mark_channels, impl_mark_trait_common,
};
use avenger_common::value::ScalarOrArray;
use avenger_scenegraph::marks::{mark::SceneMark, rect::SceneRectMark};
use datafusion::{
    arrow::record_batch::RecordBatch, common::ScalarValue, dataframe::DataFrame,
    prelude::SessionContext,
};
use serde::{Deserialize, Serialize};

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
                data_mode: MarkDataMode::Inherit,
                facet_data_scope: FacetDataScope::FILTERED,
                exclude_from_scale_domains: false,
                visible: None,
                details: None,
                zindex: None,
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

#[async_trait]
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

#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledTreeRect {
    pub(crate) state: CompiledMarkState,
    pub(crate) node_mode: TreeRectNodeMode,
}

impl CompiledMarkCore for CompiledTreeRect {
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

    fn mark_specific_default(&self, channel: &str) -> Option<ScalarValue> {
        tree_rect_channel_defaults(channel)
    }
}

#[typetag::serde]
#[async_trait]
impl CompiledMark for CompiledTreeRect {
    async fn render_from_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &dyn MarkRuntimeContext,
        _coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
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
        let nodes = if self.should_join_render_data(data) {
            nodes_for_render_data(data, &selected_nodes)?
        } else {
            selected_nodes
        };
        let len = nodes.len();

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
        let corner_radius = coerce_numeric_channel_with_renderer(
            self,
            data,
            scalars,
            "corner_radius",
            &mark_context,
            0.0,
        )?;
        let u = coerce_numeric_channel_with_renderer(self, data, scalars, "u", &mark_context, 0.0)?;
        let u2 =
            coerce_numeric_channel_with_renderer(self, data, scalars, "u2", &mark_context, 1.0)?;
        let v = coerce_numeric_channel_with_renderer(self, data, scalars, "v", &mark_context, 0.0)?;
        let v2 =
            coerce_numeric_channel_with_renderer(self, data, scalars, "v2", &mark_context, 1.0)?;
        let u = u.as_vec(len, None);
        let u2 = u2.as_vec(len, None);
        let v = v.as_vec(len, None);
        let v2 = v2.as_vec(len, None);
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

        Ok(vec![SceneMark::Rect(SceneRectMark {
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
            stroke,
            stroke_width,
            corner_radius,
            indices: None,
            zindex: self.state.zindex,
        })])
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

fn nodes_for_render_data<'a>(
    data: Option<&RecordBatch>,
    selected_nodes: &[&'a VisibleTreemapNode],
) -> Result<Vec<&'a VisibleTreemapNode>, AvengerChartError> {
    let Some(data) = data else {
        return Ok(selected_nodes.to_vec());
    };
    if data.num_rows() == 0 {
        return Ok(Vec::new());
    }
    let Some(path_level_names) = path_level_names_for_selected_nodes(selected_nodes) else {
        return Ok(selected_nodes.to_vec());
    };
    let has_all_path_columns = path_level_names
        .iter()
        .all(|name| data.column_by_name(name).is_some());
    if !has_all_path_columns {
        if data.num_columns() == 1 && data.column_by_name("_dummy").is_some() {
            return Ok(selected_nodes.to_vec());
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
        let node = node_by_path_id.get(path_id.as_str()).ok_or_else(|| {
            AvengerChartError::InvalidArgument(format!(
                "TreeRect mark data row {row_index} resolved to path '{path_id}', which is not a visible treemap cell for this mark"
            ))
        })?;
        nodes.push(*node);
    }
    Ok(nodes)
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
        array::{Float64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    };
    use avenger_chart::plot::Plot;
    use datafusion::{
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
}
