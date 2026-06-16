use avenger_chart_core::{
    AngleChannelConfig, ChannelConfig, ChannelExpr, ChannelValue, ColorChannelConfig, Legend,
    OpacityChannelConfig, ShapeChannelConfig, SizeChannelConfig, StrokeDashChannelConfig,
    StrokeWidthChannelConfig,
};

use crate::{
    AngleLegendBuilder, ColorLegendBuilder, LegendBuilder, OpacityLegendBuilder,
    ShapeLegendBuilder, SizeLegendBuilder, StrokeDashLegendBuilder, StrokeWidthLegendBuilder,
};

/// Extension methods for channel configs that support legends.
pub trait LegendableChannel: ChannelConfig {
    /// The type of legend builder for this channel.
    type LegendBuilder: LegendBuilder + Default;

    /// Configure the legend for this channel.
    fn legend<F>(mut self, f: F) -> Self
    where
        F: FnOnce(Self::LegendBuilder) -> Self::LegendBuilder,
    {
        let builder = Self::LegendBuilder::default();
        let configured = f(builder);
        let legend = configured.build();

        let new_value = apply_legend_config(self.get_value().clone(), legend);
        self.set_value(new_value);
        self
    }

    /// Disable legend for this channel.
    fn no_legend(self) -> Self {
        self.legend(|l| l.visible(false))
    }
}

fn apply_legend_config(value: ChannelValue, legend_config: Legend) -> ChannelValue {
    let legend_config = Box::new(legend_config);
    match value {
        ChannelValue::Scaled {
            expr,
            scale_name,
            position_boundary,
            scale_config,
            nested_band_config,
            axis_config,
            domain_coordination,
            transform_scope,
            ..
        } => ChannelValue::Scaled {
            expr,
            scale_name,
            position_boundary,
            scale_config,
            nested_band_config,
            legend_config: Some(legend_config),
            axis_config,
            domain_coordination,
            transform_scope,
        },
        ChannelValue::Conditional {
            conditions,
            otherwise,
            scale_config,
            nested_band_config,
            axis_config,
            domain_coordination,
            transform_scope,
            ..
        } => ChannelValue::Conditional {
            conditions,
            otherwise,
            scale_config,
            nested_band_config,
            legend_config: Some(legend_config),
            axis_config,
            domain_coordination,
            transform_scope,
        },
        ChannelValue::Value { .. } => value,
    }
}

/// Extension methods for raw channel values that carry legend configuration.
pub trait LegendableChannelValue {
    /// Attach a prebuilt legend spec to this channel value.
    fn legend(self, legend_config: Legend) -> Self;

    /// Disable legend rendering for this channel value.
    fn no_legend(self) -> Self;
}

impl LegendableChannelValue for ChannelValue {
    fn legend(self, legend_config: Legend) -> Self {
        apply_legend_config(self, legend_config)
    }

    fn no_legend(self) -> Self {
        self.legend(Legend::new().visible(false))
    }
}

impl LegendableChannelValue for ChannelExpr {
    fn legend(self, legend_config: Legend) -> Self {
        self.map_channel_value(|value| apply_legend_config(value, legend_config))
    }

    fn no_legend(self) -> Self {
        self.legend(Legend::new().visible(false))
    }
}

impl LegendableChannel for ColorChannelConfig {
    type LegendBuilder = ColorLegendBuilder;
}

impl LegendableChannel for SizeChannelConfig {
    type LegendBuilder = SizeLegendBuilder;
}

impl LegendableChannel for ShapeChannelConfig {
    type LegendBuilder = ShapeLegendBuilder;
}

impl LegendableChannel for OpacityChannelConfig {
    type LegendBuilder = OpacityLegendBuilder;
}

impl LegendableChannel for AngleChannelConfig {
    type LegendBuilder = AngleLegendBuilder;
}

impl LegendableChannel for StrokeWidthChannelConfig {
    type LegendBuilder = StrokeWidthLegendBuilder;
}

impl LegendableChannel for StrokeDashChannelConfig {
    type LegendBuilder = StrokeDashLegendBuilder;
}
