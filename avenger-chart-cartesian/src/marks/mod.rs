pub mod area;
mod detail;
pub mod image;
pub mod line;
pub mod path;
pub mod rect;
pub mod rule;
pub mod subplot;
pub mod symbol;
pub mod text;
pub mod trail;
pub mod uniform_raster_2d;
mod util;

use std::sync::Arc;

use avenger_chart_core::{ChannelValue, ColorChannelConfig, IntoExpr, PositionConfig};
use avenger_chart_marks::{
    Area, Image, Line, PathMark, RasterChannelsConfig, Rect, Rule, Symbol, Text, Trail,
    UniformRaster2D,
};

use crate::{Cartesian, CartesianAxis, CartesianPositionConfig};

pub use area::CompiledCartesianArea;
pub use image::CompiledCartesianImage;
pub use line::{CompiledCartesianLine, ensure_dictionary_array_fn};
pub use path::CompiledCartesianPath;
pub use rect::CompiledCartesianRect;
pub use rule::CompiledCartesianRule;
pub use subplot::{
    CARTESIAN_SUBPLOT_PARTITION_CHANNEL, CARTESIAN_SUBPLOT_X_CHANNEL, CARTESIAN_SUBPLOT_Y_CHANNEL,
    CartesianSubplotPositionChannels, CompiledCartesianSubplot,
};
pub use symbol::CompiledCartesianSymbol;
pub use text::CompiledCartesianText;
pub use trail::CompiledCartesianTrail;
pub use uniform_raster_2d::CompiledCartesianUniformRaster2D;

/// Cartesian position-channel builders for the generic `Area` mark.
pub trait CartesianAreaPositionChannels: Sized {
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

impl CartesianAreaPositionChannels for Area<Cartesian> {
    fn x<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("x", value.into())
    }

    fn x_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(CartesianPositionConfig) -> CartesianPositionConfig,
    {
        configure_area_position_channel(self, "x", value.into(), f)
    }

    fn x2<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("x2", value.into())
    }

    fn x2_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(CartesianPositionConfig) -> CartesianPositionConfig,
    {
        configure_area_position_channel(self, "x2", value.into(), f)
    }

    fn y<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("y", value.into())
    }

    fn y_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(CartesianPositionConfig) -> CartesianPositionConfig,
    {
        configure_area_position_channel(self, "y", value.into(), f)
    }

    fn y2<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("y2", value.into())
    }

    fn y2_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(CartesianPositionConfig) -> CartesianPositionConfig,
    {
        configure_area_position_channel(self, "y2", value.into(), f)
    }
}

fn configure_area_position_channel(
    mark: Area<Cartesian>,
    channel_name: &str,
    channel_value: ChannelValue,
    f: impl FnOnce(CartesianPositionConfig) -> CartesianPositionConfig,
) -> Area<Cartesian> {
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

/// Cartesian position-channel builders for the generic `Image` mark.
pub trait CartesianImagePositionChannels: Sized {
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

impl CartesianImagePositionChannels for Image<Cartesian> {
    fn x<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("x", value.into())
    }

    fn x_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(CartesianPositionConfig) -> CartesianPositionConfig,
    {
        configure_image_position_channel(self, "x", value.into(), f)
    }

    fn y<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("y", value.into())
    }

    fn y_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(CartesianPositionConfig) -> CartesianPositionConfig,
    {
        configure_image_position_channel(self, "y", value.into(), f)
    }
}

fn configure_image_position_channel(
    mark: Image<Cartesian>,
    channel_name: &str,
    channel_value: ChannelValue,
    f: impl FnOnce(CartesianPositionConfig) -> CartesianPositionConfig,
) -> Image<Cartesian> {
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

/// Cartesian raster-channel builders for the generic `UniformRaster2D` mark.
pub trait CartesianUniformRaster2DChannels: Sized {
    fn raster_with<V, F>(self, data: V, f: F) -> Self
    where
        V: IntoExpr,
        F: FnOnce(RasterChannelsConfig<CartesianAxis>) -> RasterChannelsConfig<CartesianAxis>;
}

impl CartesianUniformRaster2DChannels for UniformRaster2D<Cartesian> {
    fn raster_with<V, F>(self, data: V, f: F) -> Self
    where
        V: IntoExpr,
        F: FnOnce(RasterChannelsConfig<CartesianAxis>) -> RasterChannelsConfig<CartesianAxis>,
    {
        let raster_expr = data.into_expr();
        let fields = avenger_chart_marks::UniformRaster2DFields::new(raster_expr.clone());
        let config = RasterChannelsConfig::new(ColorChannelConfig::new(ChannelValue::from(
            fields.values_data(),
        )));
        let parts = f(config).into_parts();
        let (x_position, x_axis_config) = match parts.x {
            Some(x) => {
                let (position, axis_config) = x.take();
                (Some(position), axis_config)
            }
            None => (None, None),
        };
        let (y_position, y_axis_config) = match parts.y {
            Some(y) => {
                let (position, axis_config) = y.take();
                (Some(position), axis_config)
            }
            None => (None, None),
        };
        let mut mark = self.configure_raster_with_overlay(
            raster_expr,
            Some(parts.fill),
            Some((x_position, y_position)),
            parts.fill_by,
            parts.opacity_by_total,
        );
        if let Some(axis_config) = x_axis_config {
            mark.state_mut()
                .axis_configs
                .insert("x".to_string(), Arc::new(axis_config));
        }
        if let Some(axis_config) = y_axis_config {
            mark.state_mut()
                .axis_configs
                .insert("y".to_string(), Arc::new(axis_config));
        }
        mark
    }
}

/// Cartesian position-channel builders for the generic `PathMark` mark.
pub trait CartesianPathPositionChannels: Sized {
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

impl CartesianPathPositionChannels for PathMark<Cartesian> {
    fn x<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("x", value.into())
    }

    fn x_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(CartesianPositionConfig) -> CartesianPositionConfig,
    {
        configure_path_position_channel(self, "x", value.into(), f)
    }

    fn y<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("y", value.into())
    }

    fn y_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(CartesianPositionConfig) -> CartesianPositionConfig,
    {
        configure_path_position_channel(self, "y", value.into(), f)
    }
}

fn configure_path_position_channel(
    mark: PathMark<Cartesian>,
    channel_name: &str,
    channel_value: ChannelValue,
    f: impl FnOnce(CartesianPositionConfig) -> CartesianPositionConfig,
) -> PathMark<Cartesian> {
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

/// Cartesian position-channel builders for the generic `Trail` mark.
pub trait CartesianTrailPositionChannels: Sized {
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

impl CartesianTrailPositionChannels for Trail<Cartesian> {
    fn x<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("x", value.into())
    }

    fn x_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(CartesianPositionConfig) -> CartesianPositionConfig,
    {
        configure_trail_position_channel(self, "x", value.into(), f)
    }

    fn y<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("y", value.into())
    }

    fn y_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(CartesianPositionConfig) -> CartesianPositionConfig,
    {
        configure_trail_position_channel(self, "y", value.into(), f)
    }
}

fn configure_trail_position_channel(
    mark: Trail<Cartesian>,
    channel_name: &str,
    channel_value: ChannelValue,
    f: impl FnOnce(CartesianPositionConfig) -> CartesianPositionConfig,
) -> Trail<Cartesian> {
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

/// Cartesian position-channel builders for the generic `Rule` mark.
pub trait CartesianRulePositionChannels: Sized {
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

impl CartesianRulePositionChannels for Rule<Cartesian> {
    fn x<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("x", value.into())
    }

    fn x_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(CartesianPositionConfig) -> CartesianPositionConfig,
    {
        configure_rule_position_channel(self, "x", value.into(), f)
    }

    fn x2<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("x2", value.into())
    }

    fn x2_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(CartesianPositionConfig) -> CartesianPositionConfig,
    {
        configure_rule_position_channel(self, "x2", value.into(), f)
    }

    fn y<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("y", value.into())
    }

    fn y_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(CartesianPositionConfig) -> CartesianPositionConfig,
    {
        configure_rule_position_channel(self, "y", value.into(), f)
    }

    fn y2<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("y2", value.into())
    }

    fn y2_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(CartesianPositionConfig) -> CartesianPositionConfig,
    {
        configure_rule_position_channel(self, "y2", value.into(), f)
    }
}

fn configure_rule_position_channel(
    mark: Rule<Cartesian>,
    channel_name: &str,
    channel_value: ChannelValue,
    f: impl FnOnce(CartesianPositionConfig) -> CartesianPositionConfig,
) -> Rule<Cartesian> {
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

/// Cartesian position-channel builders for the generic `Text` mark.
pub trait CartesianTextPositionChannels: Sized {
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

impl CartesianTextPositionChannels for Text<Cartesian> {
    fn x<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("x", value.into())
    }

    fn x_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(CartesianPositionConfig) -> CartesianPositionConfig,
    {
        configure_text_position_channel(self, "x", value.into(), f)
    }

    fn y<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("y", value.into())
    }

    fn y_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(CartesianPositionConfig) -> CartesianPositionConfig,
    {
        configure_text_position_channel(self, "y", value.into(), f)
    }
}

fn configure_text_position_channel(
    mark: Text<Cartesian>,
    channel_name: &str,
    channel_value: ChannelValue,
    f: impl FnOnce(CartesianPositionConfig) -> CartesianPositionConfig,
) -> Text<Cartesian> {
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
