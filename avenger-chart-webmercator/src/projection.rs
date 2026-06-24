use std::f64::consts::{FRAC_PI_4, PI};

pub const EARTH_RADIUS_M: f64 = 6_378_137.0;
pub const WEB_MERCATOR_LIMIT: f64 = PI * EARTH_RADIUS_M;
pub const WEB_MERCATOR_MAX_LAT: f64 = 85.051_128_779_806_6;
pub const DEFAULT_TILE_SIZE: f64 = 256.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WebMercatorPoint {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WebMercatorLonLat {
    pub lon: f64,
    pub lat: f64,
}

pub fn clamp_latitude(lat: f64) -> f64 {
    lat.clamp(-WEB_MERCATOR_MAX_LAT, WEB_MERCATOR_MAX_LAT)
}

pub fn project_lon_lat(lon: f64, lat: f64) -> WebMercatorPoint {
    let lon_rad = lon.to_radians();
    let lat_rad = clamp_latitude(lat).to_radians();
    WebMercatorPoint {
        x: EARTH_RADIUS_M * lon_rad,
        y: EARTH_RADIUS_M * (FRAC_PI_4 + lat_rad / 2.0).tan().ln(),
    }
}

pub fn unproject_xy(x: f64, y: f64) -> WebMercatorLonLat {
    let lon = (x / EARTH_RADIUS_M).to_degrees();
    let lat = (2.0 * (y / EARTH_RADIUS_M).exp().atan() - PI / 2.0).to_degrees();
    WebMercatorLonLat { lon, lat }
}

pub fn units_per_pixel_for_zoom(zoom: f64) -> f64 {
    units_per_pixel_for_zoom_with_tile_size(zoom, DEFAULT_TILE_SIZE)
}

pub fn units_per_pixel_for_zoom_with_tile_size(zoom: f64, tile_size: f64) -> f64 {
    2.0 * WEB_MERCATOR_LIMIT / (tile_size * 2.0_f64.powf(zoom))
}

pub fn zoom_for_units_per_pixel(units_per_pixel: f64) -> f64 {
    zoom_for_units_per_pixel_with_tile_size(units_per_pixel, DEFAULT_TILE_SIZE)
}

pub fn zoom_for_units_per_pixel_with_tile_size(units_per_pixel: f64, tile_size: f64) -> f64 {
    (2.0 * WEB_MERCATOR_LIMIT / (tile_size * units_per_pixel)).log2()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_close(actual: f64, expected: f64, epsilon: f64) {
        assert!(
            (actual - expected).abs() <= epsilon,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn projects_known_epsg3857_values() {
        let origin = project_lon_lat(0.0, 0.0);
        assert_close(origin.x, 0.0, 1e-9);
        assert_close(origin.y, 0.0, 1e-8);

        let east = project_lon_lat(180.0, 0.0);
        assert_close(east.x, WEB_MERCATOR_LIMIT, 1e-6);
        assert_close(east.y, 0.0, 1e-8);

        let north = project_lon_lat(0.0, WEB_MERCATOR_MAX_LAT);
        assert_close(north.x, 0.0, 1e-9);
        assert_close(north.y, WEB_MERCATOR_LIMIT, 1e-6);
    }

    #[test]
    fn inverse_projection_round_trips() {
        let projected = project_lon_lat(-73.9857, 40.7484);
        let lon_lat = unproject_xy(projected.x, projected.y);
        assert_close(lon_lat.lon, -73.9857, 1e-10);
        assert_close(lon_lat.lat, 40.7484, 1e-10);
    }

    #[test]
    fn zoom_resolution_conversions_round_trip() {
        for zoom in [0.0, 1.0, 4.5, 12.0] {
            let units_per_pixel = units_per_pixel_for_zoom(zoom);
            assert_close(zoom_for_units_per_pixel(units_per_pixel), zoom, 1e-12);
        }
    }

    #[test]
    fn clamps_latitude_to_web_mercator_limit() {
        assert_eq!(clamp_latitude(90.0), WEB_MERCATOR_MAX_LAT);
        assert_eq!(clamp_latitude(-90.0), -WEB_MERCATOR_MAX_LAT);
    }
}
