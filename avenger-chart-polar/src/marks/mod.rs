pub mod line;
pub mod subplot;
pub mod symbol;

use std::sync::Arc;

use avenger_chart_core::{ChannelValue, GeometrySpace, PositionConfig};
use avenger_chart_marks::{Line, Symbol};

use crate::{Polar, PolarPositionConfig};

pub use line::CompiledPolarLine;
pub use subplot::{
    CompiledPolarSubplot, POLAR_SUBPLOT_PARTITION_CHANNEL, PolarSubplotPositionChannels,
};
pub use symbol::CompiledPolarSymbol;

/// Polar position-channel builders for the generic `Symbol` mark.
pub trait PolarSymbolPositionChannels: Sized {
    fn r<V: Into<ChannelValue>>(self, value: V) -> Self;
    fn r_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(PolarPositionConfig) -> PolarPositionConfig;
    fn theta<V: Into<ChannelValue>>(self, value: V) -> Self;
    fn theta_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(PolarPositionConfig) -> PolarPositionConfig;
}

impl PolarSymbolPositionChannels for Symbol<Polar> {
    fn r<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("r", value.into())
    }

    fn r_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(PolarPositionConfig) -> PolarPositionConfig,
    {
        configure_polar_position_channel(self, "r", value.into(), f)
    }

    fn theta<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("theta", value.into())
    }

    fn theta_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(PolarPositionConfig) -> PolarPositionConfig,
    {
        configure_polar_position_channel(self, "theta", value.into(), f)
    }
}

fn configure_polar_position_channel(
    mark: Symbol<Polar>,
    channel_name: &str,
    channel_value: ChannelValue,
    f: impl FnOnce(PolarPositionConfig) -> PolarPositionConfig,
) -> Symbol<Polar> {
    let config = PolarPositionConfig::new(channel_value);
    let configured = f(config);
    let (channel_value, axis_config) = configured.take_axis_config();
    let mut mark = mark.with_channel_value(channel_name, channel_value);
    if let Some(axis_config) = axis_config {
        mark.state_mut()
            .axis_configs
            .insert(channel_name.to_string(), Arc::new(axis_config));
    }
    mark
}

/// Polar position-channel builders for the generic `Line` mark.
pub trait PolarLinePositionChannels: Sized {
    fn r<V: Into<ChannelValue>>(self, value: V) -> Self;
    fn r_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(PolarPositionConfig) -> PolarPositionConfig;
    fn theta<V: Into<ChannelValue>>(self, value: V) -> Self;
    fn theta_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(PolarPositionConfig) -> PolarPositionConfig;
    fn geometry_space(self, geometry_space: GeometrySpace) -> Self;
}

impl PolarLinePositionChannels for Line<Polar> {
    fn r<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("r", value.into())
    }

    fn r_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(PolarPositionConfig) -> PolarPositionConfig,
    {
        configure_line_polar_position_channel(self, "r", value.into(), f)
    }

    fn theta<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("theta", value.into())
    }

    fn theta_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(PolarPositionConfig) -> PolarPositionConfig,
    {
        configure_line_polar_position_channel(self, "theta", value.into(), f)
    }

    fn geometry_space(mut self, geometry_space: GeometrySpace) -> Self {
        self.state_mut().geometry_space = Some(geometry_space);
        self
    }
}

fn configure_line_polar_position_channel(
    mark: Line<Polar>,
    channel_name: &str,
    channel_value: ChannelValue,
    f: impl FnOnce(PolarPositionConfig) -> PolarPositionConfig,
) -> Line<Polar> {
    let config = PolarPositionConfig::new(channel_value);
    let configured = f(config);
    let (channel_value, axis_config) = configured.take_axis_config();
    let mut mark = mark.with_channel_value(channel_name, channel_value);
    if let Some(axis_config) = axis_config {
        mark.state_mut()
            .axis_configs
            .insert(channel_name.to_string(), Arc::new(axis_config));
    }
    mark
}
