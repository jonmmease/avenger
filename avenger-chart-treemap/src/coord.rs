use std::{any::Any, collections::HashMap, ops::RangeInclusive};

use async_trait::async_trait;
use avenger_chart_core::{
    AvengerChartError, CoordMeasurement, CoordinateMeasureRequest, CoordinateMeasurementProvider,
    CoordinateSystem, CoordinateSystemCore, CoordinateSystemTransform,
    CoordinateSystemTransformCore, DefaultLogicalExprNodeExt, IntoExpr, PlotGeometry,
    PointGeometry, SerializableExpr,
};
use avenger_common::value::ScalarOrArray;
use avenger_scales::scales::ScaleImpl;
use datafusion::{common::ScalarValue, prelude::SessionContext};
use datafusion_proto::protobuf::LogicalExprNode;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

use crate::guide::TreemapGuide;
use crate::layout::{
    HierarchyInputRow, HierarchyLayout, build_hierarchy_layout_with_options, collect_hierarchy_rows,
};

pub const ROOT_PATH_ID: &str = "__root__";

#[serde_as]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TreemapPathLevel {
    pub name: String,
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub expr: LogicalExprNode,
}

impl TreemapPathLevel {
    pub fn new(name: impl Into<String>, expr: impl IntoExpr) -> Self {
        Self {
            name: name.into(),
            expr: LogicalExprNode::from_default_expr(expr.into_expr())
                .expect("serialize treemap path expression"),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HierarchyViewWindow {
    pub root_path_id: Option<String>,
    pub root_path_param: Option<String>,
    pub display_levels: usize,
}

impl Default for HierarchyViewWindow {
    fn default() -> Self {
        Self {
            root_path_id: None,
            root_path_param: None,
            display_levels: usize::MAX,
        }
    }
}

impl HierarchyViewWindow {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn root_path_id(mut self, id: impl Into<String>) -> Self {
        self.root_path_id = Some(id.into());
        self
    }

    pub fn root_path_param(mut self, param: impl Into<String>) -> Self {
        self.root_path_param = Some(param.into());
        self
    }

    pub fn display_levels(mut self, levels: usize) -> Self {
        self.display_levels = levels;
        self
    }
}

#[serde_as]
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Treemap {
    path: Vec<TreemapPathLevel>,
    #[serde_as(as = "Option<FromInto<SerializableExpr>>")]
    value: Option<LogicalExprNode>,
    view_window: HierarchyViewWindow,
    layout_options: TreemapLayoutOptions,
}

impl Treemap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn path<I, E>(mut self, exprs: I) -> Self
    where
        I: IntoIterator<Item = E>,
        E: IntoExpr,
    {
        self.path = exprs
            .into_iter()
            .enumerate()
            .map(|(index, expr)| TreemapPathLevel::new(format!("level_{index}"), expr))
            .collect();
        self
    }

    pub fn path_levels<I>(mut self, levels: I) -> Self
    where
        I: IntoIterator<Item = TreemapPathLevel>,
    {
        self.path = levels.into_iter().collect();
        self
    }

    pub fn path_columns<I, S>(mut self, columns: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.path = columns
            .into_iter()
            .map(|column| {
                let column = column.into();
                TreemapPathLevel::new(column.clone(), datafusion::logical_expr::col(column))
            })
            .collect();
        self
    }

    pub fn value(mut self, expr: impl IntoExpr) -> Self {
        self.value = Some(
            LogicalExprNode::from_default_expr(expr.into_expr())
                .expect("serialize treemap value expression"),
        );
        self
    }

    pub fn view_window(mut self, view_window: HierarchyViewWindow) -> Self {
        self.view_window = view_window;
        self
    }

    pub fn root_path_id(mut self, id: impl Into<String>) -> Self {
        self.view_window.root_path_id = Some(id.into());
        self
    }

    pub fn root_path_param(mut self, param: impl Into<String>) -> Self {
        self.view_window.root_path_param = Some(param.into());
        self
    }

    pub fn display_levels(mut self, levels: usize) -> Self {
        self.view_window.display_levels = levels;
        self
    }

    pub fn header_bars(mut self, header_bars: TreemapHeaderBars) -> Self {
        self.layout_options.header_bars = header_bars;
        self
    }

    pub fn padding(mut self, padding: TreemapPadding) -> Self {
        self.layout_options.padding = padding;
        self
    }
}

impl CoordinateSystemCore for Treemap {
    fn required_channels(&self) -> &'static [&'static str] {
        &[]
    }

    fn validate(&self) -> Result<(), AvengerChartError> {
        validate_path_levels(&self.path)
    }
}

impl CoordinateSystem for Treemap {
    type Guide = TreemapGuide;

    fn create_transform(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(TreemapTransform {
            path: self.path.clone(),
            value: self.value.clone(),
            view_window: self.view_window.clone(),
            layout_options: self.layout_options.clone(),
        })
    }
}

#[serde_as]
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TreemapTransform {
    pub path: Vec<TreemapPathLevel>,
    #[serde_as(as = "Option<FromInto<SerializableExpr>>")]
    pub value: Option<LogicalExprNode>,
    pub view_window: HierarchyViewWindow,
    pub layout_options: TreemapLayoutOptions,
}

impl TreemapTransform {
    pub fn solve_rows(
        &self,
        rows: Vec<HierarchyInputRow>,
        plot_width: f32,
        plot_height: f32,
    ) -> Result<TreemapCoordMeasurement, AvengerChartError> {
        let layout = build_hierarchy_layout_with_options(
            rows,
            &self.view_window,
            TreemapRect::new(0.0, 0.0, plot_width, plot_height),
            &self.layout_options,
        )?;
        Ok(TreemapCoordMeasurement { layout })
    }
}

impl CoordinateSystemTransformCore for TreemapTransform {
    fn required_channels(&self) -> &'static [&'static str] {
        &[]
    }

    fn transform(
        &self,
        _position_channels: &HashMap<&str, ScalarOrArray<f32>>,
        _position_values: Option<&HashMap<&str, Vec<ScalarValue>>>,
        plot_width: f32,
        plot_height: f32,
    ) -> Result<Box<dyn PlotGeometry>, AvengerChartError> {
        Ok(Box::new(PointGeometry {
            x: ScalarOrArray::new_scalar(plot_width / 2.0),
            y: ScalarOrArray::new_scalar(plot_height / 2.0),
        }))
    }

    fn default_scale_options(
        &self,
        _channel: &str,
        _scale_impl: &dyn ScaleImpl,
    ) -> HashMap<String, ScalarValue> {
        HashMap::new()
    }

    fn measurement_provider(&self) -> Option<&dyn CoordinateMeasurementProvider> {
        Some(self)
    }

    fn runtime_param_dependencies(&self) -> Vec<String> {
        self.view_window
            .root_path_param
            .iter()
            .cloned()
            .collect::<Vec<_>>()
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl CoordinateMeasurementProvider for TreemapTransform {
    async fn measure_coordinate(
        &self,
        request: CoordinateMeasureRequest<'_>,
    ) -> Result<Option<Box<dyn CoordMeasurement>>, AvengerChartError> {
        validate_path_levels(&self.path)?;
        let Some(data) = request.data else {
            return Err(AvengerChartError::InvalidArgument(
                "Treemap coordinate measurement requires plot or facet-scoped data".to_string(),
            ));
        };

        let value = self.value.as_ref().ok_or_else(|| {
            AvengerChartError::InvalidArgument(
                "Treemap coordinate measurement requires a value aggregate expression".to_string(),
            )
        })?;
        let rows =
            collect_treemap_rows(data.clone(), request.session_context, &self.path, value).await?;
        let view_window = resolve_view_window(&self.view_window, request.params)?;
        let measurement = build_hierarchy_layout_with_options(
            rows,
            &view_window,
            TreemapRect::new(0.0, 0.0, request.plot_width, request.plot_height),
            &self.layout_options,
        )?;
        Ok(Some(Box::new(TreemapCoordMeasurement {
            layout: measurement,
        })))
    }
}

#[typetag::serde]
impl CoordinateSystemTransform for TreemapTransform {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }
}

#[derive(Clone, Debug)]
pub struct TreemapCoordMeasurement {
    pub layout: HierarchyLayout,
}

impl TreemapCoordMeasurement {
    pub fn nodes(&self) -> &[TreemapNode] {
        &self.layout.nodes
    }

    pub fn visible_nodes(&self) -> &[VisibleTreemapNode] {
        &self.layout.visible_nodes
    }

    pub fn visible_terminal_nodes(&self) -> impl Iterator<Item = &VisibleTreemapNode> {
        self.layout
            .visible_nodes
            .iter()
            .filter(|node| node.is_visible_leaf)
    }

    pub fn breadcrumbs(&self) -> &[TreemapNode] {
        &self.layout.breadcrumbs
    }

    pub fn node(&self, path_id: &str) -> Option<&TreemapNode> {
        self.layout.node(path_id)
    }
}

impl CoordMeasurement for TreemapCoordMeasurement {
    fn clone_box(&self) -> Box<dyn CoordMeasurement> {
        Box::new(self.clone())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TreemapLayoutOptions {
    pub header_bars: TreemapHeaderBars,
    pub padding: TreemapPadding,
}

impl Default for TreemapLayoutOptions {
    fn default() -> Self {
        Self {
            header_bars: TreemapHeaderBars::none(),
            padding: TreemapPadding::default(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TreemapHeaderBars {
    pub enabled: bool,
    pub min_relative_depth: usize,
    pub max_relative_depth: usize,
    pub height_px: f32,
    pub min_width_px: f32,
    pub min_height_px: f32,
}

impl TreemapHeaderBars {
    pub fn none() -> Self {
        Self {
            enabled: false,
            min_relative_depth: 1,
            max_relative_depth: 1,
            height_px: 20.0,
            min_width_px: 32.0,
            min_height_px: 28.0,
        }
    }

    pub fn enabled() -> Self {
        Self {
            enabled: true,
            ..Self::none()
        }
    }

    pub fn depth_range(mut self, range: RangeInclusive<usize>) -> Self {
        self.min_relative_depth = *range.start();
        self.max_relative_depth = *range.end();
        self
    }

    pub fn height_px(mut self, height: f32) -> Self {
        self.height_px = height;
        self
    }

    pub fn min_size_px(mut self, width: f32, height: f32) -> Self {
        self.min_width_px = width;
        self.min_height_px = height;
        self
    }

    pub(crate) fn is_enabled_for(
        &self,
        relative_depth: usize,
        rect: TreemapRect,
        has_children: bool,
    ) -> bool {
        self.enabled
            && has_children
            && relative_depth >= self.min_relative_depth
            && relative_depth <= self.max_relative_depth
            && rect.width >= self.min_width_px
            && rect.height >= self.min_height_px
            && self.height_px > 0.0
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TreemapPadding {
    pub outer_px: f32,
    pub inner_px: f32,
    pub depth_inner_px: Vec<f32>,
    pub content_inset_px: f32,
}

impl Default for TreemapPadding {
    fn default() -> Self {
        Self {
            outer_px: 0.0,
            inner_px: 0.0,
            depth_inner_px: Vec::new(),
            content_inset_px: 0.0,
        }
    }
}

impl TreemapPadding {
    pub fn new(inner_px: f32) -> Self {
        Self {
            inner_px,
            ..Self::default()
        }
    }

    pub fn outer_px(mut self, outer_px: f32) -> Self {
        self.outer_px = outer_px;
        self
    }

    pub fn inner_px(mut self, inner_px: f32) -> Self {
        self.inner_px = inner_px;
        self
    }

    pub fn depth_inner_px<I>(mut self, values: I) -> Self
    where
        I: IntoIterator<Item = f32>,
    {
        self.depth_inner_px = values.into_iter().collect();
        self
    }

    pub fn content_inset_px(mut self, content_inset_px: f32) -> Self {
        self.content_inset_px = content_inset_px;
        self
    }

    pub(crate) fn inner_px_for_depth(&self, relative_depth: usize) -> f32 {
        self.depth_inner_px
            .get(relative_depth)
            .copied()
            .unwrap_or(self.inner_px)
            .max(0.0)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TreemapPathComponent {
    pub name: String,
    pub value: ScalarValue,
    pub label: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TreemapRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl TreemapRect {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn area(&self) -> f32 {
        self.width.max(0.0) * self.height.max(0.0)
    }

    pub fn inset(self, amount: f32) -> Self {
        let amount = amount.max(0.0);
        Self {
            x: self.x + amount,
            y: self.y + amount,
            width: (self.width - amount * 2.0).max(0.0),
            height: (self.height - amount * 2.0).max(0.0),
        }
    }

    pub fn inset_top(self, amount: f32) -> Self {
        let amount = amount.max(0.0).min(self.height.max(0.0));
        Self {
            x: self.x,
            y: self.y + amount,
            width: self.width,
            height: (self.height - amount).max(0.0),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TreemapNode {
    pub path_id: String,
    pub depth: usize,
    pub label: String,
    pub path: Vec<TreemapPathComponent>,
    pub value: f64,
    pub parent_path_id: Option<String>,
    pub child_path_ids: Vec<String>,
    pub is_data_leaf: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VisibleTreemapNode {
    pub node: TreemapNode,
    pub rect: TreemapRect,
    pub outer_rect: TreemapRect,
    pub content_rect: TreemapRect,
    pub header_rect: Option<TreemapRect>,
    pub label_rect: TreemapRect,
    pub view_depth: usize,
    pub display_levels: usize,
    pub is_visible_leaf: bool,
    pub has_hidden_descendants: bool,
    pub can_zoom: bool,
}

async fn collect_treemap_rows(
    data: datafusion::dataframe::DataFrame,
    ctx: &SessionContext,
    path: &[TreemapPathLevel],
    value: &LogicalExprNode,
) -> Result<Vec<HierarchyInputRow>, AvengerChartError> {
    let group_exprs = path
        .iter()
        .map(|level| Ok(level.expr.to_default_expr(ctx)?.alias(level.name.clone())))
        .collect::<Result<Vec<_>, AvengerChartError>>()?;
    let value_expr = value.to_default_expr(ctx)?.alias("__treemap_value");
    let batches = data
        .aggregate(group_exprs, vec![value_expr])
        .map_err(AvengerChartError::DataFusionError)?
        .collect()
        .await
        .map_err(AvengerChartError::DataFusionError)?;
    collect_hierarchy_rows(&batches, path)
}

fn resolve_view_window(
    view_window: &HierarchyViewWindow,
    params: &IndexMap<String, ScalarValue>,
) -> Result<HierarchyViewWindow, AvengerChartError> {
    let mut resolved = view_window.clone();
    if let Some(param) = &view_window.root_path_param
        && let Some(value) = params.get(param)
        && let Some(root_path_id) = scalar_to_root_path_param(value)?
    {
        resolved.root_path_id = if root_path_id == ROOT_PATH_ID || root_path_id.is_empty() {
            None
        } else {
            Some(root_path_id)
        };
    }
    Ok(resolved)
}

fn scalar_to_root_path_param(value: &ScalarValue) -> Result<Option<String>, AvengerChartError> {
    match value {
        ScalarValue::Utf8(value) | ScalarValue::LargeUtf8(value) => Ok(value.clone()),
        ScalarValue::Null => Ok(None),
        other if other.is_null() => Ok(None),
        other => Err(AvengerChartError::InvalidArgument(format!(
            "Treemap root path param must be a string or null, received {}",
            other.data_type()
        ))),
    }
}

fn validate_path_levels(levels: &[TreemapPathLevel]) -> Result<(), AvengerChartError> {
    if levels.is_empty() {
        return Err(AvengerChartError::InvalidArgument(
            "Treemap requires at least one path level".to_string(),
        ));
    }
    let mut seen = std::collections::HashSet::new();
    for level in levels {
        if level.name.is_empty() || level.name.starts_with("__") {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Treemap path level name '{}' is invalid",
                level.name
            )));
        }
        if !seen.insert(level.name.as_str()) {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Treemap path level name '{}' is duplicated",
                level.name
            )));
        }
    }
    Ok(())
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
    use avenger_chart_core::CompiledMark;
    use datafusion::{functions_aggregate::expr_fn::sum, logical_expr::col};
    use indexmap::IndexMap;

    #[tokio::test]
    async fn provider_aggregates_plot_data_into_treemap_measurement() {
        let ctx = SessionContext::new();
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("region", DataType::Utf8, false),
                Field::new("product", DataType::Utf8, false),
                Field::new("sales", DataType::Float64, false),
            ])),
            vec![
                Arc::new(StringArray::from(vec!["East", "East", "West"])),
                Arc::new(StringArray::from(vec!["A", "A", "B"])),
                Arc::new(Float64Array::from(vec![2.0, 3.0, 5.0])),
            ],
        )
        .unwrap();
        let df = ctx.read_batch(batch).unwrap();
        let transform = Treemap::new()
            .path_columns(["region", "product"])
            .value(sum(col("sales")))
            .create_transform();
        let transform = transform
            .as_any()
            .downcast_ref::<TreemapTransform>()
            .unwrap();
        let params = IndexMap::new();
        let compiled_marks = Vec::<Arc<dyn CompiledMark>>::new();
        let facet_path = Vec::new();
        let request = CoordinateMeasureRequest {
            plot_width: 200.0,
            plot_height: 100.0,
            params: &params,
            session_context: &ctx,
            data: Some(&df),
            compiled_marks: &compiled_marks,
            facet_path: &facet_path,
            scales: HashMap::new(),
        };

        let measurement = transform
            .measure_coordinate(request)
            .await
            .unwrap()
            .expect("treemap provider should install measurement");
        let measurement = measurement
            .as_any()
            .downcast_ref::<TreemapCoordMeasurement>()
            .unwrap();

        assert_eq!(measurement.node(ROOT_PATH_ID).unwrap().value, 10.0);
        assert_eq!(
            measurement.node("region=East/product=A").unwrap().value,
            5.0
        );
        assert_eq!(
            measurement.node("region=West/product=B").unwrap().value,
            5.0
        );
        assert_eq!(measurement.visible_nodes()[0].rect.width, 200.0);
        assert_eq!(measurement.visible_nodes()[0].rect.height, 100.0);
    }

    #[tokio::test]
    async fn provider_uses_root_path_param_for_zoom_window_measurement() {
        let ctx = SessionContext::new();
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("region", DataType::Utf8, false),
                Field::new("product", DataType::Utf8, false),
                Field::new("sales", DataType::Float64, false),
            ])),
            vec![
                Arc::new(StringArray::from(vec!["East", "East", "West"])),
                Arc::new(StringArray::from(vec!["A", "B", "C"])),
                Arc::new(Float64Array::from(vec![2.0, 3.0, 5.0])),
            ],
        )
        .unwrap();
        let df = ctx.read_batch(batch).unwrap();
        let transform = Treemap::new()
            .path_columns(["region", "product"])
            .value(sum(col("sales")))
            .root_path_param("treemap_root")
            .display_levels(1)
            .create_transform();
        let transform = transform
            .as_any()
            .downcast_ref::<TreemapTransform>()
            .unwrap();
        assert_eq!(
            transform.runtime_param_dependencies(),
            vec!["treemap_root".to_string()]
        );

        let compiled_marks = Vec::<Arc<dyn CompiledMark>>::new();
        let facet_path = Vec::new();
        let mut params = IndexMap::new();
        params.insert(
            "treemap_root".to_string(),
            ScalarValue::Utf8(Some("region=East".to_string())),
        );
        let request = CoordinateMeasureRequest {
            plot_width: 200.0,
            plot_height: 100.0,
            params: &params,
            session_context: &ctx,
            data: Some(&df),
            compiled_marks: &compiled_marks,
            facet_path: &facet_path,
            scales: HashMap::new(),
        };
        let measurement = transform
            .measure_coordinate(request)
            .await
            .unwrap()
            .expect("treemap provider should install measurement");
        let measurement = measurement
            .as_any()
            .downcast_ref::<TreemapCoordMeasurement>()
            .unwrap();
        assert_eq!(
            measurement.visible_nodes()[0].node.path_id,
            "region=East".to_string()
        );
        assert_eq!(measurement.visible_nodes()[0].view_depth, 1);
        assert_eq!(
            measurement.breadcrumbs()[1].path_id,
            "region=East".to_string()
        );

        params.insert(
            "treemap_root".to_string(),
            ScalarValue::Utf8(Some(ROOT_PATH_ID.to_string())),
        );
        let request = CoordinateMeasureRequest {
            plot_width: 200.0,
            plot_height: 100.0,
            params: &params,
            session_context: &ctx,
            data: Some(&df),
            compiled_marks: &compiled_marks,
            facet_path: &facet_path,
            scales: HashMap::new(),
        };
        let measurement = transform
            .measure_coordinate(request)
            .await
            .unwrap()
            .expect("treemap provider should install measurement");
        let measurement = measurement
            .as_any()
            .downcast_ref::<TreemapCoordMeasurement>()
            .unwrap();
        assert_eq!(measurement.visible_nodes()[0].node.path_id, ROOT_PATH_ID);
        assert_eq!(measurement.breadcrumbs().len(), 1);
    }
}
