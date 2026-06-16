use datafusion::logical_expr::Expr;

use crate::{
    Axis, ChannelConfig, ChannelValue, NestedBandLevelConfig, NestedBandSpec, PositionBoundary,
};

/// Generic configuration for position channels across coordinate systems.
///
/// This core type stores a channel value plus an optional coordinate-specific
/// axis configuration. Scale and legend fluent methods are supplied by the
/// scales/legend crates through extension traits.
#[derive(Clone)]
pub struct GenericPositionConfig<A: Clone + Default + Send + Sync + 'static> {
    pub(crate) inner: ChannelValue,
    pub(crate) axis_config: Option<A>,
}

impl<A: Clone + Default + Send + Sync + 'static> GenericPositionConfig<A> {
    /// Create a new position channel from a channel value.
    pub fn new(value: ChannelValue) -> Self {
        Self {
            inner: value,
            axis_config: None,
        }
    }

    /// Disable scaling for this channel.
    pub fn no_scale(self) -> Self {
        Self {
            inner: self.inner.no_scale(),
            axis_config: self.axis_config,
        }
    }

    /// Configure the axis for this channel.
    pub fn axis<F>(mut self, f: F) -> Self
    where
        F: FnOnce(A) -> A,
    {
        let axis = self.axis_config.unwrap_or_default();
        self.axis_config = Some(f(axis));
        self
    }

    /// Set the band parameter.
    pub fn band(self, band: f64) -> Self {
        Self {
            inner: self.inner.band(band),
            axis_config: self.axis_config,
        }
    }

    /// Configure one level of a nested categorical band position channel.
    pub fn level<F>(self, level: usize, f: F) -> Self
    where
        A: Axis,
        F: FnOnce(NestedBandLevelConfig<A>) -> NestedBandLevelConfig<A>,
    {
        let mut nested = self
            .inner
            .get_nested_band_config()
            .cloned()
            .unwrap_or_else(NestedBandSpec::default);
        let level_spec = nested.levels.remove(&level).unwrap_or_default();
        nested
            .levels
            .insert(level, f(NestedBandLevelConfig::new(level_spec)).into_spec());
        Self {
            inner: self.inner.with_nested_band_config(nested),
            axis_config: self.axis_config,
        }
    }

    /// Set the boundary for a specific nested categorical band level.
    pub fn level_band(self, level: usize, band: f64) -> Self {
        Self {
            inner: self
                .inner
                .with_position_boundary(PositionBoundary::level_band(level, band)),
            axis_config: self.axis_config,
        }
    }

    /// Set a custom scale name.
    pub fn with_scale_name(self, name: impl Into<String>) -> Self {
        Self {
            inner: self.inner.with_scale_name(name),
            axis_config: self.axis_config,
        }
    }

    /// Get the inner ChannelValue.
    pub fn into_inner(self) -> ChannelValue {
        self.inner
    }

    /// Get the axis configuration if present.
    pub fn axis_config(&self) -> Option<&A> {
        self.axis_config.as_ref()
    }
}

impl<A: Clone + Default + Send + Sync + 'static> ChannelConfig for GenericPositionConfig<A> {
    fn get_value(&self) -> &ChannelValue {
        &self.inner
    }

    fn set_value(&mut self, value: ChannelValue) {
        self.inner = value;
    }

    fn into_inner(self) -> ChannelValue {
        self.inner
    }
}

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

impl<A: Clone + Default + Send + Sync + 'static> PositionConfig for GenericPositionConfig<A> {
    type Axis = A;

    fn new(value: ChannelValue) -> Self {
        Self::new(value)
    }

    fn take_axis_config(self) -> (ChannelValue, Option<Self::Axis>) {
        (self.inner, self.axis_config)
    }

    fn into_inner(self) -> ChannelValue {
        self.inner
    }
}

impl<A: Clone + Default + Send + Sync + 'static> From<ChannelValue> for GenericPositionConfig<A> {
    fn from(value: ChannelValue) -> Self {
        Self::new(value)
    }
}

impl<A: Clone + Default + Send + Sync + 'static> From<Expr> for GenericPositionConfig<A> {
    fn from(expr: Expr) -> Self {
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

#[cfg(test)]
mod tests {
    use datafusion::prelude::col;

    use crate::{NestScope, PositionBoundary};

    use super::*;

    #[test]
    fn nested_band_position_config_level_and_level_band() {
        let config = GenericPositionConfig::<()>::new(ChannelValue::from(col("x")))
            .level(1, |level| {
                level
                    .nest_scope(NestScope::Shared)
                    .padding_inner(0.0)
                    .padding_outer_px(4.0)
            })
            .level_band(1, 1.0);

        let nested = config
            .inner
            .get_nested_band_config()
            .expect("nested config");
        let level = nested.level(1).expect("level config");
        assert_eq!(level.nest_scope, Some(NestScope::Shared));
        assert_eq!(level.padding_inner, Some(0.0));
        assert_eq!(level.padding_outer_px, Some(4.0));
        assert_eq!(
            config.inner.get_position_boundary(),
            Some(PositionBoundary::LevelBand {
                level: 1,
                band: 1.0
            })
        );
    }
}
