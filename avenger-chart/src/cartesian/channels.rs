use crate::{cartesian::CartesianAxis, channel::GenericPositionConfig};

/// Configuration for Cartesian position channels (x, y, x2, y2)
/// These channels support scales and axes but not legends
pub type CartesianPositionConfig = GenericPositionConfig<CartesianAxis>;
