use crate::coords::{Cartesian, CoordinateSystem, Polar};
use crate::error::AvengerChartError;
use crate::marks::util::{
    get_color_channel, get_numeric_channel, get_numeric_channel_scalar_or_array,
};
use crate::marks::{ChannelDescriptor, ChannelType, Mark, MarkConfig};
use crate::{define_common_mark_channels, define_position_mark_channels, impl_mark_common};
use avenger_common::value::ScalarOrArray;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::rect::SceneRectMark;
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::dataframe::DataFrame;
use datafusion::scalar::ScalarValue;
use std::collections::HashMap;

pub struct Rect<C: CoordinateSystem> {
    config: MarkConfig<C>,
    __phantom: std::marker::PhantomData<C>,
}

// Common mark methods
impl_mark_common!(Rect, "rect");

// Define common channels for all coordinate systems
define_common_mark_channels! {
    Rect {
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
            default: ScalarValue::Float32(Some(1.0))
        },
        opacity: {
            type: ChannelType::Numeric,
            default: ScalarValue::Float32(Some(1.0))
        },
        corner_radius: {
            type: ChannelType::Numeric,
            default: ScalarValue::Float32(Some(0.0))
        },
    }
}

// Define position channels for Cartesian coordinates
define_position_mark_channels! {
    Rect<Cartesian> {
        x: { type: ChannelType::Position },
        x2: { type: ChannelType::Position },
        y: { type: ChannelType::Position },
        y2: { type: ChannelType::Position },
    }
}

// Define position channels for Polar coordinates
define_position_mark_channels! {
    Rect<Polar> {
        r: { type: ChannelType::Position },
        r2: { type: ChannelType::Position },
        theta: { type: ChannelType::Position },
        theta2: { type: ChannelType::Position },
    }
}

// Implement new Mark trait methods for Cartesian Rect
impl Mark<Cartesian> for Rect<Cartesian> {
    fn into_config(self) -> MarkConfig<Cartesian> {
        self.config
    }

    fn supported_channels(&self) -> Vec<ChannelDescriptor> {
        Self::all_channel_descriptors()
    }

    fn render_from_data(
        &self,
        batch: Option<&RecordBatch>,
        scalars: &HashMap<String, ScalarValue>,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // Determine number of marks
        let num_rows = batch.map(|b| b.num_rows()).unwrap_or(1) as u32;

        // Extract position values
        let x_values = get_numeric_channel("x", batch, scalars, 0.0)?;
        let x2_values = get_numeric_channel("x2", batch, scalars, 0.0)?;
        let y_values = get_numeric_channel("y", batch, scalars, 0.0)?;
        let y2_values = get_numeric_channel("y2", batch, scalars, 0.0)?;

        // Extract style values
        let fill = get_color_channel("fill", batch, scalars, [0.27, 0.51, 0.71, 1.0])?;
        let stroke = get_color_channel("stroke", batch, scalars, [0.0, 0.0, 0.0, 1.0])?;
        let stroke_width =
            get_numeric_channel_scalar_or_array("stroke_width", batch, scalars, 1.0)?;
        let corner_radius =
            get_numeric_channel_scalar_or_array("corner_radius", batch, scalars, 0.0)?;

        // Create SceneRectMark
        let rect_mark = SceneRectMark {
            name: "rect".to_string(),
            clip: true,
            len: num_rows,
            gradients: vec![],
            x: ScalarOrArray::new_array(x_values),
            y: ScalarOrArray::new_array(y_values),
            width: None,
            height: None,
            x2: Some(ScalarOrArray::new_array(x2_values)),
            y2: Some(ScalarOrArray::new_array(y2_values)),
            fill,
            stroke,
            stroke_width,
            corner_radius,
            indices: None,
            zindex: self.config.zindex,
        };

        Ok(vec![SceneMark::Rect(rect_mark)])
    }
}

// Stub implementation for Polar coordinates
impl Mark<Polar> for Rect<Polar> {
    fn into_config(self) -> MarkConfig<Polar> {
        self.config
    }

    fn supported_channels(&self) -> Vec<ChannelDescriptor> {
        Self::all_channel_descriptors()
    }

    fn render_from_data(
        &self,
        _batch: Option<&RecordBatch>,
        _scalars: &HashMap<String, ScalarValue>,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // TODO: Implement polar rect rendering
        Err(AvengerChartError::InternalError(
            "Polar rect rendering not yet implemented".to_string(),
        ))
    }
}
