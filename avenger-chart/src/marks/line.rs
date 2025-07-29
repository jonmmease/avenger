use crate::coords::{Cartesian, CoordinateSystem, Polar};
use crate::error::AvengerChartError;
use crate::marks::util::{
    coerce_bool_channel, coerce_color_channel, coerce_numeric_channel, coerce_stroke_cap_channel,
    coerce_stroke_dash_channel, coerce_stroke_join_channel,
};
use crate::marks::{ChannelType, Mark, MarkState};
use crate::{
    define_common_mark_channels, define_position_mark_channels, impl_mark_common,
    impl_mark_trait_common,
};
use avenger_common::value::ScalarOrArray;
use avenger_scenegraph::marks::line::SceneLineMark;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::dataframe::DataFrame;
use datafusion::scalar::ScalarValue;

pub struct Line<C: CoordinateSystem> {
    state: MarkState<C>,
    __phantom: std::marker::PhantomData<C>,
}

// Generate common methods
impl_mark_common!(Line, "line");

// Define common channels for all coordinate systems
define_common_mark_channels! {
    Line {
        stroke: {
            type: ChannelType::Color,
            default: ScalarValue::Utf8(Some("#000000".to_string())),
            allow_column: true
        },
        stroke_width: {
            type: ChannelType::Size,
            default: ScalarValue::Float32(Some(2.0)),
            allow_column: true
        },
        stroke_dash: {
            type: ChannelType::Enum { values: &["solid", "dashed", "dotted", "dashdot"] },
            default: ScalarValue::Utf8(Some("solid".to_string())),
            allow_column: true
        },
        stroke_cap: {
            type: ChannelType::Enum { values: &["butt", "round", "square"] },
            default: ScalarValue::Utf8(Some("butt".to_string())),
            allow_column: false
        },
        stroke_join: {
            type: ChannelType::Enum { values: &["bevel", "miter", "round"] },
            default: ScalarValue::Utf8(Some("miter".to_string())),
            allow_column: false
        },
        opacity: {
            type: ChannelType::Numeric,
            default: ScalarValue::Float32(Some(1.0)),
            allow_column: false  // Line opacity must be constant
        },
        interpolate: {
            type: ChannelType::Enum { values: &["linear", "step", "step-before", "step-after", "basis", "cardinal", "monotone"] },
            default: ScalarValue::Utf8(Some("linear".to_string())),
            allow_column: false
        },
        defined: {
            type: ChannelType::Numeric,  // Will be coerced to boolean
            default: ScalarValue::Boolean(Some(true))
        },
        order: {
            type: ChannelType::Numeric,
            allow_column: true
        },
    }
}

// Define position channels for Cartesian coordinates
define_position_mark_channels! {
    Line<Cartesian> {
        x: { type: ChannelType::Position },
        y: { type: ChannelType::Position },
    }
}

// Define position channels for Polar coordinates
define_position_mark_channels! {
    Line<Polar> {
        r: { type: ChannelType::Position },
        theta: { type: ChannelType::Position },
    }
}

// Implement Mark trait for Cartesian Line
impl Mark<Cartesian> for Line<Cartesian> {
    impl_mark_trait_common!(Line, Cartesian, "line");

    fn supports_order(&self) -> bool {
        true
    }

    fn partitioning_channels(&self) -> Vec<&'static str> {
        // Partition by visual properties that can vary per line
        vec!["stroke", "stroke_width", "stroke_dash"]
    }

    fn render_from_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        use avenger_common::value::ScalarOrArrayValue;
        use std::collections::HashMap;

        // For lines, we need array data for positions
        let data = data.ok_or_else(|| {
            AvengerChartError::InternalError(
                "Line mark requires array data for x and y positions".to_string(),
            )
        })?;

        let num_rows = data.num_rows();

        // Extract position arrays (x, y) - these must be arrays
        let x = coerce_numeric_channel(Some(data), scalars, "x", 0.0)?;
        let y = coerce_numeric_channel(Some(data), scalars, "y", 0.0)?;

        // Extract defined array (for gaps in the line)
        let defined = coerce_bool_channel(Some(data), scalars, "defined", true)?;

        // Extract style properties - now these can be either scalar or array
        let stroke = coerce_color_channel(Some(data), scalars, "stroke", [0.0, 0.0, 0.0, 1.0])?;
        let stroke_width = coerce_numeric_channel(Some(data), scalars, "stroke_width", 2.0)?;

        // stroke_dash now supports column references
        let stroke_dash = coerce_stroke_dash_channel(Some(data), scalars, "stroke_dash")?;

        // These remain scalar-only
        let stroke_cap =
            coerce_stroke_cap_channel(None, scalars, "stroke_cap", Default::default())?;
        let stroke_join =
            coerce_stroke_join_channel(None, scalars, "stroke_join", Default::default())?;

        // Check if we need to create multiple lines based on varying visual properties
        let has_varying_stroke = matches!(stroke.value(), ScalarOrArrayValue::Array(_));
        let has_varying_width = matches!(stroke_width.value(), ScalarOrArrayValue::Array(_));
        let has_varying_dash = matches!(stroke_dash.value(), ScalarOrArrayValue::Array(_));

        if !has_varying_stroke && !has_varying_width && !has_varying_dash {
            // Simple case: single line with constant properties
            // We know these are either scalars or arrays with all identical values,
            // so first() will give us the right value
            let stroke_color = stroke.first().unwrap().clone();
            let stroke_width_value = *stroke_width.first().unwrap();
            let stroke_dash_value = stroke_dash.first().unwrap().clone();

            let line_mark = SceneLineMark {
                name: "line".to_string(),
                clip: true,
                len: num_rows as u32,
                gradients: vec![],
                x,
                y,
                defined,
                stroke: stroke_color,
                stroke_width: stroke_width_value,
                stroke_cap,
                stroke_join,
                stroke_dash: stroke_dash_value,
                zindex: self.state.zindex,
            };

            return Ok(vec![SceneMark::Line(line_mark)]);
        }

        // Complex case: need to create multiple lines based on unique combinations
        // Check if we have a partition digest column
        let groups = if let Some(digest_column) = data.column_by_name("_partition_digest") {
            // Use the pre-computed digest for grouping
            use datafusion::arrow::array::BinaryArray;

            // The digest function always returns a binary array (MD5 hash)
            let binary_array = digest_column
                .as_any()
                .downcast_ref::<BinaryArray>()
                .ok_or_else(|| {
                    AvengerChartError::InternalError(format!(
                        "Expected _partition_digest to be a binary array, got {:?}",
                        digest_column.data_type()
                    ))
                })?;

            let mut groups: HashMap<Vec<u8>, Vec<usize>> = HashMap::new();
            for i in 0..num_rows {
                let digest = binary_array.value(i);
                groups.entry(digest.to_vec()).or_default().push(i);
            }

            // Convert to hash map with numeric keys for compatibility
            groups
                .into_iter()
                .enumerate()
                .map(|(idx, (_, indices))| (idx as u64, indices))
                .collect()
        } else {
            // Fallback: group all data together if no digest
            let mut groups = HashMap::new();
            groups.insert(0u64, (0..num_rows).collect());
            groups
        };

        // Create a line mark for each group
        let mut scene_marks = Vec::new();

        for (_hash, indices) in groups {
            if indices.is_empty() {
                continue;
            }

            // Extract values for this group
            let first_idx = indices[0];

            // Get the actual values for this group
            let stroke_color = match stroke.value() {
                ScalarOrArrayValue::Scalar(color) => color.clone(),
                ScalarOrArrayValue::Array(colors) => colors.get(first_idx).cloned().unwrap_or(
                    avenger_common::types::ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0]),
                ),
            };

            let stroke_width_value = match stroke_width.value() {
                ScalarOrArrayValue::Scalar(width) => *width,
                ScalarOrArrayValue::Array(widths) => widths.get(first_idx).cloned().unwrap_or(2.0),
            };

            let stroke_dash_value = match stroke_dash.value() {
                ScalarOrArrayValue::Scalar(dash) => dash.clone(),
                ScalarOrArrayValue::Array(dashes) => dashes.get(first_idx).cloned().flatten(),
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
                zindex: self.state.zindex,
            };

            scene_marks.push(SceneMark::Line(line_mark));
        }

        Ok(scene_marks)
    }
}

// Implement Mark trait for Polar Line
impl Mark<Polar> for Line<Polar> {
    impl_mark_trait_common!(Line, Polar, "line");

    fn render_from_data(
        &self,
        _data: Option<&RecordBatch>,
        _scalars: &RecordBatch,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        Err(AvengerChartError::InternalError(
            "Polar line mark rendering not yet implemented".to_string(),
        ))
    }
}
