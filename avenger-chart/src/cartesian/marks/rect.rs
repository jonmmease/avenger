use arrow::array::RecordBatch;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::rect::SceneRectMark;
use crate::cartesian::Cartesian;
use crate::{define_position_mark_channels, impl_mark_trait_common};
use crate::marks::{ChannelType, Mark};

// Import Rect for the macro, then re-export it
pub(crate) use crate::marks::rect::Rect;
use crate::error::AvengerChartError;
use crate::marks::util::{coerce_color_channel, coerce_numeric_channel};


// Define position channels for Cartesian Rect
define_position_mark_channels! {
    Rect<Cartesian> {
        x: { type: ChannelType::Numeric },
        x2: { type: ChannelType::Numeric },
        y: { type: ChannelType::Numeric },
        y2: { type: ChannelType::Numeric },
    }
}

// Implement Mark trait for Cartesian Rect
impl Mark<Cartesian> for Rect<Cartesian> {
    impl_mark_trait_common!(Rect, Cartesian, "rect");

    fn render_from_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // Determine number of marks from data batch or default to 1
        let len = data.map_or(1, |data| data.num_rows()) as u32;

        // Extract position values using Coercer
        let x = coerce_numeric_channel(data, scalars, "x", 0.0)?;
        let x2 = coerce_numeric_channel(data, scalars, "x2", 0.0)?;
        let y = coerce_numeric_channel(data, scalars, "y", 0.0)?;
        let y2 = coerce_numeric_channel(data, scalars, "y2", 0.0)?;

        // Extract style values using Coercer
        let fill = coerce_color_channel(data, scalars, "fill", [0.27, 0.51, 0.71, 1.0])?;
        let stroke = coerce_color_channel(data, scalars, "stroke", [0.0, 0.0, 0.0, 1.0])?;
        let stroke_width = coerce_numeric_channel(data, scalars, "stroke_width", 1.0)?;
        let corner_radius = coerce_numeric_channel(data, scalars, "corner_radius", 0.0)?;

        // Create SceneRectMark
        let rect_mark = SceneRectMark {
            name: "rect".to_string(),
            clip: true,
            len,
            gradients: vec![],
            x,
            y,
            width: None,
            height: None,
            x2: Some(x2),
            y2: Some(y2),
            fill,
            stroke,
            stroke_width,
            corner_radius,
            indices: None,
            zindex: self.state.zindex,
        };

        Ok(vec![SceneMark::Rect(rect_mark)])
    }
}
