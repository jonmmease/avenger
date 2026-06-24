pub mod rect;
pub mod symbol;

use std::sync::Arc;

use avenger_chart_core::{ChannelExpr, ChannelValue, GenericPositionConfig, PositionConfig};
use avenger_chart_marks::Symbol;
use datafusion::{
    functions::expr_fn::{ln, radians, tan},
    logical_expr::{Expr, lit, when},
    prelude::col,
};

use crate::{
    WebMercator,
    projection::{EARTH_RADIUS_M, WEB_MERCATOR_MAX_LAT},
};

pub use rect::CompiledWebMercatorRect;
pub use symbol::CompiledWebMercatorSymbol;

pub type WebMercatorPositionConfig = GenericPositionConfig<()>;

pub trait IntoWebMercatorExpr {
    fn into_webmercator_expr(self) -> Expr;
}

impl IntoWebMercatorExpr for Expr {
    fn into_webmercator_expr(self) -> Expr {
        self
    }
}

impl IntoWebMercatorExpr for &Expr {
    fn into_webmercator_expr(self) -> Expr {
        self.clone()
    }
}

impl IntoWebMercatorExpr for ChannelExpr {
    fn into_webmercator_expr(self) -> Expr {
        self.into_data_expr()
    }
}

impl IntoWebMercatorExpr for &ChannelExpr {
    fn into_webmercator_expr(self) -> Expr {
        self.data_expr().clone()
    }
}

impl IntoWebMercatorExpr for &str {
    fn into_webmercator_expr(self) -> Expr {
        col(self)
    }
}

impl IntoWebMercatorExpr for String {
    fn into_webmercator_expr(self) -> Expr {
        col(self)
    }
}

impl IntoWebMercatorExpr for f64 {
    fn into_webmercator_expr(self) -> Expr {
        lit(self)
    }
}

impl IntoWebMercatorExpr for f32 {
    fn into_webmercator_expr(self) -> Expr {
        lit(self)
    }
}

impl IntoWebMercatorExpr for i32 {
    fn into_webmercator_expr(self) -> Expr {
        lit(self)
    }
}

impl IntoWebMercatorExpr for i64 {
    fn into_webmercator_expr(self) -> Expr {
        lit(self)
    }
}

pub trait WebMercatorSymbolPositionChannels: Sized {
    fn projected_x<V: Into<ChannelValue>>(self, value: V) -> Self;
    fn projected_x_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(WebMercatorPositionConfig) -> WebMercatorPositionConfig;
    fn projected_y<V: Into<ChannelValue>>(self, value: V) -> Self;
    fn projected_y_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(WebMercatorPositionConfig) -> WebMercatorPositionConfig;
    fn longitude<V: IntoWebMercatorExpr>(self, value: V) -> Self;
    fn longitude_with<V, F>(self, value: V, f: F) -> Self
    where
        V: IntoWebMercatorExpr,
        F: FnOnce(WebMercatorPositionConfig) -> WebMercatorPositionConfig;
    fn latitude<V: IntoWebMercatorExpr>(self, value: V) -> Self;
    fn latitude_with<V, F>(self, value: V, f: F) -> Self
    where
        V: IntoWebMercatorExpr,
        F: FnOnce(WebMercatorPositionConfig) -> WebMercatorPositionConfig;
}

impl WebMercatorSymbolPositionChannels for Symbol<WebMercator> {
    fn projected_x<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("x", value.into())
    }

    fn projected_x_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(WebMercatorPositionConfig) -> WebMercatorPositionConfig,
    {
        configure_position_channel(self, "x", value.into(), f)
    }

    fn projected_y<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("y", value.into())
    }

    fn projected_y_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(WebMercatorPositionConfig) -> WebMercatorPositionConfig,
    {
        configure_position_channel(self, "y", value.into(), f)
    }

    fn longitude<V: IntoWebMercatorExpr>(self, value: V) -> Self {
        self.with_channel_value(
            "x",
            ChannelValue::from(longitude_expr(value.into_webmercator_expr())),
        )
    }

    fn longitude_with<V, F>(self, value: V, f: F) -> Self
    where
        V: IntoWebMercatorExpr,
        F: FnOnce(WebMercatorPositionConfig) -> WebMercatorPositionConfig,
    {
        configure_position_channel(
            self,
            "x",
            ChannelValue::from(longitude_expr(value.into_webmercator_expr())),
            f,
        )
    }

    fn latitude<V: IntoWebMercatorExpr>(self, value: V) -> Self {
        self.with_channel_value(
            "y",
            ChannelValue::from(latitude_expr(value.into_webmercator_expr())),
        )
    }

    fn latitude_with<V, F>(self, value: V, f: F) -> Self
    where
        V: IntoWebMercatorExpr,
        F: FnOnce(WebMercatorPositionConfig) -> WebMercatorPositionConfig,
    {
        configure_position_channel(
            self,
            "y",
            ChannelValue::from(latitude_expr(value.into_webmercator_expr())),
            f,
        )
    }
}

fn configure_position_channel(
    mark: Symbol<WebMercator>,
    channel_name: &str,
    channel_value: ChannelValue,
    f: impl FnOnce(WebMercatorPositionConfig) -> WebMercatorPositionConfig,
) -> Symbol<WebMercator> {
    let config = WebMercatorPositionConfig::new(channel_value);
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

fn longitude_expr(lon: Expr) -> Expr {
    lit(EARTH_RADIUS_M) * radians(lon)
}

fn latitude_expr(lat: Expr) -> Expr {
    let clamped = clamp_latitude_expr(lat);
    lit(EARTH_RADIUS_M)
        * ln(tan(
            lit(std::f64::consts::FRAC_PI_4) + radians(clamped) / lit(2.0)
        ))
}

fn clamp_latitude_expr(lat: Expr) -> Expr {
    when(
        lat.clone().gt(lit(WEB_MERCATOR_MAX_LAT)),
        lit(WEB_MERCATOR_MAX_LAT),
    )
    .when(
        lat.clone().lt(lit(-WEB_MERCATOR_MAX_LAT)),
        lit(-WEB_MERCATOR_MAX_LAT),
    )
    .otherwise(lat)
    .expect("failed to build WebMercator latitude clamp expression")
}

#[cfg(test)]
mod tests {
    use super::*;
    use avenger_chart_core::ChannelValue;
    use datafusion::prelude::SessionContext;

    #[test]
    fn longitude_and_latitude_write_projected_position_channels() {
        let ctx = SessionContext::new();
        let symbol = Symbol::<WebMercator>::new()
            .longitude("lon")
            .latitude("lat");
        let channels = symbol.state().data.channels();
        let x = channels
            .get("x")
            .and_then(|value| value.expr(&ctx))
            .expect("x expr");
        let y = channels
            .get("y")
            .and_then(|value| value.expr(&ctx))
            .expect("y expr");
        assert!(x.to_string().contains("radians"));
        assert!(y.to_string().contains("ln"));
        assert!(channels.get("longitude").is_none());
        assert!(channels.get("latitude").is_none());
    }

    #[test]
    fn projected_methods_preserve_direct_channel_values() {
        let symbol = Symbol::<WebMercator>::new()
            .projected_x(10.0)
            .projected_y(20.0);
        assert!(matches!(
            symbol.state().data.channels().get("x"),
            Some(ChannelValue::Value { .. })
        ));
        assert!(matches!(
            symbol.state().data.channels().get("y"),
            Some(ChannelValue::Value { .. })
        ));
    }
}
