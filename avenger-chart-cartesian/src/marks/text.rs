use std::sync::Arc;

use avenger_chart_core::{
    AvengerChartError, ChannelDescriptor, CompiledDataContext, CompiledMark, CompiledMarkCore,
    CompiledMarkState, CoordinateSystemTransformCore, LegendRendererSelection, Mark,
    MarkRuntimeContext, ScaleTypePreference, apply_opacity_to_color_channel,
    coerce_bool_channel_with_renderer, coerce_color_channel_with_renderer,
    coerce_font_style_channel, coerce_font_weight_channel, coerce_numeric_channel_with_renderer,
    coerce_opacity_channel_with_renderer, coerce_stroke_cap_channel_values_with_renderer,
    coerce_stroke_dash_channel, coerce_stroke_join_channel_values_with_renderer,
    coerce_text_align_channel, coerce_text_baseline_channel, coerce_text_channel,
    default_scale_type_for_data_type, impl_mark_trait_common, is_continuous_scale,
};
use avenger_chart_marks::{Text, text_channel_defaults};
use avenger_common::{
    types::{SceneTextLeaderArrow, SceneTextLeaderShape, StrokeCap, StrokeJoin},
    value::{ScalarOrArray, ScalarOrArrayValue},
};
use avenger_scales::scales::{ConfiguredScale, ScaleImpl};
use avenger_scenegraph::marks::{mark::SceneMark, text::SceneTextMark};
use avenger_text::types::{FontStyle, FontWeight, FontWeightNameSpec, TextAlign, TextBaseline};
use datafusion::{
    arrow::array::RecordBatch,
    arrow::datatypes::DataType,
    common::ScalarValue,
    logical_expr::{Expr, lit},
};
use serde::{Deserialize, Serialize};

use crate::{Cartesian, marks::util};

#[async_trait::async_trait]
impl Mark<Cartesian> for Text<Cartesian> {
    impl_mark_trait_common!(Text);

    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        _session_context: &datafusion::prelude::SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        Ok(Arc::new(CompiledCartesianText {
            state: compiled_state,
        }))
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledCartesianText {
    pub(crate) state: CompiledMarkState,
}

impl CompiledMarkCore for CompiledCartesianText {
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
        "text"
    }

    fn supported_channels(&self) -> Vec<ChannelDescriptor> {
        vec![
            ChannelDescriptor {
                name: "x",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "y",
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
                name: "angle",
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
                name: "font",
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
            ChannelDescriptor {
                name: "limit",
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
                name: "defined",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "dx",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "dy",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "leader",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "leader_stroke",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "leader_stroke_width",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "leader_stroke_dash",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "leader_stroke_cap",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "leader_stroke_join",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "leader_label_padding",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "leader_target_radius",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "leader_min_length",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "leader_shape",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "leader_arrow",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "leader_arrow_length",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "leader_arrow_width",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
        ]
    }

    fn mark_specific_default(&self, channel: &str) -> Option<ScalarValue> {
        text_channel_defaults(channel)
    }

    fn preferred_scale_type(
        &self,
        channel: &str,
        data_type: &DataType,
    ) -> Option<ScaleTypePreference> {
        match (channel, data_type) {
            ("x" | "y", DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View) => {
                Some(ScaleTypePreference::Point)
            }
            ("color", DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View) => {
                Some(ScaleTypePreference::Ordinal)
            }
            ("leader_stroke", DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View) => {
                Some(ScaleTypePreference::Ordinal)
            }
            (
                "text"
                | "font"
                | "align"
                | "baseline"
                | "font_weight"
                | "font_style"
                | "defined"
                | "dx"
                | "dy"
                | "leader"
                | "leader_label_padding"
                | "leader_target_radius"
                | "leader_min_length"
                | "leader_shape"
                | "leader_arrow"
                | "leader_arrow_length"
                | "leader_arrow_width"
                | "leader_stroke_cap"
                | "leader_stroke_join",
                _,
            ) => None,
            _ => default_scale_type_for_data_type(data_type),
        }
    }

    fn preferred_legend_renderer(
        &self,
        _channel: &str,
        _scale: &ConfiguredScale,
    ) -> Option<LegendRendererSelection> {
        None
    }

    fn default_scale_options(
        &self,
        channel: &str,
        scale_impl: &dyn ScaleImpl,
        _data_type: &DataType,
    ) -> std::collections::HashMap<String, Expr> {
        let mut options = std::collections::HashMap::new();
        if matches!(channel, "color" | "leader_stroke") && is_continuous_scale(scale_impl) {
            options.insert("nice".to_string(), lit(true));
        }
        options
    }
}

#[typetag::serde]
#[async_trait::async_trait]
impl CompiledMark for CompiledCartesianText {
    async fn render_from_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &dyn MarkRuntimeContext,
        coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let mark_context = context.core_view();
        let len = util::scene_len(data);
        let position = util::transform_cartesian_point_channels(
            self, data, scalars, context, coord, "x", "y",
        )?;

        let dx =
            coerce_numeric_channel_with_renderer(self, data, scalars, "dx", &mark_context, 0.0)?;
        let dy =
            coerce_numeric_channel_with_renderer(self, data, scalars, "dy", &mark_context, 0.0)?;
        let text = coerce_text_channel(data, scalars, "text", String::new())?;
        let align = coerce_text_align_channel(data, scalars, "align", TextAlign::Left)?;
        let baseline =
            coerce_text_baseline_channel(data, scalars, "baseline", TextBaseline::Alphabetic)?;
        let angle =
            coerce_numeric_channel_with_renderer(self, data, scalars, "angle", &mark_context, 0.0)?;
        let color = coerce_color_channel_with_renderer(
            self,
            data,
            scalars,
            "color",
            &mark_context,
            [0.0, 0.0, 0.0, 1.0],
        )?;
        let font = coerce_text_channel(data, scalars, "font", "sans-serif".to_string())?;
        let font_size = coerce_numeric_channel_with_renderer(
            self,
            data,
            scalars,
            "font_size",
            &mark_context,
            10.0,
        )?;
        let font_weight = coerce_font_weight_channel(
            data,
            scalars,
            "font_weight",
            FontWeight::Name(FontWeightNameSpec::Normal),
        )?;
        let font_style = coerce_font_style_channel(data, scalars, "font_style", FontStyle::Normal)?;
        let limit =
            coerce_numeric_channel_with_renderer(self, data, scalars, "limit", &mark_context, 0.0)?;
        let opacity = coerce_opacity_channel_with_renderer(
            self,
            data,
            scalars,
            "opacity",
            &mark_context,
            1.0,
        )?;
        let color = apply_opacity_to_color_channel(color, &opacity, len as usize);
        let defined =
            coerce_bool_channel_with_renderer(self, data, scalars, "defined", &mark_context, true)?;
        let leader =
            coerce_bool_channel_with_renderer(self, data, scalars, "leader", &mark_context, false)?;
        let leader_stroke = coerce_color_channel_with_renderer(
            self,
            data,
            scalars,
            "leader_stroke",
            &mark_context,
            [0.0, 0.0, 0.0, 0.7],
        )?;
        let leader_stroke = apply_opacity_to_color_channel(leader_stroke, &opacity, len as usize);
        let leader_stroke_width = coerce_numeric_channel_with_renderer(
            self,
            data,
            scalars,
            "leader_stroke_width",
            &mark_context,
            1.0,
        )?;
        let leader_stroke_dash = util::optional_stroke_dash(coerce_stroke_dash_channel(
            data,
            scalars,
            "leader_stroke_dash",
        )?);
        let leader_stroke_cap = coerce_stroke_cap_channel_values_with_renderer(
            self,
            data,
            scalars,
            "leader_stroke_cap",
            &mark_context,
            StrokeCap::Round,
        )?;
        let leader_stroke_join = coerce_stroke_join_channel_values_with_renderer(
            self,
            data,
            scalars,
            "leader_stroke_join",
            &mark_context,
            StrokeJoin::Round,
        )?;
        let leader_label_padding = coerce_numeric_channel_with_renderer(
            self,
            data,
            scalars,
            "leader_label_padding",
            &mark_context,
            2.0,
        )?;
        let leader_target_radius = coerce_numeric_channel_with_renderer(
            self,
            data,
            scalars,
            "leader_target_radius",
            &mark_context,
            0.0,
        )?;
        let leader_min_length = coerce_numeric_channel_with_renderer(
            self,
            data,
            scalars,
            "leader_min_length",
            &mark_context,
            1.0,
        )?;
        let leader_shape = coerce_text_leader_shape_channel(
            data,
            scalars,
            "leader_shape",
            SceneTextLeaderShape::Straight,
        )?;
        let leader_arrow = coerce_text_leader_arrow_channel(
            data,
            scalars,
            "leader_arrow",
            SceneTextLeaderArrow::None,
        )?;
        let leader_arrow_length = coerce_numeric_channel_with_renderer(
            self,
            data,
            scalars,
            "leader_arrow_length",
            &mark_context,
            6.0,
        )?;
        let leader_arrow_width = coerce_numeric_channel_with_renderer(
            self,
            data,
            scalars,
            "leader_arrow_width",
            &mark_context,
            5.0,
        )?;

        Ok(vec![
            SceneTextMark {
                name: "text".to_string(),
                clip: true,
                len,
                text,
                x: position.x,
                y: position.y,
                defined,
                dx,
                dy,
                align,
                baseline,
                angle,
                color,
                font,
                font_size,
                font_weight,
                font_style,
                limit,
                leader,
                leader_stroke,
                leader_stroke_width,
                leader_stroke_cap,
                leader_stroke_join,
                leader_stroke_dash,
                leader_label_padding,
                leader_target_radius,
                leader_min_length,
                leader_shape,
                leader_arrow,
                leader_arrow_length,
                leader_arrow_width,
                indices: None,
                zindex: self.state.zindex,
                interactive: true,
            }
            .into(),
        ])
    }
}

fn coerce_text_leader_shape_channel(
    data: Option<&RecordBatch>,
    scalars: &RecordBatch,
    channel: &str,
    default: SceneTextLeaderShape,
) -> Result<ScalarOrArray<SceneTextLeaderShape>, AvengerChartError> {
    let values = coerce_text_channel(data, scalars, channel, leader_shape_name(default))?;
    match values.value() {
        ScalarOrArrayValue::Scalar(value) => {
            Ok(ScalarOrArray::new_scalar(parse_text_leader_shape(value)?))
        }
        ScalarOrArrayValue::Array(values) => values
            .iter()
            .map(|value| parse_text_leader_shape(value))
            .collect::<Result<Vec<_>, _>>()
            .map(ScalarOrArray::new_array),
    }
}

fn coerce_text_leader_arrow_channel(
    data: Option<&RecordBatch>,
    scalars: &RecordBatch,
    channel: &str,
    default: SceneTextLeaderArrow,
) -> Result<ScalarOrArray<SceneTextLeaderArrow>, AvengerChartError> {
    let values = coerce_text_channel(data, scalars, channel, leader_arrow_name(default))?;
    match values.value() {
        ScalarOrArrayValue::Scalar(value) => {
            Ok(ScalarOrArray::new_scalar(parse_text_leader_arrow(value)?))
        }
        ScalarOrArrayValue::Array(values) => values
            .iter()
            .map(|value| parse_text_leader_arrow(value))
            .collect::<Result<Vec<_>, _>>()
            .map(ScalarOrArray::new_array),
    }
}

fn parse_text_leader_shape(value: &str) -> Result<SceneTextLeaderShape, AvengerChartError> {
    match value.to_ascii_lowercase().as_str() {
        "straight" => Ok(SceneTextLeaderShape::Straight),
        "elbow" => Ok(SceneTextLeaderShape::Elbow),
        "curved" => Ok(SceneTextLeaderShape::Curved),
        other => Err(AvengerChartError::InvalidArgument(format!(
            "Invalid text leader shape '{other}'. Expected one of: straight, elbow, curved"
        ))),
    }
}

fn parse_text_leader_arrow(value: &str) -> Result<SceneTextLeaderArrow, AvengerChartError> {
    match value.to_ascii_lowercase().as_str() {
        "none" => Ok(SceneTextLeaderArrow::None),
        "open" => Ok(SceneTextLeaderArrow::Open),
        "triangle" => Ok(SceneTextLeaderArrow::Triangle),
        other => Err(AvengerChartError::InvalidArgument(format!(
            "Invalid text leader arrow '{other}'. Expected one of: none, open, triangle"
        ))),
    }
}

fn leader_shape_name(value: SceneTextLeaderShape) -> String {
    match value {
        SceneTextLeaderShape::Straight => "straight",
        SceneTextLeaderShape::Elbow => "elbow",
        SceneTextLeaderShape::Curved => "curved",
    }
    .to_string()
}

fn leader_arrow_name(value: SceneTextLeaderArrow) -> String {
    match value {
        SceneTextLeaderArrow::None => "none",
        SceneTextLeaderArrow::Open => "open",
        SceneTextLeaderArrow::Triangle => "triangle",
    }
    .to_string()
}
