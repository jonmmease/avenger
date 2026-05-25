pub mod line;
pub mod rect;
pub mod symbol;

use std::sync::Arc;

use avenger_chart_core::{ChannelValue, PositionConfig};
use avenger_chart_marks::{Line, Rect, Symbol};

use crate::{Cartesian, CartesianPositionConfig};

pub use line::{CompiledCartesianLine, ensure_dictionary_array_fn};
pub use rect::CompiledCartesianRect;
pub use symbol::CompiledCartesianSymbol;

/// Cartesian position-channel builders for the generic `Line` mark.
pub trait CartesianLinePositionChannels: Sized {
    fn x<V: Into<ChannelValue>>(self, value: V) -> Self;
    fn x_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(CartesianPositionConfig) -> CartesianPositionConfig;
    fn y<V: Into<ChannelValue>>(self, value: V) -> Self;
    fn y_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(CartesianPositionConfig) -> CartesianPositionConfig;
}

impl CartesianLinePositionChannels for Line<Cartesian> {
    fn x<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("x", value.into())
    }

    fn x_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(CartesianPositionConfig) -> CartesianPositionConfig,
    {
        configure_line_position_channel(self, "x", value.into(), f)
    }

    fn y<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("y", value.into())
    }

    fn y_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(CartesianPositionConfig) -> CartesianPositionConfig,
    {
        configure_line_position_channel(self, "y", value.into(), f)
    }
}

fn configure_line_position_channel(
    mark: Line<Cartesian>,
    channel_name: &str,
    channel_value: ChannelValue,
    f: impl FnOnce(CartesianPositionConfig) -> CartesianPositionConfig,
) -> Line<Cartesian> {
    let config = CartesianPositionConfig::new(channel_value);
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

/// Cartesian position-channel builders for the generic `Rect` mark.
pub trait CartesianRectPositionChannels: Sized {
    fn x<V: Into<ChannelValue>>(self, value: V) -> Self;
    fn x_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(CartesianPositionConfig) -> CartesianPositionConfig;
    fn x2<V: Into<ChannelValue>>(self, value: V) -> Self;
    fn x2_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(CartesianPositionConfig) -> CartesianPositionConfig;
    fn y<V: Into<ChannelValue>>(self, value: V) -> Self;
    fn y_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(CartesianPositionConfig) -> CartesianPositionConfig;
    fn y2<V: Into<ChannelValue>>(self, value: V) -> Self;
    fn y2_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(CartesianPositionConfig) -> CartesianPositionConfig;
}

impl CartesianRectPositionChannels for Rect<Cartesian> {
    fn x<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("x", value.into())
    }

    fn x_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(CartesianPositionConfig) -> CartesianPositionConfig,
    {
        configure_rect_position_channel(self, "x", value.into(), f)
    }

    fn x2<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("x2", value.into())
    }

    fn x2_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(CartesianPositionConfig) -> CartesianPositionConfig,
    {
        configure_rect_position_channel(self, "x2", value.into(), f)
    }

    fn y<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("y", value.into())
    }

    fn y_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(CartesianPositionConfig) -> CartesianPositionConfig,
    {
        configure_rect_position_channel(self, "y", value.into(), f)
    }

    fn y2<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("y2", value.into())
    }

    fn y2_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(CartesianPositionConfig) -> CartesianPositionConfig,
    {
        configure_rect_position_channel(self, "y2", value.into(), f)
    }
}

fn configure_rect_position_channel(
    mark: Rect<Cartesian>,
    channel_name: &str,
    channel_value: ChannelValue,
    f: impl FnOnce(CartesianPositionConfig) -> CartesianPositionConfig,
) -> Rect<Cartesian> {
    let config = CartesianPositionConfig::new(channel_value);
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

/// Cartesian position-channel builders for the generic `Symbol` mark.
pub trait CartesianSymbolPositionChannels: Sized {
    fn x<V: Into<ChannelValue>>(self, value: V) -> Self;
    fn x_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(CartesianPositionConfig) -> CartesianPositionConfig;
    fn y<V: Into<ChannelValue>>(self, value: V) -> Self;
    fn y_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(CartesianPositionConfig) -> CartesianPositionConfig;
}

impl CartesianSymbolPositionChannels for Symbol<Cartesian> {
    fn x<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("x", value.into())
    }

    fn x_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(CartesianPositionConfig) -> CartesianPositionConfig,
    {
        configure_symbol_position_channel(self, "x", value.into(), f)
    }

    fn y<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("y", value.into())
    }

    fn y_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(CartesianPositionConfig) -> CartesianPositionConfig,
    {
        configure_symbol_position_channel(self, "y", value.into(), f)
    }
}

fn configure_symbol_position_channel(
    mark: Symbol<Cartesian>,
    channel_name: &str,
    channel_value: ChannelValue,
    f: impl FnOnce(CartesianPositionConfig) -> CartesianPositionConfig,
) -> Symbol<Cartesian> {
    let config = CartesianPositionConfig::new(channel_value);
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
