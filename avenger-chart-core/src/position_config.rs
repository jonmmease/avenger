use crate::ChannelValue;

/// Trait for coordinate-specific position channel configuration types.
pub trait PositionConfig: Sized {
    type Axis: Clone;

    /// Create a new position config from a channel value.
    fn new(value: ChannelValue) -> Self;

    /// Extract the axis configuration, consuming self.
    ///
    /// Returns `None` for coordinate systems that do not support axes.
    fn take_axis_config(self) -> (ChannelValue, Option<Self::Axis>);

    /// Get the inner channel value without axis config.
    fn into_inner(self) -> ChannelValue;
}
