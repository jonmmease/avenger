use std::sync::Arc;

use avenger_chart_core::{
    AvengerChartError, ChannelDescriptor, CompiledDataContext, CompiledMark, CompiledMarkCore,
    CompiledMarkState, CoordinateSystemTransformCore, LegendRendererKind, LegendRendererSelection,
    Mark, MarkRuntimeContext, PointGeometry, RadiusExpression, apply_opacity_to_color,
    coerce_bool_channel_with_renderer, coerce_color_channel_with_renderer,
    coerce_numeric_channel_with_renderer, coerce_opacity_channel_with_renderer,
    impl_mark_trait_common, is_continuous_scale, serialization::DefaultLogicalExprNodeExt,
};
use avenger_chart_marks::{Line, PartitionKey, ensure_dictionary_array, line_channel_defaults};
use avenger_common::{types::ColorOrGradient, value::ScalarOrArrayValue};
use avenger_scales::scales::coerce::Coercer;
use avenger_scenegraph::marks::{line::SceneLineMark, mark::SceneMark};
use datafusion::{
    arrow::array::{AsArray, RecordBatch},
    common::ScalarValue,
    logical_expr::{Expr, lit},
};
use datafusion_proto::protobuf::LogicalExprNode;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::Cartesian;

pub use avenger_chart_marks::ensure_dictionary_array as ensure_dictionary_array_fn;

// Implement Mark trait for Cartesian Line with any axis type
#[async_trait::async_trait]
impl Mark<Cartesian> for Line<Cartesian> {
    impl_mark_trait_common!(Line);

    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        _session_context: &datafusion::prelude::SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        Ok(Arc::new(CompiledCartesianLine {
            state: compiled_state,
        }))
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledCartesianLine {
    pub(crate) state: CompiledMarkState,
}

impl CompiledMarkCore for CompiledCartesianLine {
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
        "line"
    }

    fn supported_channels(&self) -> Vec<ChannelDescriptor> {
        vec![
            // Position channels
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
            // Style channels
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
                name: "opacity",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "stroke_cap",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "stroke_join",
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
        line_channel_defaults(channel)
    }

    fn radius_expression(
        &self,
        dimension: &str,
        resolve_channel: &dyn Fn(&str) -> Expr,
    ) -> Option<RadiusExpression> {
        match dimension {
            "y" => {
                let stroke_width_expr = resolve_channel("stroke_width");
                let radius_expr = stroke_width_expr * lit(2.0);
                let radius_expr_node = LogicalExprNode::from_default_expr(radius_expr)
                    .expect("Failed to serialize expr");
                Some(RadiusExpression::Symmetric(radius_expr_node))
            }
            "x" => None,
            _ => None,
        }
    }

    fn preferred_legend_renderer(
        &self,
        channel: &str,
        scale: &avenger_scales::scales::ConfiguredScale,
    ) -> Option<LegendRendererSelection> {
        // Check if scale is continuous (for colorbar)
        let is_continuous = is_continuous_scale(scale.scale_impl.as_ref());

        match channel {
            // Use colorbar for continuous color scales
            "stroke" if is_continuous => Some(LegendRendererSelection::BuiltIn(
                LegendRendererKind::Colorbar,
            )),
            // Line marks use line legend for stroke properties
            "stroke" | "stroke_width" | "stroke_dash" | "opacity" => {
                Some(LegendRendererSelection::BuiltIn(LegendRendererKind::Line))
            }
            // No legend for position channels
            "x" | "y" | "defined" | "order" | "stroke_cap" | "stroke_join" | "interpolate" => None,
            // For any other channel, default to line legend rendering.
            _ => Some(LegendRendererSelection::BuiltIn(LegendRendererKind::Line)),
        }
    }
}

#[typetag::serde]
#[async_trait::async_trait]
impl CompiledMark for CompiledCartesianLine {
    async fn render_from_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &dyn MarkRuntimeContext,
        coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // For lines, we need array data for positions
        let data = data.ok_or_else(|| {
            AvengerChartError::InternalError(
                "Line mark requires array data for x and y positions".to_string(),
            )
        })?;

        let mark_context = context.core_view();
        let len = data.num_rows();
        let coercer = Coercer::default();

        // Extract position channels
        let mut position_channels = std::collections::HashMap::new();
        for channel_name in coord.required_channels() {
            let value = coerce_numeric_channel_with_renderer(
                self,
                Some(data),
                scalars,
                channel_name,
                &mark_context,
                0.0,
            )?;
            position_channels.insert(*channel_name, value);
        }

        // Transform position channels to plot coordinates
        let geometry = coord.transform(
            &position_channels,
            None,
            context.plot_width(),
            context.plot_height(),
        )?;
        let geometry = geometry
            .as_any()
            .downcast_ref::<PointGeometry>()
            .ok_or_else(|| {
                AvengerChartError::CoordinateSystemError(
                    "Failed to downcast to PointGeometry".to_string(),
                )
            })?;

        let x = geometry.x.clone();
        let y = geometry.y.clone();

        // Extract defined array (for gaps in the line)
        let defined = coerce_bool_channel_with_renderer(
            self,
            Some(data),
            scalars,
            "defined",
            &mark_context,
            true,
        )?;

        // Extract style channels - check if they vary or are scalar
        let stroke_array = data.column_by_name("stroke");
        let width_array = data.column_by_name("stroke_width");
        let dash_array = data.column_by_name("stroke_dash");
        let opacity_array = data.column_by_name("opacity");

        let has_varying_stroke = stroke_array.is_some();
        let has_varying_width = width_array.is_some();
        let has_varying_dash = dash_array.is_some();
        let has_varying_opacity = opacity_array.is_some();

        // Extract scalar style properties
        // TODO: Implement proper coercion for stroke_cap and stroke_join when available
        let stroke_cap = self
            .default_channel_value("stroke_cap", &mark_context)
            .and_then(|v| match v {
                ScalarValue::Utf8(Some(s)) => match s.as_str() {
                    "butt" => Some(avenger_common::types::StrokeCap::Butt),
                    "round" => Some(avenger_common::types::StrokeCap::Round),
                    "square" => Some(avenger_common::types::StrokeCap::Square),
                    _ => None,
                },
                _ => None,
            })
            .unwrap_or(avenger_common::types::StrokeCap::Round);

        let stroke_join = self
            .default_channel_value("stroke_join", &mark_context)
            .and_then(|v| match v {
                ScalarValue::Utf8(Some(s)) => match s.as_str() {
                    "miter" => Some(avenger_common::types::StrokeJoin::Miter),
                    "round" => Some(avenger_common::types::StrokeJoin::Round),
                    "bevel" => Some(avenger_common::types::StrokeJoin::Bevel),
                    _ => None,
                },
                _ => None,
            })
            .unwrap_or(avenger_common::types::StrokeJoin::Miter);

        // Simple case: all style properties are uniform
        if !has_varying_stroke && !has_varying_width && !has_varying_dash && !has_varying_opacity {
            let stroke_scalar = coerce_color_channel_with_renderer(
                self,
                None,
                scalars,
                "stroke",
                &mark_context,
                [0.0, 0.0, 0.0, 1.0],
            )?;
            let stroke = match stroke_scalar.value() {
                ScalarOrArrayValue::Scalar(c) => c.clone(),
                ScalarOrArrayValue::Array(arr) => arr[0].clone(),
            };
            let opacity_scalar = coerce_opacity_channel_with_renderer(
                self,
                None,
                scalars,
                "opacity",
                &mark_context,
                1.0,
            )?;
            let opacity = match opacity_scalar.value() {
                ScalarOrArrayValue::Scalar(opacity) => *opacity,
                ScalarOrArrayValue::Array(arr) => arr[0],
            };
            let stroke = apply_opacity_to_color(&stroke, opacity);

            let stroke_width_scalar = coerce_numeric_channel_with_renderer(
                self,
                None,
                scalars,
                "stroke_width",
                &mark_context,
                2.0,
            )?;
            let stroke_width = match stroke_width_scalar.value() {
                ScalarOrArrayValue::Scalar(w) => *w,
                ScalarOrArrayValue::Array(arr) => arr[0],
            };

            let stroke_dash = if let Some(dash_scalar) = scalars.column_by_name("stroke_dash") {
                let dash_vec = coercer
                    .to_stroke_dash(dash_scalar)?
                    .first()
                    .unwrap()
                    .clone();
                if dash_vec.is_empty() {
                    None
                } else {
                    Some(dash_vec)
                }
            } else {
                None
            };

            let line_mark = SceneLineMark {
                name: "line".to_string(),
                clip: true,
                len: len as u32,
                x,
                y,
                gradients: vec![],
                stroke,
                stroke_width,
                stroke_dash,
                stroke_cap,
                stroke_join,
                defined,
                zindex: self.state.zindex,
                interactive: true,
            };

            return Ok(vec![SceneMark::Line(line_mark)]);
        }

        // Complex case: need to partition based on varying style properties
        // Convert to dictionary arrays for efficient partitioning
        let stroke_dict = if has_varying_stroke {
            Some(ensure_dictionary_array(stroke_array.unwrap())?)
        } else {
            None
        };

        let width_dict = if has_varying_width {
            Some(ensure_dictionary_array(width_array.unwrap())?)
        } else {
            None
        };

        let dash_dict = if has_varying_dash {
            Some(ensure_dictionary_array(dash_array.unwrap())?)
        } else {
            None
        };

        let opacity_dict = if has_varying_opacity {
            Some(ensure_dictionary_array(opacity_array.unwrap())?)
        } else {
            None
        };

        // Get dictionary arrays and their keys outside the loop
        let stroke_keys = stroke_dict.as_ref().map(|d| {
            let dict = d.as_any_dictionary();
            (dict, dict.normalized_keys())
        });
        let width_keys = width_dict.as_ref().map(|d| {
            let dict = d.as_any_dictionary();
            (dict, dict.normalized_keys())
        });
        let dash_keys = dash_dict.as_ref().map(|d| {
            let dict = d.as_any_dictionary();
            (dict, dict.normalized_keys())
        });
        let opacity_keys = opacity_dict.as_ref().map(|d| {
            let dict = d.as_any_dictionary();
            (dict, dict.normalized_keys())
        });

        // Coerce unique dictionary values only once
        let stroke_values = if let Some((dict, _)) = &stroke_keys {
            let values = dict.values();
            Some(coercer.to_color(values, Some(ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0])))?)
        } else {
            None
        };

        let width_values = if let Some((dict, _)) = &width_keys {
            let values = dict.values();
            Some(coercer.to_numeric(values, Some(2.0))?)
        } else {
            None
        };

        let dash_values = if let Some((dict, _)) = &dash_keys {
            let values = dict.values();
            Some(coercer.to_stroke_dash(values)?)
        } else {
            None
        };

        let opacity_values = if let Some((dict, _)) = &opacity_keys {
            let values = dict.values();
            Some(
                coercer
                    .to_numeric(values, Some(1.0))?
                    .map(|v| v.clamp(0.0, 1.0)),
            )
        } else {
            None
        };

        // Get defaults for scalar values
        let stroke_default = if let Some(stroke_scalar) = scalars.column_by_name("stroke") {
            coercer
                .to_color(
                    stroke_scalar,
                    Some(ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0])),
                )?
                .first()
                .unwrap()
                .clone()
        } else {
            ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0])
        };

        let width_default = if let Some(width_scalar) = scalars.column_by_name("stroke_width") {
            *coercer
                .to_numeric(width_scalar, Some(2.0))?
                .first()
                .unwrap()
        } else {
            2.0
        };

        let dash_default = if let Some(dash_scalar) = scalars.column_by_name("stroke_dash") {
            let dash_vec = coercer
                .to_stroke_dash(dash_scalar)?
                .first()
                .unwrap()
                .clone();
            if dash_vec.is_empty() {
                None
            } else {
                Some(dash_vec)
            }
        } else {
            None
        };

        let opacity_default = if let Some(opacity_scalar) = scalars.column_by_name("opacity") {
            *coercer
                .to_numeric(opacity_scalar, Some(1.0))?
                .first()
                .unwrap()
        } else {
            1.0
        }
        .clamp(0.0, 1.0);

        // Build partition map
        let mut partition_groups: IndexMap<PartitionKey, Vec<usize>> = IndexMap::new();

        for i in 0..len {
            let key = PartitionKey {
                stroke: stroke_keys.as_ref().and_then(|(dict, keys)| {
                    if dict.is_null(i) { None } else { Some(keys[i]) }
                }),
                width: width_keys.as_ref().and_then(|(dict, keys)| {
                    if dict.is_null(i) { None } else { Some(keys[i]) }
                }),
                dash: dash_keys.as_ref().and_then(|(dict, keys)| {
                    if dict.is_null(i) { None } else { Some(keys[i]) }
                }),
                opacity: opacity_keys.as_ref().and_then(|(dict, keys)| {
                    if dict.is_null(i) { None } else { Some(keys[i]) }
                }),
            };
            partition_groups.entry(key).or_default().push(i);
        }

        // Create a line mark for each partition
        let mut scene_marks = Vec::new();

        for (partition_key, indices) in partition_groups {
            if indices.is_empty() {
                continue;
            }

            // Get the values for this partition
            let stroke_color = if let Some(key) = partition_key.stroke {
                if let Some(values) = &stroke_values {
                    values.as_vec(values.len(), None)[key].clone()
                } else {
                    stroke_default.clone()
                }
            } else {
                stroke_default.clone()
            };
            let opacity_value = if let Some(key) = partition_key.opacity {
                if let Some(values) = &opacity_values {
                    values.as_vec(values.len(), None)[key]
                } else {
                    opacity_default
                }
            } else {
                opacity_default
            };
            let stroke_color = apply_opacity_to_color(&stroke_color, opacity_value);

            let stroke_width_value = if let Some(key) = partition_key.width {
                if let Some(values) = &width_values {
                    values.as_vec(values.len(), None)[key]
                } else {
                    width_default
                }
            } else {
                width_default
            };

            let stroke_dash_value = if let Some(key) = partition_key.dash {
                if let Some(values) = &dash_values {
                    let dash_vec = values.as_vec(values.len(), None)[key].clone();
                    if dash_vec.is_empty() {
                        None
                    } else {
                        Some(dash_vec)
                    }
                } else {
                    dash_default.clone()
                }
            } else {
                dash_default.clone()
            };

            // Extract arrays for just this group's indices
            let mut group_x = Vec::with_capacity(indices.len());
            let mut group_y = Vec::with_capacity(indices.len());
            let mut group_defined = Vec::with_capacity(indices.len());

            // Extract values maintaining order
            match (x.value(), y.value()) {
                (ScalarOrArrayValue::Array(x_arr), ScalarOrArrayValue::Array(y_arr)) => {
                    let defined_default = match defined.value() {
                        ScalarOrArrayValue::Scalar(val) => *val,
                        ScalarOrArrayValue::Array(_) => true,
                    };

                    for &idx in &indices {
                        if let (Some(&x_val), Some(&y_val)) = (x_arr.get(idx), y_arr.get(idx)) {
                            group_x.push(x_val);
                            group_y.push(y_val);

                            let def_val = match defined.value() {
                                ScalarOrArrayValue::Scalar(val) => *val,
                                ScalarOrArrayValue::Array(arr) => {
                                    arr.get(idx).cloned().unwrap_or(defined_default)
                                }
                            };
                            group_defined.push(def_val);
                        }
                    }
                }
                _ => {
                    return Err(AvengerChartError::InternalError(
                        "Line positions must be arrays".to_string(),
                    ));
                }
            }

            let line_mark = SceneLineMark {
                name: "line".to_string(),
                clip: true,
                len: indices.len() as u32,
                x: avenger_common::value::ScalarOrArray::from(group_x),
                y: avenger_common::value::ScalarOrArray::from(group_y),
                gradients: vec![],
                stroke: stroke_color,
                stroke_width: stroke_width_value,
                stroke_dash: stroke_dash_value,
                stroke_cap,
                stroke_join,
                defined: avenger_common::value::ScalarOrArray::from(group_defined),
                zindex: self.state.zindex,
                interactive: true,
            };

            scene_marks.push(SceneMark::Line(line_mark));
        }

        Ok(scene_marks)
    }
}
