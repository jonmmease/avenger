use crate::define_position_channels;
use crate::impl_mark_trait_common;
use crate::marks::{ChannelType, Mark};

use crate::polar::Polar;
use arrow::array::RecordBatch;
use avenger_scenegraph::marks::mark::SceneMark;

// Import Rect for the macro, then re-export it
use crate::error::AvengerChartError;
use crate::marks::rect::Rect;

// Define position channels for Polar Rect using the macro
define_position_channels! {
    Rect<Polar> {
        r: {
            type: ChannelType::Numeric,
            with_config: crate::polar::channels::PolarPositionConfig
        },
        r2: {
            type: ChannelType::Numeric,
            with_config: crate::polar::channels::PolarPositionConfig
        },
        theta: {
            type: ChannelType::Numeric,
            with_config: crate::polar::channels::PolarPositionConfig
        },
        theta2: {
            type: ChannelType::Numeric,
            with_config: crate::polar::channels::PolarPositionConfig
        }
    }
}

// Implement Mark trait for PolarGeneral Rect with any axis type
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
