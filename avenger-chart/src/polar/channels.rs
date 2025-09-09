use crate::channels::GenericPositionConfig;
use crate::polar::axis::PolarAxis;

/// Configuration for Polar position channels (r, theta)
/// These channels support scales and axes
pub type PolarPositionConfig = GenericPositionConfig<PolarAxis>;