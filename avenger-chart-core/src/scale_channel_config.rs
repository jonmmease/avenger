use crate::{
    Auto, ChannelConfig, ChannelExpr, ChannelValue, CoordinationScope, DomainCoordination, Scale,
    ScaleSpec,
};

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

    /// Configure this channel's scale-domain owner scope.
    ///
    /// This applies to data channels such as x, y, color, and size. Facet row
    /// and column channels use the same `CoordinationScope` values to configure
    /// facet slot sharing, but expose that through facet-specific
    /// `with_slot_sharing` options.
    fn with_domain_scope(mut self, mode: CoordinationScope) -> Self {
        let value = self.get_value().clone().with_domain_scope(mode);
        self.set_value(value);
        self
    }

    /// Configure this channel's semantic domain group.
    fn with_domain_group(mut self, group: impl Into<String>) -> Self {
        let value = self.get_value().clone().with_domain_group(group);
        self.set_value(value);
        self
    }

    /// Configure this channel's full domain coordination target.
    fn with_domain_coordination(mut self, coordination: DomainCoordination) -> Self {
        let value = self
            .get_value()
            .clone()
            .with_domain_coordination(coordination);
        self.set_value(value);
        self
    }

    /// Share this channel's plot scale domain across all facets.
    fn share_domain(self) -> Self {
        self.with_domain_scope(CoordinationScope::Shared)
    }

    /// Make this channel's plot scale domain independent for each facet.
    fn free_domain(self) -> Self {
        self.with_domain_scope(CoordinationScope::Free)
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

impl ScaleChannelValue for ChannelExpr {
    fn scale<F>(self, f: F) -> Self
    where
        F: Fn(Scale<Auto>) -> Scale<Auto> + Send + Sync + 'static,
    {
        self.map_channel_value(|value| apply_scale_config(value, f(Scale::new())))
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
    let scale_config = Box::new(scale_config.into_config());
    match value {
        ChannelValue::Scaled {
            expr,
            scale_name,
            band,
            legend_config,
            axis_config,
            domain_coordination,
            transform_scope,
            ..
        } => ChannelValue::Scaled {
            expr,
            scale_name,
            band,
            scale_config: Some(scale_config),
            legend_config,
            axis_config,
            domain_coordination,
            transform_scope,
        },
        ChannelValue::Conditional {
            conditions,
            otherwise,
            legend_config,
            axis_config,
            domain_coordination,
            transform_scope,
            ..
        } => ChannelValue::Conditional {
            conditions,
            otherwise,
            scale_config: Some(scale_config),
            legend_config,
            axis_config,
            domain_coordination,
            transform_scope,
        },
        ChannelValue::Value { .. } => value,
    }
}

#[cfg(test)]
mod tests {
    use datafusion::prelude::{col, lit};

    use crate::{
        ChannelConfig, ChannelValue, CoordinationScope, DomainCoordination,
        DomainCoordinationGroup, ScaleChannelConfig,
    };

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

    #[test]
    fn channel_value_domain_group_preserves_scope() {
        let cv: ChannelValue = col("x").into();
        let cv = cv
            .with_domain_scope(CoordinationScope::Level(1))
            .with_domain_group("height");

        let coordination = cv
            .get_domain_coordination()
            .expect("domain coordination metadata");
        assert_eq!(coordination.scope, CoordinationScope::Level(1));
        assert_eq!(
            coordination.group,
            DomainCoordinationGroup::Named("height".to_string())
        );
    }

    #[test]
    fn channel_value_domain_coordination_sets_full_target() {
        let cv: ChannelValue = col("x").into();
        let cv = cv.with_domain_coordination(
            DomainCoordination::named(CoordinationScope::Shared, "height").unwrap(),
        );

        let coordination = cv
            .get_domain_coordination()
            .expect("domain coordination metadata");
        assert_eq!(coordination.scope, CoordinationScope::Level(u8::MAX));
        assert_eq!(
            coordination.group,
            DomainCoordinationGroup::Named("height".to_string())
        );
    }

    #[test]
    fn channel_config_domain_group_preserves_scope() {
        let config = crate::GenericPositionConfig::<crate::NoGuide>::new(col("x").into())
            .with_domain_group("height")
            .with_domain_scope(CoordinationScope::Level(1));

        let coordination = config
            .get_value()
            .get_domain_coordination()
            .expect("domain coordination metadata");
        assert_eq!(coordination.scope, CoordinationScope::Level(1));
        assert_eq!(
            coordination.group,
            DomainCoordinationGroup::Named("height".to_string())
        );
    }
}
