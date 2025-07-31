use crate::coords::{Cartesian, CoordinateSystem, Polar};
use crate::error::AvengerChartError;
use crate::marks::util::{
    coerce_bool_channel, coerce_numeric_channel, coerce_stroke_cap_channel,
    coerce_stroke_join_channel,
};
use crate::marks::{ChannelType, Mark, MarkState};
use crate::{
    define_common_mark_channels, define_position_mark_channels, impl_mark_common,
    impl_mark_trait_common,
};
use avenger_common::value::ScalarOrArray;
use avenger_scenegraph::marks::line::SceneLineMark;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::arrow::array::{Array, ArrayRef, AsArray};
use datafusion::arrow::compute::kernels::cast::cast;
use datafusion::arrow::datatypes::DataType;
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::dataframe::DataFrame;
use datafusion::scalar::ScalarValue;
use indexmap::IndexMap;

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

// Partitioning support for multi-series lines
#[derive(Hash, Eq, PartialEq, Debug, Clone, Ord, PartialOrd)]
struct PartitionKey {
    stroke: Option<usize>,
    width: Option<usize>,
    dash: Option<usize>,
}

/// Convert an array to dictionary encoding for efficient partitioning
fn ensure_dictionary_array(array: &ArrayRef) -> Result<ArrayRef, AvengerChartError> {
    match array.data_type() {
        DataType::Dictionary(_, _) => Ok(array.clone()),
        _ => {
            // Convert to dictionary for efficient partitioning
            let dict_type = DataType::Dictionary(
                Box::new(DataType::Int16),
                Box::new(array.data_type().clone()),
            );
            Ok(cast(array, &dict_type)?)
        }
    }
}

// Implement Mark trait for Cartesian Line
impl Mark<Cartesian> for Line<Cartesian> {
    impl_mark_trait_common!(Line, Cartesian, "line");

    fn supports_order(&self) -> bool {
        true
    }

    fn render_from_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
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
        let x = coerce_numeric_channel(Some(data), scalars, "x", 0.0)?;
        let y = coerce_numeric_channel(Some(data), scalars, "y", 0.0)?;

        // Extract defined array (for gaps in the line)
        let defined = coerce_bool_channel(Some(data), scalars, "defined", true)?;

        // These remain scalar-only
        let stroke_cap =
            coerce_stroke_cap_channel(None, scalars, "stroke_cap", Default::default())?;
        let stroke_join =
            coerce_stroke_join_channel(None, scalars, "stroke_join", Default::default())?;

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
                zindex: self.state.zindex,
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
