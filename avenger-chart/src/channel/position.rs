//! Generic position channel configuration that can be used across coordinate systems

use super::value::ChannelValue;
use crate::scales::{Auto, Scale, ScaleSpec as ScaleTypeSpec};

/// Trait for all position configuration types
pub trait PositionConfig: Sized {
    type Axis: Clone;

    /// Create a new position config from a channel value
    fn new(value: ChannelValue) -> Self;

    /// Extract the axis configuration, consuming self
    /// Returns None for coordinate systems that don't support axes
    fn take_axis_config(self) -> (ChannelValue, Option<Self::Axis>);

    /// Get the inner channel value without axis config
    fn into_inner(self) -> ChannelValue;
}

/// Generic configuration for position channels across coordinate systems
/// This struct provides common functionality for position channels that support
/// scales and axes in any coordinate system
#[derive(Clone)]
pub struct GenericPositionConfig<A: Clone + Default + Send + Sync + 'static> {
    pub(crate) inner: ChannelValue,
    pub(crate) axis_config: Option<A>,
}

impl<A: Clone + Default + Send + Sync + 'static> GenericPositionConfig<A> {
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
        F: FnOnce(A) -> A,
    {
        let axis = self.axis_config.unwrap_or_default();
        self.axis_config = Some(f(axis));
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
    pub fn axis_config(&self) -> Option<&A> {
        self.axis_config.as_ref()
    }
}

impl<A: Clone + Default + Send + Sync + 'static> PositionConfig for GenericPositionConfig<A> {
    type Axis = A;

    fn new(value: ChannelValue) -> Self {
        Self {
            inner: value,
            axis_config: None,
        }
    }

    fn take_axis_config(self) -> (ChannelValue, Option<Self::Axis>) {
        (self.inner, self.axis_config)
    }

    fn into_inner(self) -> ChannelValue {
        self.inner
    }
}

// Implement From traits for common types
impl<A: Clone + Default + Send + Sync + 'static> From<ChannelValue> for GenericPositionConfig<A> {
    fn from(value: ChannelValue) -> Self {
        Self::new(value)
    }
}

impl<A: Clone + Default + Send + Sync + 'static> From<datafusion::logical_expr::Expr>
    for GenericPositionConfig<A>
{
    fn from(expr: datafusion::logical_expr::Expr) -> Self {
        Self::new(ChannelValue::from(expr))
    }
}

impl<A: Clone + Default + Send + Sync + 'static> From<&str> for GenericPositionConfig<A> {
    fn from(s: &str) -> Self {
        Self::new(ChannelValue::from(s))
    }
}

impl<A: Clone + Default + Send + Sync + 'static> From<f64> for GenericPositionConfig<A> {
    fn from(v: f64) -> Self {
        Self::new(ChannelValue::from(v))
    }
}

impl<A: Clone + Default + Send + Sync + 'static> From<f32> for GenericPositionConfig<A> {
    fn from(v: f32) -> Self {
        Self::new(ChannelValue::from(v))
    }
}

impl<A: Clone + Default + Send + Sync + 'static> From<i32> for GenericPositionConfig<A> {
    fn from(v: i32) -> Self {
        Self::new(ChannelValue::from(v))
    }
}
