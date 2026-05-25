use crate::{Auto, ChannelConfig, ChannelValue, Scale, ScaleSharing, ScaleSpec};

/// Extension methods for channel configs that carry scale configuration.
pub trait ScaleChannelConfig: ChannelConfig {
    /// Configure the scale for this channel.
    fn scale<F>(mut self, f: F) -> Self
    where
        F: Fn(Scale<Auto>) -> Scale<Auto> + Send + Sync + 'static,
    {
        let scale_changes = f(Scale::new());
        let new_value = apply_scale_config(self.get_value().clone(), scale_changes);
        self.set_value(new_value);
        self
    }

    /// Configure the scale with explicit type.
    fn scale_with<S: ScaleSpec + Default>(
        self,
        f: impl Fn(Scale<S>) -> Scale<S> + Send + Sync + 'static,
    ) -> Self {
        self.scale(move |default_scale| {
            let typed_scale = default_scale.into_type::<S>();
            f(typed_scale).into_auto()
        })
    }

    /// Configure how this channel's plot scale domain is shared across facets.
    ///
    /// This applies to data channels such as x, y, color, and size. Facet row
    /// and column channels use the same `ScaleSharing` values to configure
    /// facet slot sharing, but expose that through facet-specific
    /// `with_slot_sharing` options.
    fn with_scale_sharing(mut self, mode: ScaleSharing) -> Self {
        let normalized = mode.to_normalized();
        let mut value = self.get_value().clone();
        value = match value {
            ChannelValue::Scaled {
                expr,
                scale_name,
                band,
                scale_config,
                legend_config,
                ..
            } => ChannelValue::Scaled {
                expr,
                scale_name,
                band,
                scale_config,
                legend_config,
                share_mode: Some(normalized),
            },
            ChannelValue::Conditional {
                conditions,
                otherwise,
                scale_config,
                legend_config,
                ..
            } => ChannelValue::Conditional {
                conditions,
                otherwise,
                scale_config,
                legend_config,
                share_mode: Some(normalized),
            },
            other => other,
        };
        self.set_value(value);
        self
    }

    /// Share this channel's plot scale domain across all facets.
    fn share_scale(self) -> Self {
        self.with_scale_sharing(ScaleSharing::Shared)
    }

    /// Make this channel's plot scale domain independent for each facet.
    fn free_scale(self) -> Self {
        self.with_scale_sharing(ScaleSharing::Free)
    }
}

impl<T: ChannelConfig> ScaleChannelConfig for T {}

/// Extension methods for raw channel values that carry scale configuration.
pub trait ScaleChannelValue {
    /// Configure the scale for this channel value.
    fn scale<F>(self, f: F) -> Self
    where
        F: Fn(Scale<Auto>) -> Scale<Auto> + Send + Sync + 'static;

    /// Configure the scale with explicit type.
    fn scale_with<S: ScaleSpec + Default>(
        self,
        f: impl Fn(Scale<S>) -> Scale<S> + Send + Sync + 'static,
    ) -> Self;
}

impl ScaleChannelValue for ChannelValue {
    fn scale<F>(self, f: F) -> Self
    where
        F: Fn(Scale<Auto>) -> Scale<Auto> + Send + Sync + 'static,
    {
        apply_scale_config(self, f(Scale::new()))
    }

    fn scale_with<S: ScaleSpec + Default>(
        self,
        f: impl Fn(Scale<S>) -> Scale<S> + Send + Sync + 'static,
    ) -> Self {
        self.scale(move |default_scale| {
            let typed_scale = default_scale.into_type::<S>();
            f(typed_scale).into_auto()
        })
    }
}

fn apply_scale_config(value: ChannelValue, scale_config: Scale<Auto>) -> ChannelValue {
    let scale_config = scale_config.into_config();
    match value {
        ChannelValue::Scaled {
            expr,
            scale_name,
            band,
            legend_config,
            share_mode,
            ..
        } => ChannelValue::Scaled {
            expr,
            scale_name,
            band,
            scale_config: Some(scale_config),
            legend_config,
            share_mode,
        },
        ChannelValue::Conditional {
            conditions,
            otherwise,
            legend_config,
            share_mode,
            ..
        } => ChannelValue::Conditional {
            conditions,
            otherwise,
            scale_config: Some(scale_config),
            legend_config,
            share_mode,
        },
        ChannelValue::Value { .. } => value,
    }
}

#[cfg(test)]
mod tests {
    use datafusion::prelude::{col, lit};

    use crate::ChannelValue;

    use super::ScaleChannelValue;

    #[test]
    fn channel_value_scale_config_serialization_roundtrip() {
        let cv: ChannelValue = col("x").into();
        let cv = cv.scale(|s| {
            s.domain_interval(lit(0.0), lit(10.0))
                .range_interval(lit(0.0), lit(100.0))
        });

        let json = serde_json::to_string(&cv).expect("serialize channel value");
        let decoded: ChannelValue = serde_json::from_str(&json).expect("deserialize channel value");
        let scale_config = decoded.get_scale_config().expect("scale config");

        assert!(scale_config.domain.is_set());
        assert!(scale_config.range.is_set());
    }
}
