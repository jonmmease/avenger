use std::sync::Arc;

use avenger_chart_core::{
    AvengerChartError, ChannelDescriptor, CompiledDataContext, CompiledMark, CompiledMarkCore,
    CompiledMarkState, CoordinateSystemTransformCore, LegendRendererKind, LegendRendererSelection,
    Mark, MarkRuntimeContext, ScaleTypePreference, apply_opacity_to_color_channel,
    coerce_color_channel_with_renderer, coerce_numeric_channel_with_renderer,
    coerce_opacity_channel_with_renderer, coerce_stroke_cap_channel_with_renderer,
    coerce_stroke_join_channel_with_renderer, default_scale_type_for_data_type,
    impl_mark_trait_common, is_continuous_scale,
};
use avenger_chart_marks::{PathMark, path_channel_defaults};
use avenger_common::{
    types::{PathTransform, StrokeCap, StrokeJoin},
    value::{ScalarOrArray, ScalarOrArrayValue},
};
use avenger_scales::scales::{ConfiguredScale, ScaleImpl, coerce::Coercer};
use avenger_scenegraph::marks::{mark::SceneMark, path::ScenePathMark};
use datafusion::{
    arrow::array::RecordBatch,
    arrow::datatypes::DataType,
    common::ScalarValue,
    logical_expr::{Expr, lit},
};
use lyon_extra::euclid::Vector2D;
use serde::{Deserialize, Serialize};

use crate::{Cartesian, marks::util};

#[async_trait::async_trait]
impl Mark<Cartesian> for PathMark<Cartesian> {
    impl_mark_trait_common!(PathMark);

    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        _session_context: &datafusion::prelude::SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        Ok(Arc::new(CompiledCartesianPath {
            state: compiled_state,
        }))
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledCartesianPath {
    pub(crate) state: CompiledMarkState,
}

impl CompiledMarkCore for CompiledCartesianPath {
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
        "path"
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
                name: "path",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "path_transform",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
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
                allow_column_ref: false,
            },
            ChannelDescriptor {
                name: "stroke_cap",
                required: false,
                default_value: None,
                allow_column_ref: false,
            },
            ChannelDescriptor {
                name: "stroke_join",
                required: false,
                default_value: None,
                allow_column_ref: false,
            },
            ChannelDescriptor {
                name: "opacity",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
        ]
    }

    fn mark_specific_default(&self, channel: &str) -> Option<ScalarValue> {
        path_channel_defaults(channel)
    }

    fn preferred_scale_type(
        &self,
        channel: &str,
        data_type: &DataType,
    ) -> Option<ScaleTypePreference> {
        if let Some(scale_type) = super::nested_position_scale_type(channel, data_type) {
            return Some(scale_type);
        }

        match (channel, data_type) {
            ("x" | "y", DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View) => {
                Some(ScaleTypePreference::Point)
            }
            ("fill" | "stroke", DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View) => {
                Some(ScaleTypePreference::Ordinal)
            }
            ("path" | "path_transform" | "stroke_cap" | "stroke_join", _) => None,
            _ => default_scale_type_for_data_type(data_type),
        }
    }

    fn preferred_legend_renderer(
        &self,
        channel: &str,
        scale: &ConfiguredScale,
    ) -> Option<LegendRendererSelection> {
        match channel {
            "fill" if is_continuous_scale(scale.scale_impl.as_ref()) => Some(
                LegendRendererSelection::BuiltIn(LegendRendererKind::Colorbar),
            ),
            "fill" | "opacity" => Some(LegendRendererSelection::BuiltIn(LegendRendererKind::Rect)),
            "stroke" if is_continuous_scale(scale.scale_impl.as_ref()) => Some(
                LegendRendererSelection::BuiltIn(LegendRendererKind::Colorbar),
            ),
            "stroke" | "stroke_width" => {
                Some(LegendRendererSelection::BuiltIn(LegendRendererKind::Line))
            }
            "x" | "y" | "path" | "path_transform" | "stroke_cap" | "stroke_join" => None,
            _ => Some(LegendRendererSelection::BuiltIn(LegendRendererKind::Rect)),
        }
    }

    fn default_scale_options(
        &self,
        channel: &str,
        scale_impl: &dyn ScaleImpl,
        _data_type: &DataType,
    ) -> std::collections::HashMap<String, Expr> {
        let mut options = std::collections::HashMap::new();
        if matches!(channel, "fill" | "stroke") && is_continuous_scale(scale_impl) {
            options.insert("nice".to_string(), lit(true));
        }
        options
    }
}

#[typetag::serde]
#[async_trait::async_trait]
impl CompiledMark for CompiledCartesianPath {
    async fn render_from_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &dyn MarkRuntimeContext,
        coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let mark_context = context.core_view();
        let len = util::scene_len(data) as usize;
        let position = util::transform_cartesian_point_channels(
            self, data, scalars, context, coord, "x", "y",
        )?;
        let coercer = Coercer::default();

        let path = if let Some(array) = data.and_then(|data| data.column_by_name("path")) {
            coercer.to_path(array).map_err(|error| {
                AvengerChartError::InternalError(format!("Error coercing channel 'path': {error}"))
            })?
        } else if let Some(array) = scalars.column_by_name("path") {
            coercer
                .to_path(array)
                .map(|values| values.to_scalar_if_len_one())
                .map_err(|error| {
                    AvengerChartError::InternalError(format!(
                        "Error coercing channel 'path': {error}"
                    ))
                })?
        } else {
            ScenePathMark::default().path
        };
        let transform =
            if let Some(array) = data.and_then(|data| data.column_by_name("path_transform")) {
                coercer.to_path_transform(array).map_err(|error| {
                    AvengerChartError::InternalError(format!(
                        "Error coercing channel 'path_transform': {error}"
                    ))
                })?
            } else if let Some(array) = scalars.column_by_name("path_transform") {
                coercer
                    .to_path_transform(array)
                    .map(|values| values.to_scalar_if_len_one())
                    .map_err(|error| {
                        AvengerChartError::InternalError(format!(
                            "Error coercing channel 'path_transform': {error}"
                        ))
                    })?
            } else {
                ScenePathMark::default().transform
            };
        let transform = translate_transforms(transform, &position.x, &position.y, len);

        let fill = coerce_color_channel_with_renderer(
            self,
            data,
            scalars,
            "fill",
            &mark_context,
            [0.0, 0.0, 0.0, 0.0],
        )?;
        let stroke = coerce_color_channel_with_renderer(
            self,
            data,
            scalars,
            "stroke",
            &mark_context,
            [0.0, 0.0, 0.0, 0.0],
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
            None,
            scalars,
            "stroke_width",
            &mark_context,
            0.0,
        )?
        .first()
        .cloned()
        .unwrap_or(0.0);
        let stroke_cap = coerce_stroke_cap_channel_with_renderer(
            self,
            None,
            scalars,
            "stroke_cap",
            &mark_context,
            StrokeCap::Butt,
        )?;
        let stroke_join = coerce_stroke_join_channel_with_renderer(
            self,
            None,
            scalars,
            "stroke_join",
            &mark_context,
            StrokeJoin::Miter,
        )?;

        Ok(vec![SceneMark::Path(ScenePathMark {
            name: "path".to_string(),
            clip: true,
            len: len as u32,
            gradients: vec![],
            stroke_cap,
            stroke_join,
            stroke_width: Some(stroke_width),
            path,
            fill,
            stroke,
            transform,
            indices: None,
            zindex: self.state.zindex,
            interactive: true,
        })])
    }
}

fn translate_transforms(
    transforms: ScalarOrArray<PathTransform>,
    x: &ScalarOrArray<f32>,
    y: &ScalarOrArray<f32>,
    len: usize,
) -> ScalarOrArray<PathTransform> {
    match (transforms.value(), x.value(), y.value()) {
        (
            ScalarOrArrayValue::Scalar(transform),
            ScalarOrArrayValue::Scalar(x),
            ScalarOrArrayValue::Scalar(y),
        ) => ScalarOrArray::new_scalar(transform.then_translate(Vector2D::new(*x, *y))),
        _ => {
            let transforms = transforms.as_vec(len, None);
            let x = x.as_vec(len, None);
            let y = y.as_vec(len, None);
            ScalarOrArray::new_array(
                transforms
                    .iter()
                    .zip(x.iter())
                    .zip(y.iter())
                    .map(|((transform, x), y)| transform.then_translate(Vector2D::new(*x, *y)))
                    .collect(),
            )
        }
    }
}
