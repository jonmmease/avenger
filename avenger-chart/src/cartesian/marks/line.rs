use crate::cartesian::Cartesian;
use crate::define_position_channels;
use crate::impl_mark_trait_common;
use crate::marks::{Mark, RadiusExpression};
use arrow::array::{AsArray, RecordBatch};
use avenger_common::value::ScalarOrArray;
use avenger_scales::scales::{ScaleImpl, ordinal::OrdinalScale, point::PointScale};
use avenger_scenegraph::marks::line::SceneLineMark;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::arrow::datatypes::DataType;
use datafusion::logical_expr::{Expr, lit};
use datafusion_common::ScalarValue;
use indexmap::IndexMap;
// Import Line for the macro, then re-export it
use crate::error::AvengerChartError;
pub use crate::marks::line::{Line, ensure_dictionary_array};
use crate::marks::util::{
    coerce_bool_channel_with_mark, coerce_numeric_channel_with_mark,
    coerce_stroke_cap_channel_with_mark, coerce_stroke_join_channel_with_mark,
};
use crate::render_context::RenderContext;
use crate::scales::ScaleRange;
use std::sync::Arc;

// Define position channels for Cartesian Line using the macro
define_position_channels! {
    Line<Cartesian> {
        x: {
            with_config: crate::cartesian::channels::CartesianPositionConfig,
        },
        y: {
            with_config: crate::cartesian::channels::CartesianPositionConfig,
        }
    }
}

// Implement Mark trait for Cartesian Line with any axis type
impl Mark<Cartesian> for Line<Cartesian> {
    impl_mark_trait_common!(Line, Cartesian, "line");

    fn supports_order(&self) -> bool {
        true
    }

    fn mark_specific_default(&self, channel: &str) -> Option<ScalarValue> {
        match channel {
            "stroke" => Some(ScalarValue::Utf8(Some("#000000".to_string()))), // Default black
            "stroke_width" => Some(ScalarValue::Float32(Some(2.0))),          // Default line width
            "stroke_cap" => Some(ScalarValue::Utf8(Some("round".to_string()))), // Default cap style
            "stroke_join" => Some(ScalarValue::Utf8(Some("round".to_string()))), // Default join style
            "opacity" => Some(ScalarValue::Float32(Some(1.0))),                  // Fully opaque
            "interpolate" => Some(ScalarValue::Utf8(Some("linear".to_string()))), // Linear interpolation
            "defined" => Some(ScalarValue::Boolean(Some(true))), // All points defined
            _ => None,
        }
    }

    fn radius_expression(
        &self,
        dimension: &str,
        resolve_channel: &dyn Fn(&str) -> Expr,
    ) -> Option<RadiusExpression> {
        match dimension {
            "y" => {
                // Get stroke_width expression (either mapped or default)
                let stroke_width_expr = resolve_channel("stroke_width");

                // For lines: vertical radius = stroke_width * 2
                // This ensures the full line thickness is visible even at plot boundaries
                let radius_expr = stroke_width_expr * lit(2.0);

                Some(RadiusExpression::Symmetric(radius_expr))
            }
            "x" => {
                // No horizontal radius for line marks
                None
            }
            _ => None,
        }
    }

    fn render_from_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &RenderContext,
        _coord: &Cartesian,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        use avenger_common::value::ScalarOrArrayValue;
        use avenger_scales::scales::coerce::Coercer;

        // For lines, we need array data for positions
        let data = data.ok_or_else(|| {
            AvengerChartError::InternalError(
                "Line mark requires array data for x and y positions".to_string(),
            )
        })?;

        let len = data.num_rows();
        let coercer = Coercer::default();

        // Extract position arrays (x, y) - these must be arrays
        let x = coerce_numeric_channel_with_mark(self, Some(data), scalars, "x", context, 0.0)?;
        let y = coerce_numeric_channel_with_mark(self, Some(data), scalars, "y", context, 0.0)?;

        // Extract defined array (for gaps in the line)
        let defined =
            coerce_bool_channel_with_mark(self, Some(data), scalars, "defined", context, true)?;

        // These remain scalar-only - use mark defaults
        let stroke_cap = coerce_stroke_cap_channel_with_mark(
            self,
            None,
            scalars,
            "stroke_cap",
            context,
            avenger_common::types::StrokeCap::Round,
        )?;
        let stroke_join = coerce_stroke_join_channel_with_mark(
            self,
            None,
            scalars,
            "stroke_join",
            context,
            avenger_common::types::StrokeJoin::Round,
        )?;

        // Check which style properties vary
        let stroke_array = data.column_by_name("stroke");
        let width_array = data.column_by_name("stroke_width");
        let dash_array = data.column_by_name("stroke_dash");

        let has_varying_stroke = stroke_array.is_some();
        let has_varying_width = width_array.is_some();
        let has_varying_dash = dash_array.is_some();

        if !has_varying_stroke && !has_varying_width && !has_varying_dash {
            // Simple case: single line with constant properties
            // Coerce scalar values only
            let stroke_color = if let Some(stroke_scalar) = scalars.column_by_name("stroke") {
                coercer
                    .to_color(
                        stroke_scalar,
                        Some(avenger_common::types::ColorOrGradient::Color([
                            0.0, 0.0, 0.0, 1.0,
                        ])),
                    )?
                    .first()
                    .unwrap()
                    .clone()
            } else {
                avenger_common::types::ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0])
            };

            let stroke_width_value =
                if let Some(width_scalar) = scalars.column_by_name("stroke_width") {
                    *coercer
                        .to_numeric(width_scalar, Some(2.0))?
                        .first()
                        .unwrap()
                } else {
                    2.0
                };

            let dash_scalar = scalars.column_by_name("stroke_dash");
            let stroke_dash_value = if let Some(dash) = dash_scalar {
                let dash_vec = coercer.to_stroke_dash(dash)?.first().unwrap().clone();
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
                gradients: vec![],
                x,
                y,
                defined,
                stroke: stroke_color,
                stroke_width: stroke_width_value,
                stroke_cap,
                stroke_join,
                stroke_dash: stroke_dash_value,
                zindex: self.get_zindex(),
            };

            return Ok(vec![SceneMark::Line(line_mark)]);
        }

        // Complex case: need to create multiple lines based on unique combinations
        // Convert varying channels to dictionary arrays for efficient partitioning
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

        // Coerce unique dictionary values only once
        let stroke_values = if let Some((dict, _)) = &stroke_keys {
            let values = dict.values();
            Some(coercer.to_color(
                values,
                Some(avenger_common::types::ColorOrGradient::Color([
                    0.0, 0.0, 0.0, 1.0,
                ])),
            )?)
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

        // Get scalar defaults
        let stroke_default = if let Some(stroke_scalar) = scalars.column_by_name("stroke") {
            coercer
                .to_color(
                    stroke_scalar,
                    Some(avenger_common::types::ColorOrGradient::Color([
                        0.0, 0.0, 0.0, 1.0,
                    ])),
                )?
                .first()
                .unwrap()
                .clone()
        } else {
            avenger_common::types::ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0])
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

        // Build partition map using dictionary keys
        let mut partition_groups: IndexMap<crate::marks::line::PartitionKey, Vec<usize>> =
            IndexMap::new();

        for i in 0..len {
            let key = crate::marks::line::PartitionKey {
                stroke: stroke_keys.as_ref().and_then(|(dict, keys)| {
                    if dict.is_null(i) { None } else { Some(keys[i]) }
                }),
                width: width_keys.as_ref().and_then(|(dict, keys)| {
                    if dict.is_null(i) { None } else { Some(keys[i]) }
                }),
                dash: dash_keys.as_ref().and_then(|(dict, keys)| {
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

            // Get the values for this partition using the partition key
            let stroke_color = if let Some(key) = partition_key.stroke {
                if let Some(values) = &stroke_values {
                    values.as_vec(values.len(), None)[key].clone()
                } else {
                    stroke_default.clone()
                }
            } else {
                stroke_default.clone()
            };

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
                    // Handle defined - it might be scalar or array
                    let defined_default = match defined.value() {
                        ScalarOrArrayValue::Scalar(val) => *val,
                        ScalarOrArrayValue::Array(_) => true,
                    };

                    for &idx in &indices {
                        if let (Some(&x_val), Some(&y_val)) = (x_arr.get(idx), y_arr.get(idx)) {
                            group_x.push(x_val);
                            group_y.push(y_val);

                            // Get defined value for this index
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

            // Create ScalarOrArray values for this group
            let group_x_scalar = ScalarOrArray::from(group_x);
            let group_y_scalar = ScalarOrArray::from(group_y);
            let group_defined_scalar = ScalarOrArray::from(group_defined);

            let line_mark = SceneLineMark {
                name: "line".to_string(),
                clip: true,
                len: indices.len() as u32,
                gradients: vec![],
                x: group_x_scalar,
                y: group_y_scalar,
                defined: group_defined_scalar,
                stroke: stroke_color,
                stroke_width: stroke_width_value,
                stroke_cap,
                stroke_join,
                stroke_dash: stroke_dash_value,
                zindex: self.get_zindex(),
            };

            scene_marks.push(SceneMark::Line(line_mark));
        }

        Ok(scene_marks)
    }

    fn preferred_scale_type(
        &self,
        channel: &str,
        data_type: &DataType,
    ) -> Option<Arc<dyn ScaleImpl>> {
        match (channel, data_type) {
            // Line marks use point scales for categorical position data
            ("x" | "y", DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View) => {
                Some(Arc::new(PointScale))
            }
            // Stroke color uses ordinal scales for categorical data
            ("stroke", DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View) => {
                Some(Arc::new(OrdinalScale))
            }
            // Stroke dash uses ordinal for categorical data
            ("stroke_dash", DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View) => {
                Some(Arc::new(OrdinalScale))
            }
            // Stroke width always uses ordinal scale for discrete mapping
            ("stroke_width", _) => Some(Arc::new(OrdinalScale)),
            // Fall back to data type-based inference for other channels
            _ => crate::marks::default_scale_for_data_type(data_type),
        }
    }

    fn default_channel_range(
        &self,
        channel: &str,
        scale_impl: &dyn ScaleImpl,
        _domain: &crate::scales::ResolvedDomain,
        _data_type: &DataType,
        theme: &dyn crate::theme::Theme,
    ) -> Option<ScaleRange> {
        use avenger_scales::scales::DomainKind;

        match channel {
            "opacity" => Some(ScaleRange::new_interval(lit(0.0), lit(1.0))),
            "stroke_width" => {
                // Use discrete range for categorical domains
                if scale_impl.domain_kind() == DomainKind::Categorical {
                    let widths: Vec<f32> = (1..=5).map(|i| i as f32).collect();
                    Some(ScaleRange::new_discrete(widths))
                } else {
                    Some(ScaleRange::new_interval(lit(0.5), lit(5.0)))
                }
            }
            "stroke_dash" => {
                // Only provide dash patterns for categorical domains
                if scale_impl.domain_kind() == DomainKind::Categorical {
                    // Use theme dash patterns
                    Some(theme.get_dash_range(None))
                } else {
                    None
                }
            }
            "stroke" => {
                // Use theme color system
                let range_kind = scale_impl.range_kind();
                Some(theme.get_range_for_channel("line", channel, range_kind, None))
            }
            _ => None,
        }
    }

    fn preferred_legend_renderer(
        &self,
        channel: &str,
        scale: &avenger_scales::scales::ConfiguredScale,
    ) -> Option<std::sync::Arc<dyn crate::legend::LegendRenderer>> {
        use crate::legend::{ColorbarRenderer, LineLegendRenderer};
        use crate::marks::util::is_continuous_scale;
        use std::sync::Arc;

        // Check if scale is continuous (for colorbar)
        let is_continuous = is_continuous_scale(scale.scale_impl.as_ref());

        match channel {
            // Use colorbar for continuous color scales
            "stroke" if is_continuous => Some(Arc::new(ColorbarRenderer::new())),
            // Line marks use line legend for stroke properties
            "stroke" | "stroke_width" | "stroke_dash" => Some(Arc::new(LineLegendRenderer::new())),
            // No legend for position channels and other non-visual channels
            "x" | "y" | "x2" | "y2" | "defined" | "order" => None,
            // For other channels like opacity, use line legend
            _ => Some(Arc::new(LineLegendRenderer::new())),
        }
    }
}
