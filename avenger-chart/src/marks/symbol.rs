use crate::coords::{Cartesian, CoordinateSystem, Polar};
use crate::error::AvengerChartError;
use crate::marks::util::{coerce_color_channel, coerce_numeric_channel};
use crate::marks::{ChannelType, Mark, MarkState};
use crate::{
    define_common_mark_channels, define_position_mark_channels, impl_mark_common,
    impl_mark_trait_common,
};
use avenger_common::value::ScalarOrArray;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::symbol::SceneSymbolMark;
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::dataframe::DataFrame;
use datafusion::scalar::ScalarValue;

pub struct Symbol<C: CoordinateSystem> {
    state: MarkState<C>,
    __phantom: std::marker::PhantomData<C>,
}

// Generate common methods
impl_mark_common!(Symbol, "symbol");

// Define common channels for all coordinate systems
define_common_mark_channels! {
    Symbol {
        size: {
            type: ChannelType::Size,
            default: ScalarValue::Float32(Some(64.0))  // Default area
        },
        fill: {
            type: ChannelType::Color,
            default: ScalarValue::Utf8(Some("#4682b4".to_string()))
        },
        stroke: {
            type: ChannelType::Color,
            default: ScalarValue::Utf8(Some("#000000".to_string()))
        },
        stroke_width: {
            type: ChannelType::Size,
            default: ScalarValue::Float32(Some(1.0)),
            allow_column: false  // Symbol stroke width must be constant
        },
        shape: {
            type: ChannelType::Enum { values: &["circle", "square", "cross", "diamond", "triangle-up", "triangle-down", "triangle-left", "triangle-right"] },
            default: ScalarValue::Utf8(Some("circle".to_string()))
        },
        angle: {
            type: ChannelType::Numeric,
            default: ScalarValue::Float32(Some(0.0))  // Default: no rotation
        },
    }
}

// Define position channels for Cartesian coordinates
define_position_mark_channels! {
    Symbol<Cartesian> {
        x: { type: ChannelType::Position },
        y: { type: ChannelType::Position },
    }
}

// Define position channels for Polar coordinates
define_position_mark_channels! {
    Symbol<Polar> {
        r: { type: ChannelType::Position },
        theta: { type: ChannelType::Position },
    }
}

// Implement Mark trait for Cartesian Symbol
impl Mark<Cartesian> for Symbol<Cartesian> {
    impl_mark_trait_common!(Symbol, Cartesian, "symbol");

    fn render_from_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        use avenger_common::value::ScalarOrArrayValue;
        use avenger_scales::scales::coerce::Coercer;

        // Symbols can render with just scalar data
        let coercer = Coercer::default();
        
        // Extract position data - these can be scalars or arrays
        let x = coerce_numeric_channel(data, scalars, "x", 0.0)?;
        let y = coerce_numeric_channel(data, scalars, "y", 0.0)?;
        
        // Determine the number of symbols
        let len = match (x.value(), y.value()) {
            (ScalarOrArrayValue::Array(x_arr), _) => x_arr.len(),
            (_, ScalarOrArrayValue::Array(y_arr)) => y_arr.len(),
            _ => 1, // Both scalars, render single symbol
        };

        // Handle shape channel efficiently
        let (shapes, shape_index) = if let Some(shape_array) = data.and_then(|d| d.column_by_name("shape")) {
            // Array data for shapes - use efficient coercion
            coercer.to_symbol_shape(shape_array, None)?
        } else if let Some(shape_scalar) = scalars.column_by_name("shape") {
            // Scalar shape - still use to_symbol_shape for consistency
            coercer.to_symbol_shape(shape_scalar, None)?
        } else {
            // Default shape
            (vec![avenger_common::types::SymbolShape::Circle], ScalarOrArray::new_scalar(0))
        };

        // Extract other channels
        let size = coerce_numeric_channel(data, scalars, "size", 64.0)?;
        let fill = coerce_color_channel(data, scalars, "fill", [70.0/255.0, 130.0/255.0, 180.0/255.0, 1.0])?;
        let stroke = coerce_color_channel(data, scalars, "stroke", [0.0, 0.0, 0.0, 1.0])?;
        let angle = coerce_numeric_channel(data, scalars, "angle", 0.0)?;
        
        // Stroke width is scalar only
        let stroke_width = if let Some(width_scalar) = scalars.column_by_name("stroke_width") {
            Some(*coercer.to_numeric(width_scalar, Some(1.0))?.first().unwrap())
        } else {
            Some(1.0)
        };

        let symbol_mark = SceneSymbolMark {
            name: "symbol".to_string(),
            clip: true,
            len: len as u32,
            gradients: vec![],
            shapes,
            stroke_width,
            shape_index,
            x,
            y,
            fill,
            size,
            stroke,
            angle,
            indices: None,
            zindex: self.state.zindex,
            x_adjustment: None,
            y_adjustment: None,
        };

        Ok(vec![SceneMark::Symbol(symbol_mark)])
    }
}

// Implement Mark trait for Polar Symbol
impl Mark<Polar> for Symbol<Polar> {
    impl_mark_trait_common!(Symbol, Polar, "symbol");

    fn render_from_data(
        &self,
        _data: Option<&RecordBatch>,
        _scalars: &RecordBatch,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        Err(AvengerChartError::InternalError(
            "Polar symbol mark rendering not yet implemented".to_string(),
        ))
    }
}
