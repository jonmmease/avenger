use std::{any::Any, collections::HashMap};

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
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

use crate::guide::TreemapGuide;
use crate::layout::{
    HierarchyInputRow, HierarchyLayout, build_hierarchy_layout, collect_hierarchy_rows,
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
    pub display_levels: usize,
}

impl Default for HierarchyViewWindow {
    fn default() -> Self {
        Self {
            root_path_id: None,
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

    pub fn display_levels(mut self, levels: usize) -> Self {
        self.view_window.display_levels = levels;
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
}

impl TreemapTransform {
    pub fn solve_rows(
        &self,
        rows: Vec<HierarchyInputRow>,
        plot_width: f32,
        plot_height: f32,
    ) -> Result<TreemapCoordMeasurement, AvengerChartError> {
        let layout = build_hierarchy_layout(
            rows,
            &self.view_window,
            TreemapRect::new(0.0, 0.0, plot_width, plot_height),
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
}

#[async_trait]
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
        let measurement = self.solve_rows(rows, request.plot_width, request.plot_height)?;
        Ok(Some(Box::new(measurement)))
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
}
