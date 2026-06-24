use avenger_chart_core::AvengerChartError;
use avenger_resource::{
    ResourceCachePolicy, ResourceKey, ResourceKind, ResourceRequest, ResourceSource,
};
use serde::{Deserialize, Serialize};

use crate::projection::{
    DEFAULT_TILE_SIZE, WEB_MERCATOR_LIMIT, zoom_for_units_per_pixel_with_tile_size,
};
use crate::viewport::WebMercatorView;

const DEFAULT_LAYER_ID: &str = "tiles";
const DEFAULT_MAX_ZOOM: u8 = 19;
const MAX_SUPPORTED_ZOOM: u8 = 30;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct RasterTileLayer {
    #[serde(default = "default_layer_id")]
    id: String,
    url_template: String,
    #[serde(default = "default_tile_size")]
    tile_size: u32,
    #[serde(default)]
    min_zoom: u8,
    #[serde(default = "default_max_zoom")]
    max_zoom: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    attribution: Option<String>,
    #[serde(default)]
    cache_policy: ResourceCachePolicy,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    subdomains: Vec<String>,
    #[serde(default = "default_tile_zindex")]
    zindex: i32,
}

impl RasterTileLayer {
    pub fn xyz(url_template: impl Into<String>) -> Self {
        Self {
            id: default_layer_id(),
            url_template: url_template.into(),
            tile_size: default_tile_size(),
            min_zoom: 0,
            max_zoom: default_max_zoom(),
            attribution: None,
            cache_policy: ResourceCachePolicy::default(),
            subdomains: Vec::new(),
            zindex: default_tile_zindex(),
        }
    }

    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = id.into();
        self
    }

    pub fn tile_size(mut self, tile_size: u32) -> Self {
        self.tile_size = tile_size;
        self
    }

    pub fn min_zoom(mut self, min_zoom: u8) -> Self {
        self.min_zoom = min_zoom;
        self
    }

    pub fn max_zoom(mut self, max_zoom: u8) -> Self {
        self.max_zoom = max_zoom;
        self
    }

    pub fn attribution(mut self, attribution: impl Into<String>) -> Self {
        self.attribution = Some(attribution.into());
        self
    }

    pub fn cache_policy(mut self, cache_policy: ResourceCachePolicy) -> Self {
        self.cache_policy = cache_policy;
        self
    }

    pub fn subdomains<I, S>(mut self, subdomains: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.subdomains = subdomains.into_iter().map(Into::into).collect();
        self
    }

    pub fn zindex(mut self, zindex: i32) -> Self {
        self.zindex = zindex;
        self
    }

    pub fn layer_id(&self) -> &str {
        &self.id
    }

    pub fn url_template(&self) -> &str {
        &self.url_template
    }

    pub fn tile_size_value(&self) -> u32 {
        self.tile_size
    }

    pub fn min_zoom_value(&self) -> u8 {
        self.min_zoom
    }

    pub fn max_zoom_value(&self) -> u8 {
        self.max_zoom
    }

    pub fn attribution_text(&self) -> Option<&str> {
        self.attribution.as_deref()
    }

    pub fn zindex_value(&self) -> i32 {
        self.zindex
    }

    pub fn validate(&self) -> Result<(), AvengerChartError> {
        if self.id.is_empty() {
            return Err(AvengerChartError::InvalidArgument(
                "RasterTileLayer id must not be empty".to_string(),
            ));
        }
        if self.url_template.is_empty() {
            return Err(AvengerChartError::InvalidArgument(format!(
                "RasterTileLayer '{}' URL template must not be empty",
                self.id
            )));
        }
        if self.tile_size == 0 {
            return Err(AvengerChartError::InvalidArgument(format!(
                "RasterTileLayer '{}' tile_size must be positive",
                self.id
            )));
        }
        if self.min_zoom > self.max_zoom {
            return Err(AvengerChartError::InvalidArgument(format!(
                "RasterTileLayer '{}' min_zoom ({}) must be <= max_zoom ({})",
                self.id, self.min_zoom, self.max_zoom
            )));
        }
        if self.max_zoom > MAX_SUPPORTED_ZOOM {
            return Err(AvengerChartError::InvalidArgument(format!(
                "RasterTileLayer '{}' max_zoom ({}) must be <= {MAX_SUPPORTED_ZOOM}",
                self.id, self.max_zoom
            )));
        }
        if self.subdomains.iter().any(|subdomain| subdomain.is_empty()) {
            return Err(AvengerChartError::InvalidArgument(format!(
                "RasterTileLayer '{}' subdomains must not contain empty values",
                self.id
            )));
        }
        Ok(())
    }

    pub fn tile_zoom_for_view(&self, view: &WebMercatorView) -> u8 {
        let zoom = zoom_for_units_per_pixel_with_tile_size(
            view.units_per_pixel,
            f64::from(self.tile_size),
        )
        .round();
        let zoom = if zoom.is_finite() { zoom } else { 0.0 };
        zoom.clamp(f64::from(self.min_zoom), f64::from(self.max_zoom)) as u8
    }

    pub fn visible_tiles(
        &self,
        view: &WebMercatorView,
    ) -> Result<Vec<VisibleRasterTile>, AvengerChartError> {
        self.validate()?;

        let z = self.tile_zoom_for_view(view);
        let tile_count = tile_count(z);
        let tile_span = world_span() / tile_count as f64;
        let x_start = tile_floor((view.x_domain.0 + WEB_MERCATOR_LIMIT) / tile_span);
        let x_end = tile_ceil_exclusive((view.x_domain.1 + WEB_MERCATOR_LIMIT) / tile_span);
        let y_start = tile_floor((WEB_MERCATOR_LIMIT - view.y_domain.1) / tile_span).max(0);
        let y_end = tile_ceil_exclusive((WEB_MERCATOR_LIMIT - view.y_domain.0) / tile_span)
            .min(tile_count - 1);

        if y_start > y_end || x_start > x_end {
            return Ok(Vec::new());
        }

        let mut tiles = Vec::new();
        for y in y_start..=y_end {
            for unwrapped_x in x_start..=x_end {
                let wrapped_x = unwrapped_x.rem_euclid(tile_count);
                tiles.push(self.visible_tile(view, z, unwrapped_x, wrapped_x, y, tile_span));
            }
        }
        Ok(tiles)
    }

    pub fn resource_request(&self, tile: &VisibleRasterTile) -> ResourceRequest {
        ResourceRequest {
            key: tile.resource_key.clone(),
            kind: ResourceKind::new(avenger_image::IMAGE_RESOURCE_KIND),
            source: resource_source(tile.url.clone()),
            priority: 0.0,
            cache_policy: self.cache_policy.clone(),
        }
    }

    fn visible_tile(
        &self,
        view: &WebMercatorView,
        z: u8,
        unwrapped_x: i64,
        wrapped_x: i64,
        y: i64,
        tile_span: f64,
    ) -> VisibleRasterTile {
        let left = -WEB_MERCATOR_LIMIT + unwrapped_x as f64 * tile_span;
        let right = left + tile_span;
        let top = WEB_MERCATOR_LIMIT - y as f64 * tile_span;
        let bottom = top - tile_span;
        let pixel_left = projected_x_to_pixel(view, left);
        let pixel_right = projected_x_to_pixel(view, right);
        let pixel_top = projected_y_to_pixel(view, top);
        let pixel_bottom = projected_y_to_pixel(view, bottom);
        let url = self.tile_url(z, wrapped_x, y);
        let resource_key = self.resource_key(z, wrapped_x, y);

        VisibleRasterTile {
            layer_id: self.id.clone(),
            z,
            x: wrapped_x,
            y,
            unwrapped_x,
            resource_key,
            url,
            pixel_x: pixel_left as f32,
            pixel_y: pixel_top as f32,
            pixel_width: (pixel_right - pixel_left) as f32,
            pixel_height: (pixel_bottom - pixel_top) as f32,
            intrinsic_size: self.tile_size,
        }
    }

    fn tile_url(&self, z: u8, x: i64, y: i64) -> String {
        let subdomain = self.subdomain(z, x, y);
        self.url_template
            .replace("{z}", &z.to_string())
            .replace("{x}", &x.to_string())
            .replace("{y}", &y.to_string())
            .replace("{s}", subdomain)
    }

    fn subdomain(&self, z: u8, x: i64, y: i64) -> &str {
        if self.subdomains.is_empty() {
            return "";
        }
        let index = (i64::from(z) + x + y).rem_euclid(self.subdomains.len() as i64) as usize;
        &self.subdomains[index]
    }

    fn resource_key(&self, z: u8, x: i64, y: i64) -> ResourceKey {
        ResourceKey::new(format!(
            "webmercator/{}/{z}/{x}/{y}/{}",
            self.id, self.tile_size
        ))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct VisibleRasterTile {
    pub layer_id: String,
    pub z: u8,
    pub x: i64,
    pub y: i64,
    pub unwrapped_x: i64,
    pub resource_key: ResourceKey,
    pub url: String,
    pub pixel_x: f32,
    pub pixel_y: f32,
    pub pixel_width: f32,
    pub pixel_height: f32,
    pub intrinsic_size: u32,
}

fn default_layer_id() -> String {
    DEFAULT_LAYER_ID.to_string()
}

fn default_tile_size() -> u32 {
    DEFAULT_TILE_SIZE as u32
}

fn default_max_zoom() -> u8 {
    DEFAULT_MAX_ZOOM
}

fn default_tile_zindex() -> i32 {
    -100
}

fn world_span() -> f64 {
    2.0 * WEB_MERCATOR_LIMIT
}

fn tile_count(z: u8) -> i64 {
    1_i64 << z
}

fn tile_floor(value: f64) -> i64 {
    value.floor() as i64
}

fn tile_ceil_exclusive(value: f64) -> i64 {
    (value - 1e-12).ceil() as i64 - 1
}

fn projected_x_to_pixel(view: &WebMercatorView, x: f64) -> f64 {
    (x - view.x_domain.0) / (view.x_domain.1 - view.x_domain.0) * f64::from(view.plot_width)
}

fn projected_y_to_pixel(view: &WebMercatorView, y: f64) -> f64 {
    (view.y_domain.1 - y) / (view.y_domain.1 - view.y_domain.0) * f64::from(view.plot_height)
}

fn resource_source(url: String) -> ResourceSource {
    if url.starts_with("data:image/") {
        ResourceSource::DataUri { data_uri: url }
    } else {
        ResourceSource::Url { url }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::viewport::WebMercatorView;

    fn assert_close(actual: f32, expected: f32, epsilon: f32) {
        assert!(
            (actual - expected).abs() <= epsilon,
            "expected {expected}, got {actual}"
        );
    }

    fn layer() -> RasterTileLayer {
        RasterTileLayer::xyz("https://tiles.example/{z}/{x}/{y}.png").id("base")
    }

    #[test]
    fn expands_xyz_url_template() {
        let tile = layer().visible_tile(
            &WebMercatorView::from_center_zoom(0.0, 0.0, 0.0, 256.0, 256.0),
            3,
            9,
            1,
            2,
            world_span() / 8.0,
        );
        assert_eq!(tile.url, "https://tiles.example/3/1/2.png");
        assert_eq!(
            tile.resource_key,
            ResourceKey::new("webmercator/base/3/1/2/256")
        );
    }

    #[test]
    fn chooses_zoom_from_view_resolution_and_clamps() {
        let view = WebMercatorView::from_center_zoom(0.0, 0.0, 8.6, 256.0, 256.0);
        assert_eq!(layer().max_zoom(8).tile_zoom_for_view(&view), 8);
        assert_eq!(layer().min_zoom(10).tile_zoom_for_view(&view), 10);
    }

    #[test]
    fn enumerates_whole_world_at_zoom_zero() {
        let view = WebMercatorView::from_center_zoom(0.0, 0.0, 0.0, 256.0, 256.0);
        let tiles = layer().max_zoom(0).visible_tiles(&view).expect("tiles");
        assert_eq!(tiles.len(), 1);
        assert_eq!((tiles[0].z, tiles[0].x, tiles[0].y), (0, 0, 0));
        assert_eq!(tiles[0].pixel_x, 0.0);
        assert_eq!(tiles[0].pixel_y, 0.0);
        assert_eq!(tiles[0].pixel_width, 256.0);
        assert_eq!(tiles[0].pixel_height, 256.0);
    }

    #[test]
    fn wraps_x_tiles_and_clamps_y_tiles() {
        let view =
            WebMercatorView::from_center_zoom(WEB_MERCATOR_LIMIT * 1.5, 0.0, 1.0, 256.0, 256.0);
        let tiles = layer().max_zoom(1).visible_tiles(&view).expect("tiles");
        assert!(
            tiles
                .iter()
                .any(|tile| tile.unwrapped_x == 2 && tile.x == 0)
        );
        assert!(tiles.iter().all(|tile| (0..=1).contains(&tile.y)));
    }

    #[test]
    fn fractional_viewport_boundaries_enumerate_intersecting_tiles() {
        let view = WebMercatorView::from_center_zoom(0.0, 0.0, 2.0, 300.0, 260.0);
        let tiles = layer()
            .min_zoom(2)
            .max_zoom(2)
            .visible_tiles(&view)
            .expect("tiles");

        assert_eq!(tiles.len(), 4);
        assert!(tiles.iter().all(|tile| tile.z == 2));
        assert!(tiles.iter().any(|tile| tile.x == 1 && tile.y == 1));
        assert!(tiles.iter().any(|tile| tile.x == 2 && tile.y == 1));
        assert!(tiles.iter().any(|tile| tile.x == 1 && tile.y == 2));
        assert!(tiles.iter().any(|tile| tile.x == 2 && tile.y == 2));
        assert!(tiles.iter().all(|tile| tile.pixel_width > 0.0));
        assert!(tiles.iter().all(|tile| tile.pixel_height > 0.0));
    }

    #[test]
    fn overzoom_clamps_tile_zoom_and_scales_tile_pixels() {
        let view = WebMercatorView::from_center_zoom(0.0, 0.0, 5.0, 512.0, 512.0);
        let tiles = layer().max_zoom(3).visible_tiles(&view).expect("tiles");

        assert!(!tiles.is_empty());
        assert!(tiles.iter().all(|tile| tile.z == 3));
        assert!(tiles.iter().any(|tile| tile.x == 3 && tile.y == 3));
        assert!(tiles.iter().any(|tile| tile.x == 4 && tile.y == 4));
        for tile in tiles {
            assert_close(tile.pixel_width, 1024.0, 1e-3);
            assert_close(tile.pixel_height, 1024.0, 1e-3);
        }
    }

    #[test]
    fn data_uri_templates_create_data_uri_requests() {
        let layer = RasterTileLayer::xyz("data:image/png;base64,abc").id("inline");
        let view = WebMercatorView::from_center_zoom(0.0, 0.0, 0.0, 256.0, 256.0);
        let tile = layer.visible_tiles(&view).expect("tiles").remove(0);
        let request = layer.resource_request(&tile);
        assert!(matches!(request.source, ResourceSource::DataUri { .. }));
    }
}
