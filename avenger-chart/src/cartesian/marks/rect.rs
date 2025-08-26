use crate::cartesian::CartesianAxis;
use crate::cartesian::coord::CartesianGeneral;
use crate::impl_mark_trait_common;
use crate::marks::{ChannelType, Mark};
use arrow::array::RecordBatch;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::rect::SceneRectMark;
// Import Rect for the macro, then re-export it
use crate::error::AvengerChartError;
pub use crate::marks::rect::Rect;
use crate::marks::util::{coerce_color_channel, coerce_numeric_channel};

// Implement position channels for Cartesian Rect with generic axis support
impl<A: CartesianAxis + Default + 'static> Rect<CartesianGeneral<A>> {
    pub fn x<V: Into<crate::marks::ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("x", value.into())
    }

    pub fn x_with<F>(self, value: impl Into<crate::marks::ChannelValue>, f: F) -> Self
    where
        F: FnOnce(
            crate::cartesian::CartesianPositionChannel<A>,
        ) -> crate::cartesian::CartesianPositionChannel<A>,
    {
        let channel_value: crate::marks::ChannelValue = value.into();
        let channel = crate::cartesian::CartesianPositionChannel::<A>::new(channel_value);
        let configured = f(channel);

        // Extract axis config before consuming channel
        let axis_config = configured.axis_config().cloned();

        // Store channel value
        let mut mark = self.with_channel_value("x", configured.into_inner());

        // Store axis config if present
        if let Some(axis_config) = axis_config {
            mark.state_mut()
                .axis_configs
                .insert("x".to_string(), axis_config);
        }

        mark
    }

    pub fn x2<V: Into<crate::marks::ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("x2", value.into())
    }

    pub fn x2_with<F>(self, value: impl Into<crate::marks::ChannelValue>, f: F) -> Self
    where
        F: FnOnce(
            crate::cartesian::CartesianPositionChannel<A>,
        ) -> crate::cartesian::CartesianPositionChannel<A>,
    {
        let channel_value: crate::marks::ChannelValue = value.into();
        let channel = crate::cartesian::CartesianPositionChannel::<A>::new(channel_value);
        let configured = f(channel);

        // Extract axis config before consuming channel
        let axis_config = configured.axis_config().cloned();

        // Store channel value
        let mut mark = self.with_channel_value("x2", configured.into_inner());

        // Store axis config if present
        if let Some(axis_config) = axis_config {
            mark.state_mut()
                .axis_configs
                .insert("x2".to_string(), axis_config);
        }

        mark
    }

    pub fn y<V: Into<crate::marks::ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("y", value.into())
    }

    pub fn y_with<F>(self, value: impl Into<crate::marks::ChannelValue>, f: F) -> Self
    where
        F: FnOnce(
            crate::cartesian::CartesianPositionChannel<A>,
        ) -> crate::cartesian::CartesianPositionChannel<A>,
    {
        let channel_value: crate::marks::ChannelValue = value.into();
        let channel = crate::cartesian::CartesianPositionChannel::<A>::new(channel_value);
        let configured = f(channel);

        // Extract axis config before consuming channel
        let axis_config = configured.axis_config().cloned();

        // Store channel value
        let mut mark = self.with_channel_value("y", configured.into_inner());

        // Store axis config if present
        if let Some(axis_config) = axis_config {
            mark.state_mut()
                .axis_configs
                .insert("y".to_string(), axis_config);
        }

        mark
    }

    pub fn y2<V: Into<crate::marks::ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("y2", value.into())
    }

    pub fn y2_with<F>(self, value: impl Into<crate::marks::ChannelValue>, f: F) -> Self
    where
        F: FnOnce(
            crate::cartesian::CartesianPositionChannel<A>,
        ) -> crate::cartesian::CartesianPositionChannel<A>,
    {
        let channel_value: crate::marks::ChannelValue = value.into();
        let channel = crate::cartesian::CartesianPositionChannel::<A>::new(channel_value);
        let configured = f(channel);

        // Extract axis config before consuming channel
        let axis_config = configured.axis_config().cloned();

        // Store channel value
        let mut mark = self.with_channel_value("y2", configured.into_inner());

        // Store axis config if present
        if let Some(axis_config) = axis_config {
            mark.state_mut()
                .axis_configs
                .insert("y2".to_string(), axis_config);
        }

        mark
    }

    pub fn position_channel_descriptors() -> Vec<crate::marks::ChannelDescriptor> {
        use crate::marks::ChannelDescriptor;
        vec![
            ChannelDescriptor {
                name: "x",
                channel_type: ChannelType::Numeric,
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "x2",
                channel_type: ChannelType::Numeric,
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "y",
                channel_type: ChannelType::Numeric,
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "y2",
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

// Implement Mark trait for Cartesian Rect with any axis type
impl<A: CartesianAxis + Default + 'static> Mark<CartesianGeneral<A>> for Rect<CartesianGeneral<A>> {
    impl_mark_trait_common!(Rect, CartesianGeneral<A>, "rect");

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
            zindex: self.get_zindex(),
        };

        Ok(vec![SceneMark::Rect(rect_mark)])
    }
}
