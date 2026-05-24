use avenger_chart_core::GenericPositionConfig;

use crate::PolarAxis;

/// Configuration for Polar position channels (r, theta).
///
/// These channels support scales and axes.
pub type PolarPositionConfig = GenericPositionConfig<PolarAxis>;
