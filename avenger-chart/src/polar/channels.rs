use crate::marks::channel::ChannelValue;
use crate::polar::axis::PolarAxis;
use crate::scales::{Auto, Scale, ScaleSpec as ScaleTypeSpec};
use std::sync::Arc;

/// Configuration for Polar position channels (r, theta)
/// These channels support scales and axes
#[derive(Clone)]
pub struct PolarPositionConfig {
    pub(crate) inner: ChannelValue,
    pub(crate) axis_config: Option<Arc<dyn Fn(PolarAxis) -> PolarAxis + Send + Sync>>,
}

impl PolarPositionConfig {
    /// Create a new position channel from a channel value
    pub fn new(value: ChannelValue) -> Self {
        Self {
            inner: value,
            axis_config: None,
        }
    }

    /// Disable scaling for this channel
    pub fn no_scale(self) -> Self {
        Self {
            inner: self.inner.no_scale(),
            axis_config: self.axis_config,
        }
    }

    /// Configure the scale for this channel
    pub fn scale<F>(self, f: F) -> Self
    where
        F: Fn(Scale<Auto>) -> Scale<Auto> + Send + Sync + 'static,
    {
        Self {
            inner: self.inner.scale(f),
            axis_config: self.axis_config,
        }
    }

    /// Configure the scale with explicit type
    pub fn scale_with<S: ScaleTypeSpec>(
        self,
        f: impl Fn(Scale<S>) -> Scale<S> + Send + Sync + 'static,
    ) -> Self {
        Self {
            inner: self.inner.scale_with(f),
            axis_config: self.axis_config,
        }
    }

    /// Set the band parameter
    pub fn band(self, band: f64) -> Self {
        Self {
            inner: self.inner.band(band),
            axis_config: self.axis_config,
        }
    }

    /// Set a custom scale name
    pub fn with_scale_name(self, name: impl Into<String>) -> Self {
        Self {
            inner: self.inner.with_scale_name(name),
            axis_config: self.axis_config,
        }
    }

    /// Configure the axis for this channel
    pub fn axis<F>(mut self, f: F) -> Self
    where
        F: Fn(PolarAxis) -> PolarAxis + Send + Sync + 'static,
    {
        self.axis_config = Some(Arc::new(f));
        self
    }

    /// Get the inner ChannelValue
    pub fn into_inner(self) -> ChannelValue {
        self.inner
    }
}

impl crate::cartesian::channels::PositionConfig for PolarPositionConfig {
    type Axis = <crate::polar::Polar as crate::coords::CoordinateSystem>::Axis;

    fn new(value: ChannelValue) -> Self {
        Self {
            inner: value,
            axis_config: None,
        }
    }

    fn take_axis_config(
        self,
    ) -> (
        ChannelValue,
        Option<Arc<dyn Fn(Self::Axis) -> Self::Axis + Send + Sync>>,
    ) {
        (self.inner, self.axis_config)
    }

    fn into_inner(self) -> ChannelValue {
        self.inner
    }
}

// Conversions from various types to PolarPositionConfig
impl From<ChannelValue> for PolarPositionConfig {
    fn from(value: ChannelValue) -> Self {
        Self::new(value)
    }
}

impl From<datafusion::logical_expr::Expr> for PolarPositionConfig {
    fn from(expr: datafusion::logical_expr::Expr) -> Self {
        Self::new(ChannelValue::from(expr))
    }
}

impl From<&str> for PolarPositionConfig {
    fn from(s: &str) -> Self {
        Self::new(ChannelValue::from(s))
    }
}

impl From<f64> for PolarPositionConfig {
    fn from(v: f64) -> Self {
        Self::new(ChannelValue::from(v))
    }
}

impl From<f32> for PolarPositionConfig {
    fn from(v: f32) -> Self {
        Self::new(ChannelValue::from(v))
    }
}

impl From<i32> for PolarPositionConfig {
    fn from(v: i32) -> Self {
        Self::new(ChannelValue::from(v))
    }
}

