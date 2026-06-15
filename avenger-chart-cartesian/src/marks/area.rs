use std::sync::Arc;

use avenger_chart_core::{
    AvengerChartError, ChannelDescriptor, CompiledDataContext, CompiledMark, CompiledMarkCore,
    CompiledMarkState, CoordinateSystemTransformCore, LegendRendererKind, LegendRendererSelection,
    Mark, MarkRuntimeContext, ScaleTypePreference, apply_opacity_to_color,
    coerce_area_orientation_channel, coerce_bool_channel_with_renderer,
    coerce_color_channel_with_renderer, coerce_numeric_channel_with_renderer,
    coerce_opacity_channel_with_renderer, coerce_stroke_cap_channel_with_renderer,
    coerce_stroke_dash_channel, coerce_stroke_join_channel_with_renderer,
    default_scale_type_for_data_type, impl_mark_trait_common, is_continuous_scale,
};
use avenger_chart_marks::{Area, AreaPartitionKey, area_channel_defaults, ensure_dictionary_array};
use avenger_color::ColorOrGradient;
use avenger_common::{
    types::{AreaOrientation, StrokeCap, StrokeJoin},
    value::ScalarOrArrayValue,
};
use avenger_scales::scales::{ConfiguredScale, ScaleImpl, coerce::Coercer};
use avenger_scenegraph::marks::{area::SceneAreaMark, mark::SceneMark};
use datafusion::{
    arrow::array::{AsArray, RecordBatch},
    arrow::datatypes::DataType,
    common::ScalarValue,
    logical_expr::{Expr, lit},
};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::{Cartesian, marks::util};

#[async_trait::async_trait]
impl Mark<Cartesian> for Area<Cartesian> {
    impl_mark_trait_common!(Area);

    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        _session_context: &datafusion::prelude::SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        Ok(Arc::new(CompiledCartesianArea {
            state: compiled_state,
        }))
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledCartesianArea {
    pub(crate) state: CompiledMarkState,
}

impl CompiledMarkCore for CompiledCartesianArea {
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
        "area"
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
                name: "x2",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "y2",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "orientation",
                required: false,
                default_value: None,
                allow_column_ref: false,
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
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "stroke_dash",
                required: false,
                default_value: None,
                allow_column_ref: true,
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
            ChannelDescriptor {
                name: "defined",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "order",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
        ]
    }

    fn supports_order(&self) -> bool {
        true
    }

    fn mark_specific_default(&self, channel: &str) -> Option<ScalarValue> {
        area_channel_defaults(channel)
    }

    fn preferred_scale_type(
        &self,
        channel: &str,
        data_type: &DataType,
    ) -> Option<ScaleTypePreference> {
        match (channel, data_type) {
            (
                "x" | "x2" | "y" | "y2",
                DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View,
            ) => Some(ScaleTypePreference::Point),
            ("fill" | "stroke", DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View) => {
                Some(ScaleTypePreference::Ordinal)
            }
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
            "stroke" | "stroke_width" | "stroke_dash" => {
                Some(LegendRendererSelection::BuiltIn(LegendRendererKind::Line))
            }
            "x" | "y" | "x2" | "y2" | "defined" | "order" | "orientation" | "stroke_cap"
            | "stroke_join" => None,
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
impl CompiledMark for CompiledCartesianArea {
    async fn render_from_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &dyn MarkRuntimeContext,
        coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let data = data.ok_or_else(|| {
            AvengerChartError::InternalError(
                "Area mark requires array data for x and y positions".to_string(),
            )
        })?;
        let mark_context = context.core_view();
        let len = data.num_rows();
        let coercer = Coercer::default();

        let start = util::transform_cartesian_point_channels(
            self,
            Some(data),
            scalars,
            context,
            coord,
            "x",
            "y",
        )?;
        let end = util::transform_cartesian_point_channels(
            self,
            Some(data),
            scalars,
            context,
            coord,
            "x2",
            "y2",
        )?;
        let defined = coerce_bool_channel_with_renderer(
            self,
            Some(data),
            scalars,
            "defined",
            &mark_context,
            true,
        )?;

        let orientation = coerce_area_orientation_channel(
            None,
            scalars,
            "orientation",
            AreaOrientation::Vertical,
        )?
        .first()
        .cloned()
        .unwrap_or(AreaOrientation::Vertical);
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
            StrokeJoin::Round,
        )?;

        let fill_array = data.column_by_name("fill");
        let stroke_array = data.column_by_name("stroke");
        let width_array = data.column_by_name("stroke_width");
        let dash_array = data.column_by_name("stroke_dash");
        let opacity_array = data.column_by_name("opacity");
        let has_varying_style = fill_array.is_some()
            || stroke_array.is_some()
            || width_array.is_some()
            || dash_array.is_some()
            || opacity_array.is_some();

        if !has_varying_style {
            let fill = coerce_color_channel_with_renderer(
                self,
                None,
                scalars,
                "fill",
                &mark_context,
                [70.0 / 255.0, 130.0 / 255.0, 180.0 / 255.0, 1.0],
            )?;
            let stroke = coerce_color_channel_with_renderer(
                self,
                None,
                scalars,
                "stroke",
                &mark_context,
                [0.0, 0.0, 0.0, 0.0],
            )?;
            let opacity = coerce_opacity_channel_with_renderer(
                self,
                None,
                scalars,
                "opacity",
                &mark_context,
                1.0,
            )?
            .first()
            .cloned()
            .unwrap_or(1.0);
            let fill = apply_opacity_to_color(&first_color(fill), opacity);
            let stroke = apply_opacity_to_color(&first_color(stroke), opacity);
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
            let stroke_dash = util::optional_stroke_dash(coerce_stroke_dash_channel(
                None,
                scalars,
                "stroke_dash",
            )?)
            .map(|dash| dash.first().cloned().unwrap_or_default());

            return Ok(vec![
                SceneAreaMark {
                    name: "area".to_string(),
                    clip: true,
                    len: len as u32,
                    orientation,
                    gradients: vec![],
                    x: start.x,
                    y: start.y,
                    x2: end.x,
                    y2: end.y,
                    defined,
                    fill,
                    stroke,
                    stroke_width,
                    stroke_cap,
                    stroke_join,
                    stroke_dash,
                    zindex: self.state.zindex,
                    interactive: true,
                }
                .into(),
            ]);
        }

        let fill_keys = fill_array
            .map(ensure_dictionary_array)
            .transpose()?
            .map(|d| {
                let dict = d.as_any_dictionary();
                (d.clone(), dict.normalized_keys())
            });
        let stroke_keys = stroke_array
            .map(ensure_dictionary_array)
            .transpose()?
            .map(|d| {
                let dict = d.as_any_dictionary();
                (d.clone(), dict.normalized_keys())
            });
        let width_keys = width_array
            .map(ensure_dictionary_array)
            .transpose()?
            .map(|d| {
                let dict = d.as_any_dictionary();
                (d.clone(), dict.normalized_keys())
            });
        let dash_keys = dash_array
            .map(ensure_dictionary_array)
            .transpose()?
            .map(|d| {
                let dict = d.as_any_dictionary();
                (d.clone(), dict.normalized_keys())
            });
        let opacity_keys = opacity_array
            .map(ensure_dictionary_array)
            .transpose()?
            .map(|d| {
                let dict = d.as_any_dictionary();
                (d.clone(), dict.normalized_keys())
            });

        let fill_values = if let Some((dict, _)) = &fill_keys {
            Some(coercer.to_color(
                dict.as_any_dictionary().values(),
                Some(ColorOrGradient::Color([
                    70.0 / 255.0,
                    130.0 / 255.0,
                    180.0 / 255.0,
                    1.0,
                ])),
            )?)
        } else {
            None
        };
        let stroke_values = if let Some((dict, _)) = &stroke_keys {
            Some(coercer.to_color(
                dict.as_any_dictionary().values(),
                Some(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])),
            )?)
        } else {
            None
        };
        let width_values = if let Some((dict, _)) = &width_keys {
            Some(coercer.to_numeric(dict.as_any_dictionary().values(), Some(0.0))?)
        } else {
            None
        };
        let dash_values = if let Some((dict, _)) = &dash_keys {
            Some(coercer.to_stroke_dash(dict.as_any_dictionary().values())?)
        } else {
            None
        };
        let opacity_values = if let Some((dict, _)) = &opacity_keys {
            Some(
                coercer
                    .to_numeric(dict.as_any_dictionary().values(), Some(1.0))?
                    .map(|v| v.clamp(0.0, 1.0)),
            )
        } else {
            None
        };

        let fill_default = first_color(coerce_color_channel_with_renderer(
            self,
            None,
            scalars,
            "fill",
            &mark_context,
            [70.0 / 255.0, 130.0 / 255.0, 180.0 / 255.0, 1.0],
        )?);
        let stroke_default = first_color(coerce_color_channel_with_renderer(
            self,
            None,
            scalars,
            "stroke",
            &mark_context,
            [0.0, 0.0, 0.0, 0.0],
        )?);
        let width_default = coerce_numeric_channel_with_renderer(
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
        let dash_default =
            util::optional_stroke_dash(coerce_stroke_dash_channel(None, scalars, "stroke_dash")?)
                .map(|dash| dash.first().cloned().unwrap_or_default());
        let opacity_default = coerce_opacity_channel_with_renderer(
            self,
            None,
            scalars,
            "opacity",
            &mark_context,
            1.0,
        )?
        .first()
        .cloned()
        .unwrap_or(1.0);

        let mut groups: IndexMap<AreaPartitionKey, Vec<usize>> = IndexMap::new();
        for i in 0..len {
            let key = AreaPartitionKey {
                fill: dictionary_key(&fill_keys, i),
                stroke: dictionary_key(&stroke_keys, i),
                width: dictionary_key(&width_keys, i),
                dash: dictionary_key(&dash_keys, i),
                opacity: dictionary_key(&opacity_keys, i),
            };
            groups.entry(key).or_default().push(i);
        }

        let mut marks = Vec::new();
        for (key, indices) in groups {
            let opacity = key
                .opacity
                .and_then(|key| {
                    opacity_values
                        .as_ref()
                        .map(|values| values.as_vec(values.len(), None)[key])
                })
                .unwrap_or(opacity_default);
            let fill = key
                .fill
                .and_then(|key| {
                    fill_values
                        .as_ref()
                        .map(|values| values.as_vec(values.len(), None)[key].clone())
                })
                .unwrap_or_else(|| fill_default.clone());
            let stroke = key
                .stroke
                .and_then(|key| {
                    stroke_values
                        .as_ref()
                        .map(|values| values.as_vec(values.len(), None)[key].clone())
                })
                .unwrap_or_else(|| stroke_default.clone());
            let stroke_width = key
                .width
                .and_then(|key| {
                    width_values
                        .as_ref()
                        .map(|values| values.as_vec(values.len(), None)[key])
                })
                .unwrap_or(width_default);
            let stroke_dash = key
                .dash
                .and_then(|key| {
                    dash_values
                        .as_ref()
                        .map(|values| values.as_vec(values.len(), None)[key].clone())
                })
                .and_then(|dash| if dash.is_empty() { None } else { Some(dash) })
                .or_else(|| dash_default.clone());

            marks.push(SceneMark::Area(SceneAreaMark {
                name: "area".to_string(),
                clip: true,
                len: indices.len() as u32,
                orientation,
                gradients: vec![],
                x: util::gather_by_indices(&start.x, len, &indices),
                y: util::gather_by_indices(&start.y, len, &indices),
                x2: util::gather_by_indices(&end.x, len, &indices),
                y2: util::gather_by_indices(&end.y, len, &indices),
                defined: util::gather_by_indices(&defined, len, &indices),
                fill: apply_opacity_to_color(&fill, opacity),
                stroke: apply_opacity_to_color(&stroke, opacity),
                stroke_width,
                stroke_cap,
                stroke_join,
                stroke_dash,
                zindex: self.state.zindex,
                interactive: true,
            }));
        }

        Ok(marks)
    }
}

fn first_color(colors: avenger_common::value::ScalarOrArray<ColorOrGradient>) -> ColorOrGradient {
    match colors.value() {
        ScalarOrArrayValue::Scalar(color) => color.clone(),
        ScalarOrArrayValue::Array(colors) => colors
            .first()
            .cloned()
            .unwrap_or_else(ColorOrGradient::transparent),
    }
}

fn dictionary_key(
    keys: &Option<(datafusion::arrow::array::ArrayRef, Vec<usize>)>,
    index: usize,
) -> Option<usize> {
    keys.as_ref().and_then(|(array, keys)| {
        let dict = array.as_any_dictionary();
        if dict.is_null(index) {
            None
        } else {
            Some(keys[index])
        }
    })
}
