use crate::cartesian::CartesianAxis;
use crate::marks::channel::ChannelValue;
use crate::scales::{Auto, Scale, ScaleSpec as ScaleTypeSpec};
use std::sync::Arc;

/// Typed wrapper for Cartesian position channels (x, y, x2, y2)
/// These channels support scales and axes but not legends
#[derive(Clone)]
pub struct CartesianPositionChannel {
    pub(crate) inner: ChannelValue,
    pub(crate) axis_config: Option<Arc<dyn Fn(CartesianAxis) -> CartesianAxis + Send + Sync>>,
}

impl CartesianPositionChannel {
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

    /// Configure the axis for this channel
    pub fn axis<F>(mut self, f: F) -> Self
    where
        F: Fn(CartesianAxis) -> CartesianAxis + Send + Sync + 'static,
    {
        self.axis_config = Some(Arc::new(f));
        self
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

    /// Get the inner ChannelValue
    pub fn into_inner(self) -> ChannelValue {
        self.inner
    }

    /// Get the axis configuration if present
    pub fn axis_config(
        &self,
    ) -> Option<&Arc<dyn Fn(CartesianAxis) -> CartesianAxis + Send + Sync>> {
        self.axis_config.as_ref()
    }
}

// Conversions from various types to CartesianPositionChannel
impl From<ChannelValue> for CartesianPositionChannel {
    fn from(value: ChannelValue) -> Self {
        Self::new(value)
    }
}

impl From<datafusion::logical_expr::Expr> for CartesianPositionChannel {
    fn from(expr: datafusion::logical_expr::Expr) -> Self {
        Self::new(ChannelValue::from(expr))
    }
}

impl From<&str> for CartesianPositionChannel {
    fn from(s: &str) -> Self {
        Self::new(ChannelValue::from(s))
    }
}

impl From<f64> for CartesianPositionChannel {
    fn from(v: f64) -> Self {
        Self::new(ChannelValue::from(v))
    }
}

impl From<f32> for CartesianPositionChannel {
    fn from(v: f32) -> Self {
        Self::new(ChannelValue::from(v))
    }
}

impl From<i32> for CartesianPositionChannel {
    fn from(v: i32) -> Self {
        Self::new(ChannelValue::from(v))
    }
}
