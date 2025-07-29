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
            allow_column: false  // Line stroke must be constant
        },
        stroke_width: {
            type: ChannelType::Size,
            default: ScalarValue::Float32(Some(2.0)),
            allow_column: false  // Line width must be constant
        },
        stroke_dash: {
            type: ChannelType::Enum { values: &["solid", "dashed", "dotted", "dashdot"] },
            default: ScalarValue::Utf8(Some("solid".to_string())),
            allow_column: false
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

    fn render_from_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // For lines, we need array data for positions
        let data = data.ok_or_else(|| {
            AvengerChartError::InternalError(
                "Line mark requires array data for x and y positions".to_string(),
            )
        })?;

        let num_rows = data.num_rows() as u32;

        // Extract position arrays (x, y) - these must be arrays
        let x = coerce_numeric_channel(Some(data), scalars, "x", 0.0)?;
        let y = coerce_numeric_channel(Some(data), scalars, "y", 0.0)?;

        // Extract defined array (for gaps in the line)
        let defined = coerce_bool_channel(Some(data), scalars, "defined", true)?;

        // Extract style properties (all scalars for line)
        let stroke = coerce_color_channel(None, scalars, "stroke", [0.0, 0.0, 0.0, 1.0])?;
        let stroke_width = coerce_numeric_channel(None, scalars, "stroke_width", 2.0)?;
        let stroke_cap =
            coerce_stroke_cap_channel(None, scalars, "stroke_cap", Default::default())?;
        let stroke_join =
            coerce_stroke_join_channel(None, scalars, "stroke_join", Default::default())?;
        let stroke_dash = coerce_stroke_dash_channel(None, scalars, "stroke_dash")?;

        // Extract stroke color - must be scalar for lines
        let stroke_color =
            match stroke.value() {
                avenger_common::value::ScalarOrArrayValue::Scalar(color) => color.clone(),
                avenger_common::value::ScalarOrArrayValue::Array(colors) => {
                    // Take first color or default
                    colors.first().cloned().unwrap_or(
                        avenger_common::types::ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0]),
                    )
                }
            };

        // Extract stroke width - must be scalar
        let stroke_width_value = match stroke_width.value() {
            avenger_common::value::ScalarOrArrayValue::Scalar(width) => *width,
            avenger_common::value::ScalarOrArrayValue::Array(widths) => {
                // Take first width or default
                widths.first().cloned().unwrap_or(2.0)
            }
        };

        // Create SceneLineMark
        let line_mark = SceneLineMark {
            name: "line".to_string(),
            clip: true,
            len: num_rows,
            gradients: vec![],
            x,
            y,
            defined,
            stroke: stroke_color,
            stroke_width: stroke_width_value,
            stroke_cap,
            stroke_join,
            stroke_dash,
            zindex: self.state.zindex,
        };

        Ok(vec![SceneMark::Line(line_mark)])
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
