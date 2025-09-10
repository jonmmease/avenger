use crate::cartesian::CartesianAxis;
use crate::channels::GenericPositionConfig;

/// Configuration for Cartesian position channels (x, y, x2, y2)
/// These channels support scales and axes but not legends
pub type CartesianPositionConfig = GenericPositionConfig<CartesianAxis>;
