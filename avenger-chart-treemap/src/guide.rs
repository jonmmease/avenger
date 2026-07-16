use std::{any::Any, collections::HashMap, sync::Arc};

use async_trait::async_trait;
use avenger_chart_core::{
    AvengerChartError, CompiledGuide, CompiledMarkCore, CoordMeasurement, CoordinateGuide,
    EventDatumFieldSpec, GuideEventDatumRows, GuideRenderContext, GuideSharingContext, GuideUpdate,
    LayoutBounds, OverflowSpaceRequirement, Theme,
};
use avenger_color::ColorOrGradient;
use avenger_common::value::ScalarOrArray;
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::{
    group::Clip, mark::SceneMark, rect::SceneRectMark, text::SceneTextMark,
};
use avenger_text::{
    default_text_engine,
    measurement::{TextMeasurementConfig, truncate_text_to_limit_with},
    types::{FontStyle, FontWeight, FontWeightNameSpec},
};
use datafusion::{
    arrow::{
        array::{ArrayRef, BooleanArray, Float64Array, Int64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    common::ScalarValue,
    dataframe::DataFrame,
    prelude::SessionContext,
};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::{
    ROOT_PATH_ID, TreemapCoordMeasurement, TreemapNode, VisibleTreemapNode,
    event::{
        HIERARCHY_CAN_ZOOM_FIELD, HIERARCHY_DEPTH_FIELD, HIERARCHY_DISPLAY_LEVELS_FIELD,
        HIERARCHY_HAS_HIDDEN_DESCENDANTS_FIELD, HIERARCHY_IS_DATA_LEAF_FIELD,
        HIERARCHY_IS_VISIBLE_LEAF_FIELD, HIERARCHY_LEVEL_NAME_FIELD,
        HIERARCHY_PARENT_PATH_ID_FIELD, HIERARCHY_PATH_ID_FIELD, HIERARCHY_SURFACE_KIND_BREADCRUMB,
        HIERARCHY_SURFACE_KIND_FIELD, HIERARCHY_SURFACE_KIND_GUIDE_HEADER, HIERARCHY_TITLE_FIELD,
        HIERARCHY_VALUE_FIELD, HIERARCHY_VIEW_DEPTH_FIELD, treemap_guide_event_datum_field_specs,
    },
};

const HEADER_HEIGHT: f32 = 18.0;
const BREADCRUMB_HEIGHT: f32 = 20.0;
const BREADCRUMB_GAP: f32 = 4.0;
const GUIDE_TEXT_INSET: f32 = 4.0;
const GUIDE_TEXT_FONT_SIZE: f32 = 10.0;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TreemapGuide {
    headers: bool,
    separators: bool,
    breadcrumbs: bool,
}

impl TreemapGuide {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn headers(mut self, visible: bool) -> Self {
        self.headers = visible;
        self
    }

    pub fn separators(mut self, visible: bool) -> Self {
        self.separators = visible;
        self
    }

    pub fn breadcrumbs(mut self, visible: bool) -> Self {
        self.breadcrumbs = visible;
        self
    }
}

impl GuideUpdate for TreemapGuide {
    fn update(self, other: Self) -> Self {
        other
    }
}

impl CoordinateGuide for TreemapGuide {
    type Axis = ();

    fn set_axes(&mut self, _axes: HashMap<String, Self::Axis>) {}

    fn set_compiled_marks<M>(&mut self, _compiled_marks: &[Arc<M>], _ctx: &SessionContext)
    where
        M: CompiledMarkCore + ?Sized,
    {
    }

    fn update(&mut self, _other: Self) {}

    fn build(self) -> Box<dyn CompiledGuide> {
        Box::new(self)
    }
}

#[typetag::serde]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl CompiledGuide for TreemapGuide {
    async fn measure_overflow(
        &self,
        _scales: &HashMap<String, ConfiguredScale>,
        _plot_width: f32,
        _plot_height: f32,
        _theme: &Theme,
        _params: &IndexMap<String, ScalarValue>,
        _data_override: Option<&DataFrame>,
        _ctx: &SessionContext,
        _sharing_context: GuideSharingContext<'_>,
        _coord_measurement: Option<&dyn CoordMeasurement>,
    ) -> Result<OverflowSpaceRequirement, AvengerChartError> {
        Ok(OverflowSpaceRequirement {
            top: if self.breadcrumbs {
                BREADCRUMB_HEIGHT + BREADCRUMB_GAP
            } else {
                0.0
            },
            ..OverflowSpaceRequirement::default()
        })
    }

    async fn evaluate(
        &self,
        _scales: &HashMap<String, ConfiguredScale>,
        _plot_width: f32,
        _plot_height: f32,
        plot_bounds: &LayoutBounds,
        _guide_overflow: &OverflowSpaceRequirement,
        _theme: &Theme,
        _params: &IndexMap<String, ScalarValue>,
        _ctx: &SessionContext,
        _data_override: Option<&DataFrame>,
        _sharing_context: GuideSharingContext<'_>,
        coord_measurement: &dyn CoordMeasurement,
        _render_context: GuideRenderContext<'_>,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let measurement = coord_measurement
            .as_any()
            .downcast_ref::<TreemapCoordMeasurement>()
            .ok_or_else(|| {
                AvengerChartError::InternalError(
                    "TreemapGuide requires TreemapCoordMeasurement".to_string(),
                )
            })?;
        let mut marks = Vec::new();

        if self.separators {
            if let Some(mark) = make_separator_mark(plot_bounds, measurement) {
                marks.push(mark);
            }
        }
        if self.headers {
            let header_nodes = header_nodes(measurement);
            if !header_nodes.is_empty() {
                marks.push(make_header_hit_rect(plot_bounds, &header_nodes));
                marks.push(make_header_text(plot_bounds, &header_nodes));
            }
        }
        if self.breadcrumbs {
            let breadcrumbs = measurement.breadcrumbs();
            if !breadcrumbs.is_empty() {
                marks.push(make_breadcrumb_hit_rect(plot_bounds, breadcrumbs));
                marks.push(make_breadcrumb_text(plot_bounds, breadcrumbs));
            }
        }
        Ok(marks)
    }

    fn event_datum_field_specs(&self) -> Vec<EventDatumFieldSpec> {
        treemap_guide_event_datum_field_specs()
    }

    fn event_datum_rows(
        &self,
        guide_marks: &[SceneMark],
        _plot_width: f32,
        _plot_height: f32,
        _params: &IndexMap<String, ScalarValue>,
        _ctx: &SessionContext,
        coord_measurement: &dyn CoordMeasurement,
    ) -> Result<Vec<GuideEventDatumRows>, AvengerChartError> {
        let measurement = coord_measurement
            .as_any()
            .downcast_ref::<TreemapCoordMeasurement>()
            .ok_or_else(|| {
                AvengerChartError::InternalError(
                    "TreemapGuide requires TreemapCoordMeasurement".to_string(),
                )
            })?;
        let header_rows = if self.headers {
            Some(guide_event_datum_batch(
                HIERARCHY_SURFACE_KIND_GUIDE_HEADER,
                header_nodes(measurement)
                    .into_iter()
                    .map(|node| GuideDatumNode::Visible(node))
                    .collect(),
            )?)
        } else {
            None
        };
        let breadcrumb_rows = if self.breadcrumbs {
            Some(guide_event_datum_batch(
                HIERARCHY_SURFACE_KIND_BREADCRUMB,
                measurement
                    .breadcrumbs()
                    .iter()
                    .map(GuideDatumNode::Breadcrumb)
                    .collect(),
            )?)
        } else {
            None
        };

        Ok(guide_marks
            .iter()
            .enumerate()
            .filter_map(|(guide_mark_index, mark)| {
                let name = scene_mark_name(mark)?;
                match name {
                    "treemap_header_hit" | "treemap_header" => {
                        header_rows.as_ref().map(|rows| GuideEventDatumRows {
                            guide_mark_index,
                            rows: rows.clone(),
                        })
                    }
                    "treemap_breadcrumb_hit" | "treemap_breadcrumb" => {
                        breadcrumb_rows.as_ref().map(|rows| GuideEventDatumRows {
                            guide_mark_index,
                            rows: rows.clone(),
                        })
                    }
                    _ => None,
                }
            })
            .collect())
    }

    fn get_clip(
        &self,
        _plot_width: f32,
        _plot_height: f32,
        _scales: &HashMap<String, ConfiguredScale>,
    ) -> Clip {
        Clip::None
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn header_nodes(measurement: &TreemapCoordMeasurement) -> Vec<&VisibleTreemapNode> {
    measurement
        .visible_nodes()
        .iter()
        .filter(|node| node.node.path_id != ROOT_PATH_ID && !node.node.child_path_ids.is_empty())
        .filter(|node| node.node.depth == node.view_depth + 1)
        .collect()
}

fn make_separator_mark(
    plot_bounds: &LayoutBounds,
    measurement: &TreemapCoordMeasurement,
) -> Option<SceneMark> {
    let nodes = header_nodes(measurement);
    if nodes.is_empty() {
        return None;
    }
    Some(SceneMark::Rect(SceneRectMark {
        name: "treemap_group_separator".to_string(),
        interactive: false,
        clip: false,
        len: nodes.len() as u32,
        gradients: Vec::new(),
        x: ScalarOrArray::from(
            nodes
                .iter()
                .map(|node| plot_bounds.x + node.outer_rect.x)
                .collect::<Vec<_>>(),
        ),
        y: ScalarOrArray::from(
            nodes
                .iter()
                .map(|node| plot_bounds.y + node.outer_rect.y)
                .collect::<Vec<_>>(),
        ),
        width: Some(ScalarOrArray::from(
            nodes
                .iter()
                .map(|node| node.outer_rect.width)
                .collect::<Vec<_>>(),
        )),
        height: Some(ScalarOrArray::from(
            nodes
                .iter()
                .map(|node| node.outer_rect.height)
                .collect::<Vec<_>>(),
        )),
        x2: None,
        y2: None,
        fill: ScalarOrArray::new_scalar(ColorOrGradient::transparent()),
        fill_pattern: avenger_scenegraph::marks::pattern::default_no_fill_pattern(),
        stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.25, 0.25, 0.25, 1.0])),
        stroke_width: ScalarOrArray::new_scalar(1.0),
        corner_radius: ScalarOrArray::new_scalar(0.0),
        indices: None,
        zindex: Some(20),
    }))
}

fn make_header_hit_rect(plot_bounds: &LayoutBounds, nodes: &[&VisibleTreemapNode]) -> SceneMark {
    SceneMark::Rect(SceneRectMark {
        name: "treemap_header_hit".to_string(),
        interactive: true,
        clip: false,
        len: nodes.len() as u32,
        gradients: Vec::new(),
        x: ScalarOrArray::from(
            nodes
                .iter()
                .map(|node| plot_bounds.x + guide_header_rect(node).x)
                .collect::<Vec<_>>(),
        ),
        y: ScalarOrArray::from(
            nodes
                .iter()
                .map(|node| plot_bounds.y + guide_header_rect(node).y)
                .collect::<Vec<_>>(),
        ),
        width: Some(ScalarOrArray::from(
            nodes
                .iter()
                .map(|node| guide_header_rect(node).width)
                .collect::<Vec<_>>(),
        )),
        height: Some(ScalarOrArray::from(
            nodes
                .iter()
                .map(|node| guide_header_rect(node).height)
                .collect::<Vec<_>>(),
        )),
        x2: None,
        y2: None,
        fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])),
        fill_pattern: avenger_scenegraph::marks::pattern::default_no_fill_pattern(),
        stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])),
        stroke_width: ScalarOrArray::new_scalar(0.0),
        corner_radius: ScalarOrArray::new_scalar(0.0),
        indices: None,
        zindex: Some(30),
    })
}

fn make_header_text(plot_bounds: &LayoutBounds, nodes: &[&VisibleTreemapNode]) -> SceneMark {
    let labels = truncate_labels(nodes.iter().map(|node| {
        (
            node.node.label.as_str(),
            guide_text_limit(guide_header_rect(node).width),
        )
    }));
    let mut mark = SceneTextMark {
        name: "treemap_header".to_string(),
        interactive: true,
        clip: false,
        len: nodes.len() as u32,
        text: ScalarOrArray::from(labels),
        x: ScalarOrArray::from(
            nodes
                .iter()
                .map(|node| plot_bounds.x + guide_header_rect(node).x + GUIDE_TEXT_INSET)
                .collect::<Vec<_>>(),
        ),
        y: ScalarOrArray::from(
            nodes
                .iter()
                .map(|node| {
                    let rect = guide_header_rect(node);
                    plot_bounds.y + rect.y + rect.height - 5.0
                })
                .collect::<Vec<_>>(),
        ),
        limit: ScalarOrArray::from(
            nodes
                .iter()
                .map(|node| guide_text_limit(guide_header_rect(node).width))
                .collect::<Vec<_>>(),
        ),
        zindex: Some(31),
        ..SceneTextMark::default()
    };
    mark.color = ScalarOrArray::new_scalar(ColorOrGradient::Color([0.16, 0.16, 0.16, 1.0]));
    SceneMark::from(mark)
}

fn guide_header_rect(node: &VisibleTreemapNode) -> crate::TreemapRect {
    node.header_rect.unwrap_or_else(|| {
        crate::TreemapRect::new(
            node.outer_rect.x,
            node.outer_rect.y,
            node.outer_rect.width,
            HEADER_HEIGHT.min(node.outer_rect.height.max(0.0)),
        )
    })
}

fn make_breadcrumb_hit_rect(plot_bounds: &LayoutBounds, nodes: &[TreemapNode]) -> SceneMark {
    let (x, width) = breadcrumb_positions(plot_bounds, nodes);
    SceneMark::Rect(SceneRectMark {
        name: "treemap_breadcrumb_hit".to_string(),
        interactive: true,
        clip: false,
        len: nodes.len() as u32,
        gradients: Vec::new(),
        x: ScalarOrArray::from(x),
        y: ScalarOrArray::new_scalar(plot_bounds.y - BREADCRUMB_HEIGHT - BREADCRUMB_GAP),
        width: Some(ScalarOrArray::from(width)),
        height: Some(ScalarOrArray::new_scalar(BREADCRUMB_HEIGHT)),
        x2: None,
        y2: None,
        fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])),
        fill_pattern: avenger_scenegraph::marks::pattern::default_no_fill_pattern(),
        stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])),
        stroke_width: ScalarOrArray::new_scalar(0.0),
        corner_radius: ScalarOrArray::new_scalar(0.0),
        indices: None,
        zindex: Some(32),
    })
}

fn make_breadcrumb_text(plot_bounds: &LayoutBounds, nodes: &[TreemapNode]) -> SceneMark {
    let (x, width) = breadcrumb_positions(plot_bounds, nodes);
    let text = truncate_labels(nodes.iter().zip(width.iter()).map(|(node, width)| {
        let label = if node.path_id == ROOT_PATH_ID {
            "All"
        } else {
            node.label.as_str()
        };
        (label, guide_text_limit(*width))
    }));
    let mut mark = SceneTextMark {
        name: "treemap_breadcrumb".to_string(),
        interactive: true,
        clip: false,
        len: nodes.len() as u32,
        text: ScalarOrArray::from(text),
        x: ScalarOrArray::from(x.iter().map(|x| *x + GUIDE_TEXT_INSET).collect::<Vec<_>>()),
        y: ScalarOrArray::new_scalar(plot_bounds.y - BREADCRUMB_GAP - 5.0),
        limit: ScalarOrArray::from(
            width
                .iter()
                .map(|width| guide_text_limit(*width))
                .collect::<Vec<_>>(),
        ),
        zindex: Some(33),
        ..SceneTextMark::default()
    };
    mark.color = ScalarOrArray::new_scalar(ColorOrGradient::Color([0.16, 0.16, 0.16, 1.0]));
    SceneMark::from(mark)
}

fn guide_text_limit(width: f32) -> f32 {
    (width - GUIDE_TEXT_INSET * 2.0).max(0.0)
}

fn truncate_labels<'a>(labels: impl IntoIterator<Item = (&'a str, f32)>) -> Vec<String> {
    let text_engine = default_text_engine();
    let font_weight = FontWeight::Name(FontWeightNameSpec::Normal);
    let font_style = FontStyle::Normal;
    labels
        .into_iter()
        .map(|(label, limit)| {
            truncate_text_to_limit_with(label, limit, |candidate| {
                Ok::<_, std::convert::Infallible>(
                    text_engine
                        .measure_bounds_with_plain_fallback_or_approx(&TextMeasurementConfig {
                            text: candidate,
                            font: "sans-serif",
                            font_size: GUIDE_TEXT_FONT_SIZE,
                            font_weight,
                            font_style,
                            syntax_mode: avenger_text::types::TextSyntaxMode::Plain,
                            params: avenger_text::empty_label_params(),
                            number_locale: None,
                            number_locale_specs: None,
                            datetime_locale: None,
                            datetime_timezone: None,
                            datetime_locale_specs: None,
                        })
                        .width,
                )
            })
            .unwrap_or_else(|_| label.to_string())
        })
        .collect()
}

fn breadcrumb_positions(plot_bounds: &LayoutBounds, nodes: &[TreemapNode]) -> (Vec<f32>, Vec<f32>) {
    let mut x = Vec::with_capacity(nodes.len());
    let mut width = Vec::with_capacity(nodes.len());
    let mut cursor = plot_bounds.x;
    for node in nodes {
        let label = if node.path_id == ROOT_PATH_ID {
            "All"
        } else {
            node.label.as_str()
        };
        let item_width = (label.chars().count() as f32 * 7.0 + 18.0).max(28.0);
        x.push(cursor);
        width.push(item_width);
        cursor += item_width + BREADCRUMB_GAP;
    }
    (x, width)
}

fn scene_mark_name(mark: &SceneMark) -> Option<&str> {
    match mark {
        SceneMark::Rect(mark) => Some(mark.name.as_str()),
        SceneMark::Text(mark) => Some(mark.name.as_str()),
        _ => None,
    }
}

enum GuideDatumNode<'a> {
    Visible(&'a VisibleTreemapNode),
    Breadcrumb(&'a TreemapNode),
}

impl<'a> GuideDatumNode<'a> {
    fn node(&self) -> &TreemapNode {
        match self {
            Self::Visible(node) => &node.node,
            Self::Breadcrumb(node) => node,
        }
    }

    fn view_depth(&self) -> usize {
        match self {
            Self::Visible(node) => node.view_depth,
            Self::Breadcrumb(_) => 0,
        }
    }

    fn display_levels(&self) -> usize {
        match self {
            Self::Visible(node) => node.display_levels,
            Self::Breadcrumb(_) => 0,
        }
    }

    fn is_visible_leaf(&self) -> bool {
        match self {
            Self::Visible(node) => node.is_visible_leaf,
            Self::Breadcrumb(_) => false,
        }
    }

    fn has_hidden_descendants(&self) -> bool {
        match self {
            Self::Visible(node) => node.has_hidden_descendants,
            Self::Breadcrumb(_) => false,
        }
    }

    fn can_zoom(&self) -> bool {
        match self {
            Self::Visible(node) => node.can_zoom,
            Self::Breadcrumb(node) => node.path_id != ROOT_PATH_ID,
        }
    }
}

fn guide_event_datum_batch(
    surface_kind: &str,
    nodes: Vec<GuideDatumNode<'_>>,
) -> Result<RecordBatch, AvengerChartError> {
    let len = nodes.len();
    let schema = Arc::new(Schema::new(vec![
        Field::new(HIERARCHY_SURFACE_KIND_FIELD, DataType::Utf8, false),
        Field::new(HIERARCHY_PATH_ID_FIELD, DataType::Utf8, false),
        Field::new(HIERARCHY_PARENT_PATH_ID_FIELD, DataType::Utf8, true),
        Field::new(HIERARCHY_DEPTH_FIELD, DataType::Int64, false),
        Field::new(HIERARCHY_VIEW_DEPTH_FIELD, DataType::Int64, false),
        Field::new(HIERARCHY_DISPLAY_LEVELS_FIELD, DataType::Int64, false),
        Field::new(HIERARCHY_IS_DATA_LEAF_FIELD, DataType::Boolean, false),
        Field::new(HIERARCHY_IS_VISIBLE_LEAF_FIELD, DataType::Boolean, false),
        Field::new(
            HIERARCHY_HAS_HIDDEN_DESCENDANTS_FIELD,
            DataType::Boolean,
            false,
        ),
        Field::new(HIERARCHY_CAN_ZOOM_FIELD, DataType::Boolean, false),
        Field::new(HIERARCHY_VALUE_FIELD, DataType::Float64, false),
        Field::new(HIERARCHY_TITLE_FIELD, DataType::Utf8, false),
        Field::new(HIERARCHY_LEVEL_NAME_FIELD, DataType::Utf8, true),
    ]));
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(vec![surface_kind; len])) as ArrayRef,
            Arc::new(StringArray::from(
                nodes
                    .iter()
                    .map(|node| node.node().path_id.clone())
                    .collect::<Vec<_>>(),
            )),
            Arc::new(StringArray::from(
                nodes
                    .iter()
                    .map(|node| node.node().parent_path_id.clone())
                    .collect::<Vec<_>>(),
            )),
            Arc::new(Int64Array::from(
                nodes
                    .iter()
                    .map(|node| node.node().depth as i64)
                    .collect::<Vec<_>>(),
            )),
            Arc::new(Int64Array::from(
                nodes
                    .iter()
                    .map(|node| node.view_depth() as i64)
                    .collect::<Vec<_>>(),
            )),
            Arc::new(Int64Array::from(
                nodes
                    .iter()
                    .map(|node| node.display_levels() as i64)
                    .collect::<Vec<_>>(),
            )),
            Arc::new(BooleanArray::from(
                nodes
                    .iter()
                    .map(|node| node.node().is_data_leaf)
                    .collect::<Vec<_>>(),
            )),
            Arc::new(BooleanArray::from(
                nodes
                    .iter()
                    .map(|node| node.is_visible_leaf())
                    .collect::<Vec<_>>(),
            )),
            Arc::new(BooleanArray::from(
                nodes
                    .iter()
                    .map(|node| node.has_hidden_descendants())
                    .collect::<Vec<_>>(),
            )),
            Arc::new(BooleanArray::from(
                nodes.iter().map(|node| node.can_zoom()).collect::<Vec<_>>(),
            )),
            Arc::new(Float64Array::from(
                nodes
                    .iter()
                    .map(|node| node.node().value)
                    .collect::<Vec<_>>(),
            )),
            Arc::new(StringArray::from(
                nodes
                    .iter()
                    .map(|node| {
                        if node.node().path_id == ROOT_PATH_ID {
                            "All".to_string()
                        } else {
                            node.node().label.clone()
                        }
                    })
                    .collect::<Vec<_>>(),
            )),
            Arc::new(StringArray::from(
                nodes
                    .iter()
                    .map(|node| {
                        node.node()
                            .path
                            .last()
                            .map(|component| component.name.clone())
                    })
                    .collect::<Vec<_>>(),
            )),
        ],
    )
    .map_err(AvengerChartError::ArrowError)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use arrow::{
        array::{Array, BooleanArray, Float64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    };
    use avenger_chart::{
        plot::Chart,
        prelude::{ChartEventBinding, ChartEventType, Param},
    };
    use datafusion::{
        common::ScalarValue,
        functions_aggregate::expr_fn::sum,
        logical_expr::{col, lit},
    };

    use crate::{TreeRect, Treemap};

    fn guide_source_batch() -> RecordBatch {
        RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("division", DataType::Utf8, false),
                Field::new("team", DataType::Utf8, false),
                Field::new("sales", DataType::Float64, false),
            ])),
            vec![
                Arc::new(StringArray::from(vec!["D1", "D1", "D2"])),
                Arc::new(StringArray::from(vec!["A", "B", "C"])),
                Arc::new(Float64Array::from(vec![2.0, 3.0, 5.0])),
            ],
        )
        .unwrap()
    }

    fn collect_rects<'a>(marks: &'a [SceneMark], name: &str, out: &mut Vec<&'a SceneRectMark>) {
        for mark in marks {
            match mark {
                SceneMark::Rect(rect) if rect.name == name => out.push(rect),
                SceneMark::Group(group) => collect_rects(&group.marks, name, out),
                _ => {}
            }
        }
    }

    fn collect_texts<'a>(marks: &'a [SceneMark], name: &str, out: &mut Vec<&'a SceneTextMark>) {
        for mark in marks {
            match mark {
                SceneMark::Text(text) if text.name == name => out.push(text),
                SceneMark::Group(group) => collect_texts(&group.marks, name, out),
                _ => {}
            }
        }
    }

    #[tokio::test]
    async fn guide_renders_headers_separators_and_breadcrumbs_from_measurement() {
        let ctx = SessionContext::new();
        let df = ctx.read_batch(guide_source_batch()).unwrap();
        let plot = Chart::with_coord(
            Treemap::new()
                .path_columns(["division", "team"])
                .value(sum(col("sales"))),
        )
        .data(df)
        .plot_size(200.0, 100.0)
        .configure_guide(
            TreemapGuide::new()
                .headers(true)
                .separators(true)
                .breadcrumbs(true),
        )
        .mark(TreeRect::new().stroke_width(0.0));

        let evaluated = plot
            .compile(&ctx)
            .await
            .unwrap()
            .evaluate(&ctx, None)
            .await
            .unwrap();

        let mut separators = Vec::new();
        collect_rects(
            &evaluated.scene_graph.marks,
            "treemap_group_separator",
            &mut separators,
        );
        assert_eq!(separators.len(), 1);
        assert_eq!(separators[0].len, 2);
        assert_eq!(separators[0].x.as_vec(2, None), vec![10.0, 110.0]);

        let mut header_hits = Vec::new();
        collect_rects(
            &evaluated.scene_graph.marks,
            "treemap_header_hit",
            &mut header_hits,
        );
        assert_eq!(header_hits.len(), 1);
        assert_eq!(header_hits[0].len, 2);

        let mut headers = Vec::new();
        collect_texts(&evaluated.scene_graph.marks, "treemap_header", &mut headers);
        assert_eq!(headers.len(), 1);
        assert_eq!(headers[0].len, 2);
        assert_eq!(
            headers[0].text.as_vec(2, None),
            vec!["D1".to_string(), "D2".to_string()]
        );

        let mut breadcrumbs = Vec::new();
        collect_texts(
            &evaluated.scene_graph.marks,
            "treemap_breadcrumb",
            &mut breadcrumbs,
        );
        assert_eq!(breadcrumbs.len(), 1);
        assert_eq!(breadcrumbs[0].text.as_vec(1, None), vec!["All".to_string()]);
    }

    #[tokio::test]
    async fn guide_headers_use_reserved_header_rects_for_hit_geometry() {
        let ctx = SessionContext::new();
        let df = ctx.read_batch(guide_source_batch()).unwrap();
        let plot = Chart::with_coord(
            Treemap::new()
                .path_columns(["division", "team"])
                .value(sum(col("sales")))
                .header_bars(crate::TreemapHeaderBars::enabled().height_px(18.0)),
        )
        .data(df)
        .plot_size(200.0, 100.0)
        .configure_guide(TreemapGuide::new().headers(true))
        .mark(TreeRect::new().stroke_width(0.0));

        let evaluated = plot
            .compile(&ctx)
            .await
            .unwrap()
            .evaluate(&ctx, None)
            .await
            .unwrap();
        let mut header_hits = Vec::new();
        collect_rects(
            &evaluated.scene_graph.marks,
            "treemap_header_hit",
            &mut header_hits,
        );
        assert_eq!(header_hits.len(), 1);
        assert_eq!(header_hits[0].len, 2);
        assert_eq!(
            header_hits[0].height.as_ref().unwrap().as_vec(2, None),
            vec![18.0, 18.0]
        );
    }

    #[tokio::test]
    async fn guide_event_datum_rows_retain_header_and_breadcrumb_metadata() {
        let ctx = SessionContext::new();
        let df = ctx.read_batch(guide_source_batch()).unwrap();
        let plot = Chart::with_coord(
            Treemap::new()
                .path_columns(["division", "team"])
                .value(sum(col("sales"))),
        )
        .data(df)
        .plot_size(200.0, 100.0)
        .configure_guide(TreemapGuide::new().headers(true).breadcrumbs(true))
        .mark(TreeRect::new().stroke_width(0.0));

        let evaluated = plot
            .compile(&ctx)
            .await
            .unwrap()
            .evaluate(&ctx, None)
            .await
            .unwrap();
        let header_rows = evaluated
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
                values.len() == 2 && values.value(0) == HIERARCHY_SURFACE_KIND_GUIDE_HEADER
            })
            .expect("header event datum rows");

        let titles = header_rows
            .rows
            .column_by_name(HIERARCHY_TITLE_FIELD)
            .expect("title")
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("title string");
        assert_eq!(titles.value(0), "D1");

        let path_ids = header_rows
            .rows
            .column_by_name(HIERARCHY_PATH_ID_FIELD)
            .expect("path id")
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("path string");
        assert_eq!(path_ids.value(0), "division=D1");

        let zoomable = header_rows
            .rows
            .column_by_name(HIERARCHY_CAN_ZOOM_FIELD)
            .expect("can zoom")
            .as_any()
            .downcast_ref::<BooleanArray>()
            .expect("can zoom bool");
        assert!(zoomable.value(0));

        let breadcrumb_rows = evaluated
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
                values.len() == 1 && values.value(0) == HIERARCHY_SURFACE_KIND_BREADCRUMB
            })
            .expect("breadcrumb event datum rows");
        let breadcrumb_titles = breadcrumb_rows
            .rows
            .column_by_name(HIERARCHY_TITLE_FIELD)
            .expect("breadcrumb title")
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("breadcrumb title string");
        assert_eq!(breadcrumb_titles.value(0), "All");
    }

    #[tokio::test]
    async fn guide_header_and_breadcrumb_datums_support_zoom_param_binding() {
        let ctx = SessionContext::new();
        let df = ctx.read_batch(guide_source_batch()).unwrap();
        let root = Param::new("treemap_root", ScalarValue::Utf8(None));
        let compiled = Chart::with_coord(
            Treemap::new()
                .path_columns(["division", "team"])
                .value(sum(col("sales")))
                .root_path_param(root.name.clone()),
        )
        .data(df)
        .plot_size(200.0, 100.0)
        .param(root.clone())
        .configure_guide(TreemapGuide::new().headers(true).breadcrumbs(true))
        .mark(TreeRect::new().stroke_width(0.0))
        .event_binding(
            ChartEventBinding::on(ChartEventType::Click)
                .filter(
                    crate::event::hierarchy_surface_kind()
                        .eq(lit(HIERARCHY_SURFACE_KIND_GUIDE_HEADER)),
                )
                .filter(crate::event::hierarchy_can_zoom().eq(lit(true)))
                .set_param(&root, crate::event::hierarchy_path_id())
                .exact(),
        )
        .compile(&ctx)
        .await
        .unwrap();

        let binding = compiled.event_bindings().first().expect("event binding");
        assert_eq!(binding.action.param_steps().count(), 1);
        assert_eq!(
            binding.action.param_steps().next().unwrap().param_name,
            root.name
        );

        let evaluated = compiled.evaluate(&ctx, None).await.unwrap();
        let header_rows = evaluated
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
                values.len() == 2 && values.value(0) == HIERARCHY_SURFACE_KIND_GUIDE_HEADER
            })
            .expect("header event datum rows");
        assert!(
            header_rows
                .rows
                .column_by_name(HIERARCHY_PATH_ID_FIELD)
                .is_some()
        );
        assert!(
            header_rows
                .rows
                .column_by_name(HIERARCHY_LEVEL_NAME_FIELD)
                .is_some()
        );

        let zoomable = header_rows
            .rows
            .column_by_name(HIERARCHY_CAN_ZOOM_FIELD)
            .expect("can zoom")
            .as_any()
            .downcast_ref::<BooleanArray>()
            .expect("can zoom bool");
        assert!(zoomable.value(0));
    }

    #[tokio::test]
    async fn guide_breadcrumb_datums_carry_zoom_out_path_ids() {
        let ctx = SessionContext::new();
        let df = ctx.read_batch(guide_source_batch()).unwrap();
        let root = Param::new("treemap_root", ScalarValue::Utf8(None));
        let compiled = Chart::with_coord(
            Treemap::new()
                .path_columns(["division", "team"])
                .value(sum(col("sales")))
                .root_path_id("division=D1")
                .root_path_param(root.name.clone()),
        )
        .data(df)
        .plot_size(200.0, 100.0)
        .param(root.clone())
        .configure_guide(TreemapGuide::new().breadcrumbs(true))
        .mark(TreeRect::new().stroke_width(0.0))
        .event_binding(
            ChartEventBinding::on(ChartEventType::Click)
                .filter(
                    crate::event::hierarchy_surface_kind()
                        .eq(lit(HIERARCHY_SURFACE_KIND_BREADCRUMB)),
                )
                .set_param(&root, crate::event::hierarchy_path_id())
                .exact(),
        )
        .compile(&ctx)
        .await
        .unwrap();

        let binding = compiled.event_bindings().first().expect("event binding");
        assert_eq!(
            binding.action.param_steps().next().unwrap().param_name,
            root.name
        );

        let evaluated = compiled.evaluate(&ctx, None).await.unwrap();
        let breadcrumb_rows = evaluated
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
                values.len() == 2 && values.value(0) == HIERARCHY_SURFACE_KIND_BREADCRUMB
            })
            .expect("breadcrumb event datum rows");

        let path_ids = breadcrumb_rows
            .rows
            .column_by_name(HIERARCHY_PATH_ID_FIELD)
            .expect("path id")
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("path id string");
        assert_eq!(path_ids.value(0), ROOT_PATH_ID);
        assert_eq!(path_ids.value(1), "division=D1");

        let zoomable = breadcrumb_rows
            .rows
            .column_by_name(HIERARCHY_CAN_ZOOM_FIELD)
            .expect("can zoom")
            .as_any()
            .downcast_ref::<BooleanArray>()
            .expect("can zoom bool");
        assert!(!zoomable.value(0));
        assert!(zoomable.value(1));
    }
}
