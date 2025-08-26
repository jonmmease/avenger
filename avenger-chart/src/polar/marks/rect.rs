use crate::impl_mark_trait_common;
use crate::marks::{ChannelType, Mark};
use crate::polar::PolarAxis;
use crate::polar::coord::PolarGeneral;
use arrow::array::RecordBatch;
use avenger_scenegraph::marks::mark::SceneMark;

// Import Rect for the macro, then re-export it
use crate::error::AvengerChartError;
use crate::marks::rect::Rect;

// Implement position channels for PolarGeneral Rect with generic axis support
impl<A: PolarAxis + Default + 'static> Rect<PolarGeneral<A>> {
    pub fn r<V: Into<crate::marks::ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("r", value.into())
    }

    pub fn r_with<F>(self, value: impl Into<crate::marks::ChannelValue>, f: F) -> Self
    where
        F: FnOnce(
            crate::marks::typed_channels::PositionChannel,
        ) -> crate::marks::typed_channels::PositionChannel,
    {
        let channel_value: crate::marks::ChannelValue = value.into();
        let channel = f(crate::marks::typed_channels::PositionChannel(channel_value));
        self.with_channel_value("r", channel.into())
    }

    pub fn r2<V: Into<crate::marks::ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("r2", value.into())
    }

    pub fn r2_with<F>(self, value: impl Into<crate::marks::ChannelValue>, f: F) -> Self
    where
        F: FnOnce(
            crate::marks::typed_channels::PositionChannel,
        ) -> crate::marks::typed_channels::PositionChannel,
    {
        let channel_value: crate::marks::ChannelValue = value.into();
        let channel = f(crate::marks::typed_channels::PositionChannel(channel_value));
        self.with_channel_value("r2", channel.into())
    }

    pub fn theta<V: Into<crate::marks::ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("theta", value.into())
    }

    pub fn theta_with<F>(self, value: impl Into<crate::marks::ChannelValue>, f: F) -> Self
    where
        F: FnOnce(
            crate::marks::typed_channels::PositionChannel,
        ) -> crate::marks::typed_channels::PositionChannel,
    {
        let channel_value: crate::marks::ChannelValue = value.into();
        let channel = f(crate::marks::typed_channels::PositionChannel(channel_value));
        self.with_channel_value("theta", channel.into())
    }

    pub fn theta2<V: Into<crate::marks::ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("theta2", value.into())
    }

    pub fn theta2_with<F>(self, value: impl Into<crate::marks::ChannelValue>, f: F) -> Self
    where
        F: FnOnce(
            crate::marks::typed_channels::PositionChannel,
        ) -> crate::marks::typed_channels::PositionChannel,
    {
        let channel_value: crate::marks::ChannelValue = value.into();
        let channel = f(crate::marks::typed_channels::PositionChannel(channel_value));
        self.with_channel_value("theta2", channel.into())
    }

    pub fn position_channel_descriptors() -> Vec<crate::marks::ChannelDescriptor> {
        use crate::marks::ChannelDescriptor;
        vec![
            ChannelDescriptor {
                name: "r",
                channel_type: ChannelType::Numeric,
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "r2",
                channel_type: ChannelType::Numeric,
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "theta",
                channel_type: ChannelType::Numeric,
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "theta2",
                channel_type: ChannelType::Numeric,
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
        ]
    }

    pub fn all_channel_descriptors() -> Vec<crate::marks::ChannelDescriptor> {
        let mut descriptors = Self::common_channel_descriptors();
        descriptors.extend(Self::position_channel_descriptors());
        descriptors
    }
}

// Implement Mark trait for PolarGeneral Rect with any axis type
impl<A: PolarAxis + Default + 'static> Mark<PolarGeneral<A>> for Rect<PolarGeneral<A>> {
    impl_mark_trait_common!(Rect, PolarGeneral<A>, "rect");

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
