use crate::coords::{Cartesian, CoordinateSystem, Polar};
use crate::error::AvengerChartError;
use crate::marks::util::{coerce_color_channel, coerce_numeric_channel};
use crate::marks::{ChannelType, Mark, MarkPadding, MarkState};
use crate::{
    define_common_mark_channels, define_position_mark_channels, impl_mark_common,
    impl_mark_trait_common,
};
use avenger_common::value::{ScalarOrArray, ScalarOrArrayValue};
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

    fn padding_channels(&self) -> Vec<&'static str> {
        // Include positional channels for geometry-based calculation
        vec!["x", "y", "size", "stroke_width", "shape", "angle"]
    }

    fn compute_padding(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        clip_bounds: &crate::marks::ClipBounds,
        plot_area_width: f32,
        plot_area_height: f32,
    ) -> Result<MarkPadding, AvengerChartError> {
        use avenger_scales::scales::coerce::Coercer;
        use avenger_geometry::marks::MarkGeometryUtils;


        // We need to know the scale to convert between data and pixel coordinates
        // The clip_bounds are in data coordinates, but we need pixel coordinates
        // Calculate scale factors using actual plot area dimensions
        let x_scale_factor = plot_area_width as f64 / (clip_bounds.x_max - clip_bounds.x_min);
        let y_scale_factor = plot_area_height as f64 / (clip_bounds.y_max - clip_bounds.y_min);

        let coercer = Coercer::default();

        // Build a SceneSymbolMark with just the channels needed for geometry
        // Extract position data - these can be scalars or arrays
        let x = coerce_numeric_channel(data, scalars, "x", 0.0)?;
        let y = coerce_numeric_channel(data, scalars, "y", 0.0)?;

        // Convert x,y from data coordinates to pixel coordinates for the scene mark
        let x_pixels = match x.value() {
            ScalarOrArrayValue::Scalar(v) => {
                let pixel_x = (v - clip_bounds.x_min as f32) * x_scale_factor as f32;
                ScalarOrArray::from(pixel_x)
            }
            ScalarOrArrayValue::Array(arr) => {
                let pixels: Vec<f32> = arr.iter()
                    .map(|v| (v - clip_bounds.x_min as f32) * x_scale_factor as f32)
                    .collect();
                ScalarOrArray::from(pixels)
            }
        };

        let y_pixels = match y.value() {
            ScalarOrArrayValue::Scalar(v) => {
                // Y is flipped in pixel coordinates
                let pixel_y = plot_area_height - (v - clip_bounds.y_min as f32) * y_scale_factor as f32;
                ScalarOrArray::from(pixel_y)
            }
            ScalarOrArrayValue::Array(arr) => {
                let pixels: Vec<f32> = arr.iter()
                    .map(|v| plot_area_height - (v - clip_bounds.y_min as f32) * y_scale_factor as f32)
                    .collect();
                ScalarOrArray::from(pixels)
            }
        };

        // Extract other channels needed for geometry
        let size = coerce_numeric_channel(data, scalars, "size", 64.0)?;
        let angle = coerce_numeric_channel(data, scalars, "angle", 0.0)?;
        
        // Get stroke width (scalar only)
        let stroke_width = if let Some(width_scalar) = scalars.column_by_name("stroke_width") {
            Some(
                *coercer
                    .to_numeric(width_scalar, Some(1.0))?
                    .first()
                    .unwrap()
            )
        } else {
            Some(1.0)
        };

        // Extract shapes
        let (shapes, shape_index) = if let Some(shape_values) = data
            .as_ref()
            .and_then(|d| d.column_by_name("shape"))
            .or_else(|| scalars.column_by_name("shape"))
        {
            coercer.to_symbol_shape(shape_values, None)?
        } else {
            // Default shape
            (
                vec![avenger_common::types::SymbolShape::default()],
                ScalarOrArray::from(0usize),
            )
        };

        // Determine length from any array channel
        let len = match (&x.value(), &y.value(), &size.value()) {
            (ScalarOrArrayValue::Array(arr), _, _) => arr.len(),
            (_, ScalarOrArrayValue::Array(arr), _) => arr.len(),
            (_, _, ScalarOrArrayValue::Array(arr)) => arr.len(),
            _ => 1, // All scalars
        };

        // Build scene mark for bounding box calculation
        let scene_mark = SceneSymbolMark {
            len: len as u32,
            x: x_pixels,
            y: y_pixels,
            size: size.clone(),
            angle: angle.clone(),
            stroke_width,
            shapes,
            shape_index,
            ..Default::default()
        };

        // Get bounding box
        let scene_mark: SceneMark = scene_mark.into();
        let bbox = scene_mark.bounding_box();
        let bbox_lower = bbox.lower();
        let bbox_upper = bbox.upper();
        
        // Calculate padding based on how much the symbols extend beyond the plot area
        let x_lower_padding = if bbox_lower[0] < 0.0 { -bbox_lower[0] as f64 } else { 0.0 };
        let x_upper_padding = if bbox_upper[0] > plot_area_width { (bbox_upper[0] - plot_area_width) as f64 } else { 0.0 };
        let y_lower_padding = if bbox_upper[1] > plot_area_height { (bbox_upper[1] - plot_area_height) as f64 } else { 0.0 }; // Y is flipped
        let y_upper_padding = if bbox_lower[1] < 0.0 { -bbox_lower[1] as f64 } else { 0.0 }; // Y is flipped

        Ok(MarkPadding {
            x_lower: if x_lower_padding > 0.0 { Some(x_lower_padding) } else { None },
            x_upper: if x_upper_padding > 0.0 { Some(x_upper_padding) } else { None },
            y_lower: if y_lower_padding > 0.0 { Some(y_lower_padding) } else { None },
            y_upper: if y_upper_padding > 0.0 { Some(y_upper_padding) } else { None },
        })
    }

    fn render_from_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        use avenger_scales::scales::coerce::Coercer;

        // Symbols can render with just scalar data
        let coercer = Coercer::default();

        // Extract position data - these can be scalars or arrays
        let x = coerce_numeric_channel(data, scalars, "x", 0.0)?;
        let y = coerce_numeric_channel(data, scalars, "y", 0.0)?;

        // Extract other channels first to determine length from any array channel
        let size = coerce_numeric_channel(data, scalars, "size", 64.0)?;
        let fill = coerce_color_channel(
            data,
            scalars,
            "fill",
            [70.0 / 255.0, 130.0 / 255.0, 180.0 / 255.0, 1.0],
        )?;
        let stroke = coerce_color_channel(data, scalars, "stroke", [0.0, 0.0, 0.0, 1.0])?;
        let angle = coerce_numeric_channel(data, scalars, "angle", 0.0)?;

        // Determine the number of symbols from any array channel
        let len = data.map_or(1, |data| data.num_rows()) as u32;

        // Handle shape channel efficiently
        let (shapes, shape_index) =
            if let Some(shape_array) = data.and_then(|d| d.column_by_name("shape")) {
                // Array data for shapes - use efficient coercion
                coercer.to_symbol_shape(shape_array, None)?
            } else if let Some(shape_scalar) = scalars.column_by_name("shape") {
                // Scalar shape - still use to_symbol_shape for consistency
                coercer.to_symbol_shape(shape_scalar, None)?
            } else {
                // Default shape
                (
                    vec![avenger_common::types::SymbolShape::Circle],
                    ScalarOrArray::new_scalar(0),
                )
            };

        // Stroke width is scalar only
        let stroke_width = if let Some(width_scalar) = scalars.column_by_name("stroke_width") {
            Some(
                *coercer
                    .to_numeric(width_scalar, Some(1.0))?
                    .first()
                    .unwrap(),
            )
        } else {
            Some(1.0)
        };

        let symbol_mark = SceneSymbolMark {
            name: "symbol".to_string(),
            clip: true,
            len,
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
