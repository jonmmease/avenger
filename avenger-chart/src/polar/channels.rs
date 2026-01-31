use crate::channel::GenericPositionConfig;

use super::axis::PolarAxis;

/// Configuration for Polar position channels (r, theta)
/// These channels support scales and axes
pub type PolarPositionConfig = GenericPositionConfig<PolarAxis>;
