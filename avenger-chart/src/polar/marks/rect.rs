use arrow::array::RecordBatch;
use avenger_scenegraph::marks::mark::SceneMark;
use crate::{define_position_mark_channels, impl_mark_trait_common};
use crate::marks::{ChannelType, Mark};
use crate::polar::Polar;

// Import Rect for the macro, then re-export it
use crate::marks::rect::Rect;
use crate::error::AvengerChartError;

// Define position channels for Polar Rect
define_position_mark_channels! {
    Rect<Polar> {
        r: { type: ChannelType::Numeric },
        r2: { type: ChannelType::Numeric },
        theta: { type: ChannelType::Numeric },
        theta2: { type: ChannelType::Numeric },
    }
}

// Implement Mark trait for Polar Rect
impl Mark<Polar> for Rect<Polar> {
    impl_mark_trait_common!(Rect, Polar, "rect");

    fn render_from_data(
        &self,
        _data: Option<&RecordBatch>,
        _scalars: &RecordBatch,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // Polar rect rendering not yet implemented
        Err(AvengerChartError::InternalError(
            "Polar rect rendering not yet implemented".to_string(),
        ))
    }
}

