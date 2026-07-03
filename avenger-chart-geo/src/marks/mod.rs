//! Geo marks: `Symbol<Geo>`, `Line<Geo>` (phase 3), `GeoShape<Geo>`
//! (phase 4).
//!
//! Position authoring (scratch/geo decision 1): `.lon_lat(&geo, lon, lat)`
//! binds the `x`/`y` position channels to fields of the projection's
//! `geo_project` UDF (raw planar units; the coordinate-owned linear scales
//! map them to pixels), and additionally records the raw spherical
//! coordinates on unscaled `lon`/`lat` channels for geometry that needs
//! great-circle resampling (`Line<Geo>` in `GeometrySpace::Coordinate`,
//! the phase-5b blend).

pub mod geo_shape;
pub mod line;
pub mod rect;
pub mod symbol;

pub use geo_shape::{CompiledGeoShape, GeoShape};
pub use line::CompiledGeoLine;
pub use rect::CompiledGeoRect;
pub use symbol::CompiledGeoSymbol;

use std::sync::Arc;

use avenger_chart_core::{ChannelValue, GenericPositionConfig, PositionConfig};
use avenger_chart_marks::{Line, Symbol};
use datafusion::logical_expr::{Expr, col, lit};

use crate::Geo;

/// Accepts column names or expressions for lon/lat inputs.
pub trait IntoGeoExpr {
    fn into_geo_expr(self) -> Expr;
}

impl IntoGeoExpr for Expr {
    fn into_geo_expr(self) -> Expr {
        self
    }
}

impl IntoGeoExpr for &str {
    fn into_geo_expr(self) -> Expr {
        col(self)
    }
}

impl IntoGeoExpr for String {
    fn into_geo_expr(self) -> Expr {
        col(self)
    }
}

impl IntoGeoExpr for &Expr {
    fn into_geo_expr(self) -> Expr {
        self.clone()
    }
}

impl IntoGeoExpr for avenger_chart_core::ChannelExpr {
    fn into_geo_expr(self) -> Expr {
        self.into_data_expr()
    }
}

impl IntoGeoExpr for &avenger_chart_core::ChannelExpr {
    fn into_geo_expr(self) -> Expr {
        self.data_expr().clone()
    }
}

impl IntoGeoExpr for f64 {
    fn into_geo_expr(self) -> Expr {
        lit(self)
    }
}

impl IntoGeoExpr for f32 {
    fn into_geo_expr(self) -> Expr {
        lit(self)
    }
}

impl IntoGeoExpr for i32 {
    fn into_geo_expr(self) -> Expr {
        lit(self)
    }
}

impl IntoGeoExpr for i64 {
    fn into_geo_expr(self) -> Expr {
        lit(self)
    }
}

/// Per-channel position configuration (domain scope, axis config), the
/// same shape as the other coordinate systems' position configs.
pub type GeoPositionConfig = GenericPositionConfig<()>;

/// Build the `x`/`y` position expressions for a projection (closed-form
/// DataFusion built-ins; see [`crate::expr`]).
pub fn geo_position_exprs(geo: &Geo, lon: Expr, lat: Expr) -> (Expr, Expr) {
    crate::expr::geo_position_exprs(&geo.projection(), lon, lat)
}

/// Position channel builders shared by geo marks.
pub trait GeoPositionChannels: Sized {
    /// Position by geographic coordinates under `geo`'s projection.
    fn lon_lat<Lon: IntoGeoExpr, Lat: IntoGeoExpr>(self, geo: &Geo, lon: Lon, lat: Lat) -> Self;

    /// Like [`Self::lon_lat`], with per-channel configuration of the
    /// derived `x`/`y` position channels (domain scope, axis config).
    fn lon_lat_with<Lon, Lat, Fx, Fy>(
        self,
        geo: &Geo,
        lon: Lon,
        lat: Lat,
        configure_x: Fx,
        configure_y: Fy,
    ) -> Self
    where
        Lon: IntoGeoExpr,
        Lat: IntoGeoExpr,
        Fx: FnOnce(GeoPositionConfig) -> GeoPositionConfig,
        Fy: FnOnce(GeoPositionConfig) -> GeoPositionConfig;

    /// Position by pre-projected raw planar units (identity workflows,
    /// EPSG-projected inputs).
    fn projected_x<V: Into<ChannelValue>>(self, value: V) -> Self;
    fn projected_x_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(GeoPositionConfig) -> GeoPositionConfig;
    fn projected_y<V: Into<ChannelValue>>(self, value: V) -> Self;
    fn projected_y_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(GeoPositionConfig) -> GeoPositionConfig;
}

macro_rules! impl_geo_position_channels {
    ($mark:ident) => {
        impl GeoPositionChannels for $mark<Geo> {
            fn lon_lat<Lon: IntoGeoExpr, Lat: IntoGeoExpr>(
                self,
                geo: &Geo,
                lon: Lon,
                lat: Lat,
            ) -> Self {
                let lon = lon.into_geo_expr();
                let lat = lat.into_geo_expr();
                let (x, y) = geo_position_exprs(geo, lon.clone(), lat.clone());
                self.with_channel_value("x", ChannelValue::from(x))
                    .with_channel_value("y", ChannelValue::from(y))
                    .with_channel_value("lon", ChannelValue::from(lon).no_scale())
                    .with_channel_value("lat", ChannelValue::from(lat).no_scale())
            }

            fn lon_lat_with<Lon, Lat, Fx, Fy>(
                self,
                geo: &Geo,
                lon: Lon,
                lat: Lat,
                configure_x: Fx,
                configure_y: Fy,
            ) -> Self
            where
                Lon: IntoGeoExpr,
                Lat: IntoGeoExpr,
                Fx: FnOnce(GeoPositionConfig) -> GeoPositionConfig,
                Fy: FnOnce(GeoPositionConfig) -> GeoPositionConfig,
            {
                let lon = lon.into_geo_expr();
                let lat = lat.into_geo_expr();
                let (x, y) = geo_position_exprs(geo, lon.clone(), lat.clone());
                let mark = self.configure_position_channel("x", ChannelValue::from(x), configure_x);
                let mark = mark.configure_position_channel("y", ChannelValue::from(y), configure_y);
                mark.with_channel_value("lon", ChannelValue::from(lon).no_scale())
                    .with_channel_value("lat", ChannelValue::from(lat).no_scale())
            }

            fn projected_x<V: Into<ChannelValue>>(self, value: V) -> Self {
                self.with_channel_value("x", value.into())
            }

            fn projected_x_with<V, F>(self, value: V, f: F) -> Self
            where
                V: Into<ChannelValue>,
                F: FnOnce(GeoPositionConfig) -> GeoPositionConfig,
            {
                self.configure_position_channel("x", value.into(), f)
            }

            fn projected_y<V: Into<ChannelValue>>(self, value: V) -> Self {
                self.with_channel_value("y", value.into())
            }

            fn projected_y_with<V, F>(self, value: V, f: F) -> Self
            where
                V: Into<ChannelValue>,
                F: FnOnce(GeoPositionConfig) -> GeoPositionConfig,
            {
                self.configure_position_channel("y", value.into(), f)
            }
        }

        impl ConfigurePositionChannel for $mark<Geo> {
            fn configure_position_channel(
                self,
                channel_name: &str,
                channel_value: ChannelValue,
                f: impl FnOnce(GeoPositionConfig) -> GeoPositionConfig,
            ) -> Self {
                let config = GeoPositionConfig::new(channel_value);
                let configured = f(config);
                let (channel_value, axis_config) = configured.take_axis_config();
                let mut mark = self.with_channel_value(channel_name, channel_value);
                if let Some(axis_config) = axis_config {
                    mark.state_mut()
                        .axis_configs
                        .insert(channel_name.to_string(), Arc::new(axis_config));
                }
                mark
            }
        }
    };
}

/// Applies a configured position channel (value + optional axis config)
/// to a mark; implemented per mark type by the impl macro since the
/// underlying methods are inherent.
trait ConfigurePositionChannel: Sized {
    fn configure_position_channel(
        self,
        channel_name: &str,
        channel_value: ChannelValue,
        f: impl FnOnce(GeoPositionConfig) -> GeoPositionConfig,
    ) -> Self;
}

impl_geo_position_channels!(Symbol);
impl_geo_position_channels!(Line);

/// Geometry-space selection for geo marks with continuous geometry
/// (mirrors `avenger-chart-polar`'s extension trait).
pub trait GeoGeometrySpace: Sized {
    /// `Coordinate` (default): great-circle segments through the
    /// projection pipeline. `Display`: straight pixel-space segments.
    fn geometry_space(self, geometry_space: avenger_chart_core::GeometrySpace) -> Self;
}

impl GeoGeometrySpace for Line<Geo> {
    fn geometry_space(mut self, geometry_space: avenger_chart_core::GeometrySpace) -> Self {
        self.state_mut().geometry_space = Some(geometry_space);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lon_lat_binds_position_and_spherical_channels() {
        let geo = Geo::equal_earth();
        let symbol = Symbol::new().lon_lat(&geo, "lon_col", "lat_col");
        let channels = symbol.state().data.channels();
        assert!(channels.contains_key("x"), "x bound");
        assert!(channels.contains_key("y"), "y bound");
        assert!(channels.contains_key("lon"), "raw lon kept");
        assert!(channels.contains_key("lat"), "raw lat kept");
    }
}
