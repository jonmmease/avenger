//! Warped raster tile layers for the `Geo` coordinate system.
//!
//! Web-Mercator-gridded XYZ tiles are selected by projecting tile
//! boundaries through the view projector (a quadtree descent from z=0)
//! and drawn as textured meshes warped through the (possibly blended)
//! projection. When the authored projection *is* unrotated Mercator and
//! the blend is inactive, the warp is an axis-aligned similarity and the
//! layer falls back to plain image marks (`tile_pixel_rect`), matching
//! the WebMercator coordinate system pixel-for-pixel.
//!
//! Layer configuration (`RasterTileLayer`, `TileLoadingPolicy`) is ported
//! from `avenger-chart-webmercator/src/tiles.rs`; discovery differs: the
//! webmercator layer intersects the view's mercator-unit domain with the
//! grid directly, while here visibility is decided in *pixel* space via
//! the forward projection, which needs no inverse projection and handles
//! antimeridian wrap and per-region LOD uniformly.

use std::collections::HashSet;
use std::f64::consts::PI;
use std::sync::Arc;

use avenger_chart_core::AvengerChartError;
use avenger_geo::projector::Projector;
use avenger_resource::{
    PrefetchScope, ResourceCachePolicy, ResourceKey, ResourceKind, ResourceRequest,
    ResourceRequestPurpose, ResourceSource,
};
use serde::{Deserialize, Serialize};

use crate::view::GeoCoordMeasurement;

const DEFAULT_LAYER_ID: &str = "tiles";
const DEFAULT_TILE_SIZE: u32 = 256;
const DEFAULT_MAX_ZOOM: u8 = 19;
const MAX_SUPPORTED_ZOOM: u8 = 30;
const DEFAULT_MAX_RENDERED_TILES: usize = 128;
const DEFAULT_SMOOTH_ZOOM_FALLBACK_BELOW: u8 = 1;
const DEFAULT_SMOOTH_ZOOM_FALLBACK_ABOVE: u8 = 1;
const DEFAULT_SMOOTH_ZOOM_PREFETCH_BELOW: u8 = 1;
const DEFAULT_SMOOTH_ZOOM_PREFETCH_ABOVE: u8 = 1;
const DEFAULT_SMOOTH_ZOOM_PAN_PREFETCH_MARGIN_TILES: u8 = 1;
/// Coarse-cover prefetch level (MapLibre Native's default delta).
const DEFAULT_SMOOTH_ZOOM_PREFETCH_COARSE_DELTA: Option<u8> = Some(4);
const DEFAULT_SMOOTH_ZOOM_MAX_PREFETCH_TILES: usize = 128;
/// Mesh subdivision stops at 2^5 cells per tile edge (plan §6.2 depth 5).
const MAX_MESH_CELLS: u32 = 32;

// ---------------------------------------------------------------------------
// Tile grid
// ---------------------------------------------------------------------------

/// A tile-address scheme mapping `(z, x, y)` to spherical regions.
pub trait TileGrid {
    /// `[[west, south], [east, north]]` in degrees.
    fn tile_bounds_lonlat(&self, z: u8, x: i64, y: i64) -> [[f64; 2]; 2];

    /// Tiles at zoom `z` intersecting the lon/lat region
    /// `[[west, south], [east, north]]`; `west > east` spans the
    /// antimeridian. Returns `(wrapped_x, y, unwrapped_x)`.
    fn tiles_for_lonlat_region(&self, region: [[f64; 2]; 2], z: u8) -> Vec<(i64, i64, i64)>;
}

/// The standard Web-Mercator XYZ grid (slippy-map addressing).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MercatorTileGrid;

impl MercatorTileGrid {
    fn tile_count(z: u8) -> i64 {
        1_i64 << z
    }

    /// Latitude of the northern edge of tile row `y` at zoom `z`.
    pub fn tile_lat(z: u8, y: i64) -> f64 {
        let n = Self::tile_count(z) as f64;
        let merc_y = PI * (1.0 - 2.0 * (y as f64) / n);
        merc_y.sinh().atan().to_degrees()
    }

    /// Raw mercator y (`ln tan(π/4 + φ/2)`) of the northern edge of row `y`.
    pub fn tile_mercator_y(z: u8, y: i64) -> f64 {
        let n = Self::tile_count(z) as f64;
        PI * (1.0 - 2.0 * (y as f64) / n)
    }
}

impl TileGrid for MercatorTileGrid {
    fn tile_bounds_lonlat(&self, z: u8, x: i64, y: i64) -> [[f64; 2]; 2] {
        let n = Self::tile_count(z) as f64;
        let west = -180.0 + 360.0 * (x as f64) / n;
        let east = -180.0 + 360.0 * ((x + 1) as f64) / n;
        let north = Self::tile_lat(z, y);
        let south = Self::tile_lat(z, y + 1);
        [[west, south], [east, north]]
    }

    fn tiles_for_lonlat_region(&self, region: [[f64; 2]; 2], z: u8) -> Vec<(i64, i64, i64)> {
        let [[west, south], [east, north]] = region;
        let n = Self::tile_count(z);
        let nf = n as f64;
        // Longitudes → continuous (unwrapped) tile columns.
        let x_of = |lon: f64| (lon + 180.0) / 360.0 * nf;
        let (x0, x1) = if west <= east {
            (x_of(west), x_of(east))
        } else {
            // Antimeridian span: continue east past +180.
            (x_of(west), x_of(east) + nf)
        };
        // Latitudes → tile rows (row 0 at the north pole).
        let y_of = |lat: f64| {
            let lat = lat.clamp(-85.06, 85.06).to_radians();
            let merc = (PI / 4.0 + lat / 2.0).tan().ln();
            (1.0 - merc / PI) / 2.0 * nf
        };
        let (y0, y1) = (y_of(north), y_of(south));
        let x_start = x0.floor() as i64;
        let x_end = ((x1 - 1e-12).ceil() as i64 - 1).max(x_start);
        let y_start = (y0.floor() as i64).max(0);
        let y_end = (((y1 - 1e-12).ceil() as i64 - 1).min(n - 1)).max(y_start);
        let mut tiles = Vec::new();
        for y in y_start..=y_end {
            for unwrapped_x in x_start..=x_end {
                tiles.push((unwrapped_x.rem_euclid(n), y, unwrapped_x));
            }
        }
        tiles
    }
}

// ---------------------------------------------------------------------------
// Layer configuration (ported from webmercator)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum TileLoadingPolicy {
    #[default]
    Immediate,
    SmoothZoom {
        fallback_below: u8,
        fallback_above: u8,
        prefetch_below: u8,
        prefetch_above: u8,
        #[serde(default = "default_smooth_zoom_pan_prefetch_margin_tiles")]
        pan_prefetch_margin_tiles: u8,
        /// Also keep a coarse full-viewport cover at `target − delta`
        /// rendered as the bottom-most cache-only fallback and fetched as
        /// prefetch: a handful of tiles that guarantee *something*
        /// renders during fast pans and first views (MapLibre Native's
        /// `prefetchZoomDelta`).
        #[serde(default = "default_smooth_zoom_prefetch_coarse_delta")]
        prefetch_coarse_delta: Option<u8>,
        max_rendered_fallback_tiles: usize,
        max_prefetch_tiles: usize,
    },
}

impl TileLoadingPolicy {
    pub fn smooth_zoom_default() -> Self {
        Self::SmoothZoom {
            fallback_below: DEFAULT_SMOOTH_ZOOM_FALLBACK_BELOW,
            fallback_above: DEFAULT_SMOOTH_ZOOM_FALLBACK_ABOVE,
            prefetch_below: DEFAULT_SMOOTH_ZOOM_PREFETCH_BELOW,
            prefetch_above: DEFAULT_SMOOTH_ZOOM_PREFETCH_ABOVE,
            pan_prefetch_margin_tiles: DEFAULT_SMOOTH_ZOOM_PAN_PREFETCH_MARGIN_TILES,
            prefetch_coarse_delta: DEFAULT_SMOOTH_ZOOM_PREFETCH_COARSE_DELTA,
            max_rendered_fallback_tiles: DEFAULT_MAX_RENDERED_TILES,
            max_prefetch_tiles: DEFAULT_SMOOTH_ZOOM_MAX_PREFETCH_TILES,
        }
    }
}

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
    #[serde(default)]
    attribution: Option<String>,
    #[serde(default)]
    cache_policy: ResourceCachePolicy,
    #[serde(default)]
    subdomains: Vec<String>,
    #[serde(default = "default_tile_zindex")]
    zindex: i32,
    #[serde(default)]
    loading_policy: TileLoadingPolicy,
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
            loading_policy: TileLoadingPolicy::Immediate,
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

    pub fn loading_policy(mut self, loading_policy: TileLoadingPolicy) -> Self {
        self.loading_policy = loading_policy;
        self
    }

    pub fn smooth_zoom(self) -> Self {
        self.loading_policy(TileLoadingPolicy::smooth_zoom_default())
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

    pub fn loading_policy_value(&self) -> &TileLoadingPolicy {
        &self.loading_policy
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
        ResourceKey::new(format!("geo/{}/{z}/{x}/{y}/{}", self.id, self.tile_size))
    }

    pub fn resource_request(&self, tile: &VisibleGeoTile) -> ResourceRequest {
        self.resource_request_with_purpose(tile, ResourceRequestPurpose::Required, 0.0)
    }

    pub fn resource_request_with_purpose(
        &self,
        tile: &VisibleGeoTile,
        purpose: ResourceRequestPurpose,
        priority: f32,
    ) -> ResourceRequest {
        ResourceRequest {
            key: tile.resource_key.clone(),
            kind: ResourceKind::new(avenger_image::IMAGE_RESOURCE_KIND),
            source: resource_source(tile.url.clone()),
            priority,
            cache_policy: self.cache_policy.clone(),
            purpose,
            screen_center: None,
            prefetch_scope: None,
        }
    }

    fn visible_tile(&self, z: u8, wrapped_x: i64, y: i64, unwrapped_x: i64) -> VisibleGeoTile {
        VisibleGeoTile {
            layer_id: self.id.clone(),
            z,
            x: wrapped_x,
            y,
            unwrapped_x,
            resource_key: self.resource_key(z, wrapped_x, y),
            url: self.tile_url(z, wrapped_x, y),
            intrinsic_size: self.tile_size,
        }
    }

    /// The tiles this layer renders for a view: a quadtree descent from
    /// z=0 that subdivides every pixel-visible tile until it reaches its
    /// *local* target zoom (per-region LOD — regions the projection
    /// compresses on screen stop at coarser zooms).
    pub fn visible_tiles(
        &self,
        scope: &TileViewScope,
    ) -> Result<Vec<VisibleGeoTile>, AvengerChartError> {
        self.visible_tiles_cached(scope, &mut TileNodeCache::default())
    }

    fn visible_tiles_cached(
        &self,
        scope: &TileViewScope,
        cache: &mut TileNodeCache,
    ) -> Result<Vec<VisibleGeoTile>, AvengerChartError> {
        let mut tiles = self.visible_tiles_uncapped(scope, cache)?;
        if tiles.len() > DEFAULT_MAX_RENDERED_TILES {
            tracing::warn!(
                layer = %self.id,
                total = tiles.len(),
                cap = DEFAULT_MAX_RENDERED_TILES,
                "tile plan exceeds per-frame cap; truncating"
            );
            tiles.truncate(DEFAULT_MAX_RENDERED_TILES);
        }
        Ok(tiles)
    }

    fn visible_tiles_uncapped(
        &self,
        scope: &TileViewScope,
        cache: &mut TileNodeCache,
    ) -> Result<Vec<VisibleGeoTile>, AvengerChartError> {
        self.validate()?;
        Ok(self.enumerate_tiles(scope, 0, None, cache))
    }

    fn enumerate_tiles(
        &self,
        scope: &TileViewScope,
        zoom_offset: i8,
        restrict_px: Option<&[f64; 4]>,
        cache: &mut TileNodeCache,
    ) -> Vec<VisibleGeoTile> {
        if let Some(view) = &scope.identity_view {
            return self.identity_range_tiles(view, zoom_offset, restrict_px);
        }
        let mut tiles = Vec::new();
        let n0 = MercatorTileGrid::tile_count(0);
        for x in 0..n0 {
            for y in 0..n0 {
                self.descend(scope, 0, x, y, zoom_offset, restrict_px, cache, &mut tiles);
            }
        }
        tiles
    }

    /// Identity fast path: the view's raw-unit domains ARE mercator
    /// units, so intersect them with the grid directly (the webmercator
    /// algorithm) — exact, and x wraps across antimeridian world copies
    /// via unwrapped columns.
    fn identity_range_tiles(
        &self,
        view: &crate::view::GeoView,
        zoom_offset: i8,
        restrict_px: Option<&[f64; 4]>,
    ) -> Vec<VisibleGeoTile> {
        let zoom = ((2.0 * PI / (view.units_per_pixel * f64::from(self.tile_size)))
            .log2()
            .round() as i16
            + i16::from(zoom_offset))
        .clamp(i16::from(self.min_zoom), i16::from(self.max_zoom)) as u8;
        // A pixel-rect restriction maps to a raw-unit domain sub-rect
        // (pixel y measured down from the y-domain top).
        let (x_lo, x_hi, y_lo, y_hi) = match restrict_px {
            Some([rx0, ry0, rx1, ry1]) => {
                let plot_w = f64::from(view.plot_width);
                let plot_h = f64::from(view.plot_height);
                let (x0d, x1d) = view.x_domain;
                let (y0d, y1d) = view.y_domain;
                let px_to_x = |px: f64| x0d + (x1d - x0d) * (px / plot_w);
                let py_to_y = |py: f64| y1d - (y1d - y0d) * (py / plot_h);
                (px_to_x(*rx0), px_to_x(*rx1), py_to_y(*ry1), py_to_y(*ry0))
            }
            None => (
                view.x_domain.0,
                view.x_domain.1,
                view.y_domain.0,
                view.y_domain.1,
            ),
        };
        let n = MercatorTileGrid::tile_count(zoom);
        let span = 2.0 * PI / n as f64;
        let tile_floor = |value: f64| value.floor() as i64;
        let tile_ceil_exclusive = |value: f64| (value - 1e-12).ceil() as i64 - 1;
        let x_start = tile_floor((x_lo + PI) / span);
        let x_end = tile_ceil_exclusive((x_hi + PI) / span).max(x_start);
        let y_start = tile_floor((PI - y_hi) / span).max(0);
        let y_end = tile_ceil_exclusive((PI - y_lo) / span)
            .min(n - 1)
            .max(y_start);
        let mut tiles = Vec::new();
        for y in y_start..=y_end {
            for unwrapped_x in x_start..=x_end {
                tiles.push(self.visible_tile(zoom, unwrapped_x.rem_euclid(n), y, unwrapped_x));
            }
        }
        tiles
    }

    #[allow(clippy::too_many_arguments)]
    fn descend(
        &self,
        scope: &TileViewScope,
        z: u8,
        x: i64,
        y: i64,
        zoom_offset: i8,
        restrict_px: Option<&[f64; 4]>,
        cache: &mut TileNodeCache,
        out: &mut Vec<VisibleGeoTile>,
    ) {
        if !scope.tile_visible(z, x, y, restrict_px, cache) {
            return;
        }
        let target = (scope.local_target_zoom(z, x, y, f64::from(self.tile_size), cache)
            + i16::from(zoom_offset))
        .clamp(i16::from(self.min_zoom), i16::from(self.max_zoom)) as u8;
        if z >= target {
            let n = MercatorTileGrid::tile_count(z);
            out.push(self.visible_tile(z, x.rem_euclid(n), y, x));
            return;
        }
        for dy in 0..2 {
            for dx in 0..2 {
                self.descend(
                    scope,
                    z + 1,
                    2 * x + dx,
                    2 * y + dy,
                    zoom_offset,
                    restrict_px,
                    cache,
                    out,
                );
            }
        }
    }

    fn visible_tiles_with_offset(
        &self,
        scope: &TileViewScope,
        zoom_offset: i8,
        cache: &mut TileNodeCache,
    ) -> Vec<VisibleGeoTile> {
        self.enumerate_tiles(scope, zoom_offset, None, cache)
    }

    /// Tiles at `zoom_offset` from the local target whose projected
    /// footprint intersects `rect_px` (plot-relative `[x0, y0, x1, y1]`) —
    /// the cursor-anchored zoom-prefetch cover.
    fn visible_tiles_in_rect(
        &self,
        scope: &TileViewScope,
        zoom_offset: i8,
        rect_px: [f64; 4],
        cache: &mut TileNodeCache,
    ) -> Vec<VisibleGeoTile> {
        self.enumerate_tiles(scope, zoom_offset, Some(&rect_px), cache)
    }

    /// The full per-frame plan: rendered tiles (fallback zooms first so
    /// target tiles draw on top) plus prefetch requests. Under
    /// `SmoothZoom`, fallback tiles are RENDER-ONLY (drawn opportunistically
    /// from cache, never fetched as Required — MapLibre-style retention);
    /// prefetch covers the pan-margin ring plus cursor-anchored zoom rects.
    /// `plot_origin` is the plot rect's origin in canvas coordinates;
    /// it converts plot-relative tile centers into the canvas-frame
    /// `screen_center` metadata carried on prefetch requests.
    pub fn tile_plan(
        &self,
        measurement: &GeoCoordMeasurement,
        plot_origin: [f32; 2],
    ) -> Result<GeoTilePlan, AvengerChartError> {
        let scope = TileViewScope::new(measurement);
        let mut cache = TileNodeCache::default();
        let focus = effective_focus(&scope, measurement.zoom_focus);
        let uncapped = self.visible_tiles_uncapped(&scope, &mut cache)?;
        // Focus-out distances drive the fetch priority and the cap
        // (farthest tiles dropped, not an enumeration-order-arbitrary
        // suffix); RENDER order stays enumeration order so pixels are
        // unchanged (draw order shifts alpha-blended seam antialiasing).
        let denom = scope.plot_width.max(scope.plot_height).max(1.0);
        let mut indexed = uncapped
            .into_iter()
            .enumerate()
            .map(|(index, tile)| {
                let center = scope.tile_screen_center(&tile);
                let center = center
                    .map(|c| [f64::from(c[0]), f64::from(c[1])])
                    .unwrap_or([scope.plot_width / 2.0, scope.plot_height / 2.0]);
                let distance = (center[0] - focus[0]).hypot(center[1] - focus[1]);
                (index, distance, tile)
            })
            .collect::<Vec<_>>();
        if indexed.len() > DEFAULT_MAX_RENDERED_TILES {
            tracing::warn!(
                layer = %self.id,
                total = indexed.len(),
                cap = DEFAULT_MAX_RENDERED_TILES,
                "tile plan exceeds per-frame cap; dropping farthest tiles"
            );
            indexed.sort_by(|a, b| a.1.total_cmp(&b.1));
            indexed.truncate(DEFAULT_MAX_RENDERED_TILES);
            indexed.sort_by_key(|(index, _, _)| *index);
        }
        let mut targets = Vec::with_capacity(indexed.len());
        let mut target_tiles = Vec::with_capacity(indexed.len());
        for (_, distance, tile) in indexed {
            target_tiles.push(PlannedGeoTile {
                tile: tile.clone(),
                is_target: true,
                unavailable_policy: PlannedTileUnavailablePolicy::RendererDefault,
                fetch_priority: -(distance / denom) as f32,
            });
            targets.push(tile);
        }

        let TileLoadingPolicy::SmoothZoom {
            fallback_below,
            fallback_above,
            prefetch_coarse_delta,
            max_rendered_fallback_tiles,
            ..
        } = &self.loading_policy
        else {
            return Ok(GeoTilePlan {
                rendered_tiles: target_tiles,
                prefetch_requests: Vec::new(),
            });
        };

        let mut rendered_tiles = Vec::new();
        let mut rendered_keys = HashSet::new();
        // Bottom-most: the coarse cover (a handful of tiles at
        // target − delta) so fast pans and first views always have
        // something renderable underneath.
        if let Some(delta) = prefetch_coarse_delta.filter(|delta| *delta > 0) {
            let offset = -i8::try_from(delta.min(8)).expect("delta clamped to <= 8");
            for tile in self.visible_tiles_with_offset(&scope, offset, &mut cache) {
                if rendered_tiles.len() >= *max_rendered_fallback_tiles {
                    break;
                }
                if target_tiles
                    .iter()
                    .any(|target| target.tile.resource_key == tile.resource_key)
                {
                    continue;
                }
                if rendered_keys.insert(tile.resource_key.clone()) {
                    rendered_tiles.push(PlannedGeoTile {
                        tile,
                        is_target: false,
                        unavailable_policy: PlannedTileUnavailablePolicy::Skip,
                        fetch_priority: 0.0,
                    });
                }
            }
        }
        let fallback_offsets = nearby_zoom_offsets(*fallback_below, *fallback_above);
        for offset in &fallback_offsets {
            for tile in self.visible_tiles_with_offset(&scope, *offset, &mut cache) {
                if rendered_tiles.len() >= *max_rendered_fallback_tiles {
                    break;
                }
                if target_tiles
                    .iter()
                    .any(|target| target.tile.resource_key == tile.resource_key)
                {
                    continue;
                }
                if rendered_keys.insert(tile.resource_key.clone()) {
                    rendered_tiles.push(PlannedGeoTile {
                        tile,
                        is_target: false,
                        unavailable_policy: PlannedTileUnavailablePolicy::Skip,
                        fetch_priority: 0.0,
                    });
                }
            }
        }
        rendered_tiles.extend(target_tiles.into_iter().map(|mut tile| {
            tile.unavailable_policy = PlannedTileUnavailablePolicy::Skip;
            tile
        }));

        let prefetch_requests = self.plan_prefetch_requests_cached(
            &scope,
            &measurement.viewport_id,
            &targets,
            measurement.zoom_focus,
            plot_origin,
            &mut cache,
        );

        Ok(GeoTilePlan {
            rendered_tiles,
            prefetch_requests,
        })
    }

    /// The prefetch working set for the current targets and cursor focus:
    /// a pan-margin ring around target tiles plus cursor-anchored zoom
    /// covers (the pixel rects the viewport would show after zooming ±d
    /// levels at the focus), deduped against target keys, capped, and
    /// prioritized by distance to the focus. Also the retarget entry
    /// point: hover re-plans call this with a new focus against an
    /// unchanged scope/targets snapshot.
    pub(crate) fn plan_prefetch_requests(
        &self,
        scope: &TileViewScope,
        viewport_id: &str,
        targets: &[VisibleGeoTile],
        zoom_focus: Option<[f32; 2]>,
        plot_origin: [f32; 2],
    ) -> Vec<ResourceRequest> {
        self.plan_prefetch_requests_cached(
            scope,
            viewport_id,
            targets,
            zoom_focus,
            plot_origin,
            &mut TileNodeCache::default(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn plan_prefetch_requests_cached(
        &self,
        scope: &TileViewScope,
        viewport_id: &str,
        targets: &[VisibleGeoTile],
        zoom_focus: Option<[f32; 2]>,
        plot_origin: [f32; 2],
        cache: &mut TileNodeCache,
    ) -> Vec<ResourceRequest> {
        let TileLoadingPolicy::SmoothZoom {
            prefetch_below,
            prefetch_above,
            pan_prefetch_margin_tiles,
            prefetch_coarse_delta,
            max_prefetch_tiles,
            ..
        } = &self.loading_policy
        else {
            return Vec::new();
        };

        let plot_width = scope.plot_width;
        let plot_height = scope.plot_height;
        let focus = effective_focus(scope, zoom_focus);
        let prefetch_scope = PrefetchScope::new(format!("geo/{viewport_id}/{}", self.id));

        let mut requests = Vec::new();
        let mut requested_keys = targets
            .iter()
            .map(|tile| tile.resource_key.clone())
            .collect::<HashSet<_>>();

        if *pan_prefetch_margin_tiles > 0 {
            // Neighbors of target tiles in tile space cover the next pan
            // step regardless of projection shape.
            let margin = i64::from(*pan_prefetch_margin_tiles);
            'margin: for tile in targets {
                let n = MercatorTileGrid::tile_count(tile.z);
                for dy in -margin..=margin {
                    for dx in -margin..=margin {
                        if dx == 0 && dy == 0 {
                            continue;
                        }
                        let y = tile.y + dy;
                        if y < 0 || y >= n {
                            continue;
                        }
                        let unwrapped_x = tile.unwrapped_x + dx;
                        let neighbor =
                            self.visible_tile(tile.z, unwrapped_x.rem_euclid(n), y, unwrapped_x);
                        if requests.len() >= *max_prefetch_tiles {
                            break 'margin;
                        }
                        if requested_keys.insert(neighbor.resource_key.clone()) {
                            requests.push(self.prefetch_request(
                                &neighbor,
                                scope,
                                focus,
                                plot_origin,
                                &prefetch_scope,
                            ));
                        }
                    }
                }
            }
        }

        for offset in nearby_zoom_offsets(*prefetch_below, *prefetch_above) {
            if requests.len() >= *max_prefetch_tiles {
                break;
            }
            let rect = anchored_zoom_rect(focus, plot_width, plot_height, offset);
            for tile in self.visible_tiles_in_rect(scope, offset, rect, cache) {
                if requests.len() >= *max_prefetch_tiles {
                    break;
                }
                if requested_keys.insert(tile.resource_key.clone()) {
                    requests.push(self.prefetch_request(
                        &tile,
                        scope,
                        focus,
                        plot_origin,
                        &prefetch_scope,
                    ));
                }
            }
        }

        // Coarse full-viewport cover (a handful of tiles at
        // target − delta): the pan-safety layer's pixels.
        if let Some(delta) = prefetch_coarse_delta.filter(|delta| *delta > 0) {
            let offset = -i8::try_from(delta.min(8)).expect("delta clamped to <= 8");
            for tile in self.visible_tiles_with_offset(scope, offset, cache) {
                if requests.len() >= *max_prefetch_tiles {
                    break;
                }
                if requested_keys.insert(tile.resource_key.clone()) {
                    requests.push(self.prefetch_request(
                        &tile,
                        scope,
                        focus,
                        plot_origin,
                        &prefetch_scope,
                    ));
                }
            }
        }

        requests
    }

    /// A `Prefetch` request carrying the tile's projected pixel center
    /// (converted to canvas coordinates via `plot_origin`) and retarget
    /// scope, prioritized by normalized distance to the focus (0 at the
    /// focus, more negative farther away; `Required` always outranks
    /// `Prefetch` regardless of priority).
    fn prefetch_request(
        &self,
        tile: &VisibleGeoTile,
        scope: &TileViewScope,
        focus: [f64; 2],
        plot_origin: [f32; 2],
        prefetch_scope: &PrefetchScope,
    ) -> ResourceRequest {
        let plot_center = scope.tile_screen_center(tile);
        let center = plot_center
            .map(|center| [f64::from(center[0]), f64::from(center[1])])
            .unwrap_or([scope.plot_width / 2.0, scope.plot_height / 2.0]);
        let denom = scope.plot_width.max(scope.plot_height).max(1.0);
        let distance = (center[0] - focus[0]).hypot(center[1] - focus[1]) / denom;
        let mut request = self.resource_request_with_purpose(
            tile,
            ResourceRequestPurpose::Prefetch,
            -(distance as f32),
        );
        // screen_center is canvas-frame (matching the scheduler's hover
        // focus hint); the distance above stays in the plot frame, which
        // shares scale with canvas so priorities are unaffected.
        request.screen_center = plot_center.map(|c| [c[0] + plot_origin[0], c[1] + plot_origin[1]]);
        request.prefetch_scope = Some(prefetch_scope.clone());
        request
    }
}

/// The prefetch/ordering focus in plot px: the last interaction cursor
/// when it lies inside the plot, else the plot center.
fn effective_focus(scope: &TileViewScope, zoom_focus: Option<[f32; 2]>) -> [f64; 2] {
    zoom_focus
        .map(|focus| [f64::from(focus[0]), f64::from(focus[1])])
        .filter(|focus| {
            focus[0].is_finite()
                && focus[1].is_finite()
                && (0.0..=scope.plot_width).contains(&focus[0])
                && (0.0..=scope.plot_height).contains(&focus[1])
        })
        .unwrap_or([scope.plot_width / 2.0, scope.plot_height / 2.0])
}

/// The plot-relative pixel rect that fills the screen after a
/// cursor-anchored zoom by `offset` levels: shrunk toward the focus for
/// zoom-in (`offset > 0`), expanded around it for zoom-out (`offset < 0`).
/// `[x0, y0, x1, y1]`; the focus keeps its relative position in the rect.
fn anchored_zoom_rect(focus: [f64; 2], plot_width: f64, plot_height: f64, offset: i8) -> [f64; 4] {
    let scale = 2.0_f64.powi(-i32::from(offset));
    let x0 = focus[0] * (1.0 - scale);
    let y0 = focus[1] * (1.0 - scale);
    [x0, y0, x0 + plot_width * scale, y0 + plot_height * scale]
}

/// Per-evaluation snapshot that recomputes one tile layer's prefetch
/// working set for a new cursor position WITHOUT re-evaluating the
/// chart. Sound because hover cannot change
/// the view — center/zoom are frozen between evaluations, so the targets
/// and projector snapshot stay valid; only the focus varies.
///
/// The canvas→plot conversion uses the guide's plot bounds, which are
/// canvas-absolute for root plots (the common tile-map case). For nested/
/// faceted geo viewports the origin is frame-local, which makes the
/// containment test conservative: the planner no-ops rather than
/// retargeting wrongly.
pub struct GeoZoomPrefetchPlanner {
    scope: PrefetchScope,
    layer: RasterTileLayer,
    view_scope: Arc<TileViewScope>,
    targets: Vec<VisibleGeoTile>,
    viewport_id: String,
    plot_origin: [f32; 2],
}

impl GeoZoomPrefetchPlanner {
    pub fn from_snapshot(
        layer: RasterTileLayer,
        measurement: &GeoCoordMeasurement,
        targets: Vec<VisibleGeoTile>,
        plot_origin: [f32; 2],
    ) -> Self {
        let scope = PrefetchScope::new(format!(
            "geo/{}/{}",
            measurement.viewport_id,
            layer.layer_id()
        ));
        Self {
            scope,
            view_scope: Arc::new(TileViewScope::new(measurement)),
            targets,
            viewport_id: measurement.viewport_id.clone(),
            layer,
            plot_origin,
        }
    }
}

impl avenger_resource::PrefetchRetargetPlanner for GeoZoomPrefetchPlanner {
    fn scope(&self) -> &PrefetchScope {
        &self.scope
    }

    fn plan(&self, cursor_canvas_px: [f32; 2]) -> Option<Vec<ResourceRequest>> {
        let focus_x = f64::from(cursor_canvas_px[0] - self.plot_origin[0]);
        let focus_y = f64::from(cursor_canvas_px[1] - self.plot_origin[1]);
        if !(0.0..=self.view_scope.plot_width).contains(&focus_x)
            || !(0.0..=self.view_scope.plot_height).contains(&focus_y)
        {
            return None;
        }
        Some(self.layer.plan_prefetch_requests(
            &self.view_scope,
            &self.viewport_id,
            &self.targets,
            Some([focus_x as f32, focus_y as f32]),
            self.plot_origin,
        ))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct VisibleGeoTile {
    pub layer_id: String,
    pub z: u8,
    pub x: i64,
    pub y: i64,
    pub unwrapped_x: i64,
    pub resource_key: ResourceKey,
    pub url: String,
    pub intrinsic_size: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GeoTilePlan {
    pub rendered_tiles: Vec<PlannedGeoTile>,
    pub prefetch_requests: Vec<ResourceRequest>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlannedGeoTile {
    pub tile: VisibleGeoTile,
    pub is_target: bool,
    pub unavailable_policy: PlannedTileUnavailablePolicy,
    /// Fetch-queue priority for Required requests: focus-out distance so
    /// the scheduler serves the middle of the view (or the cursor) first.
    /// Render order is NOT priority order.
    pub fetch_priority: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlannedTileUnavailablePolicy {
    RendererDefault,
    Skip,
}

// ---------------------------------------------------------------------------
// View scope: visibility + per-region LOD in pixel space
// ---------------------------------------------------------------------------

/// Per-plan memo of projected tile-node geometry: `tile_plan` walks the
/// quadtree up to five times (target + fallback offsets + anchored
/// prefetch rects) and every walk needs the same per-node projected bbox
/// and local target zoom. Keyed by (z, unwrapped x, y).
#[derive(Default)]
pub struct TileNodeCache {
    bbox: std::collections::HashMap<(u8, i64, i64), Option<NodeBBoxes>>,
    target_zoom: std::collections::HashMap<(u8, i64, i64), i16>,
}

/// Projected footprint of a tile node: one bbox normally, two when the
/// tile straddles the projection's antimeridian cut (its samples land on
/// both sides of the map; the union bbox would span the whole projected
/// world, pass visibility everywhere, and explode the deep-zoom descent
/// along the seam column).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NodeBBoxes {
    primary: [f64; 4],
    secondary: Option<[f64; 4]>,
}

/// Everything tile discovery needs about the current view: the pixel
/// projector, the plot rectangle, and a visibility margin. On the
/// identity fast path the raw-unit view is carried too, enabling exact
/// analytic enumeration with antimeridian world copies (unwrapped x).
pub struct TileViewScope {
    projector: Projector,
    plot_width: f64,
    plot_height: f64,
    identity_view: Option<crate::view::GeoView>,
    /// `log2(2π / units_per_pixel)`: nominal tile-grid zoom before the
    /// tile-size term. Caps runaway per-region targets (seam/pole
    /// derivative estimates are garbage) at nominal + 2.
    nominal_zoom_bits: f64,
}

impl TileViewScope {
    pub fn new(measurement: &GeoCoordMeasurement) -> Self {
        Self {
            projector: measurement.view_projector(),
            plot_width: f64::from(measurement.view.plot_width),
            plot_height: f64::from(measurement.view.plot_height),
            identity_view: is_identity_fast_path(measurement).then_some(measurement.view),
            nominal_zoom_bits: (2.0 * PI / measurement.view.units_per_pixel).log2(),
        }
    }

    /// Whether any part of the tile can appear in the plot rect (or, when
    /// `restrict_px` is set, in that plot-relative sub-rect), decided by
    /// the bounding box of projected boundary samples (3×3 grid, memoized
    /// per plan). The forward projection is total, so this needs no
    /// inverse and no antimeridian case analysis; tiles the projection
    /// clips away project to nothing and drop out.
    fn tile_visible(
        &self,
        z: u8,
        x: i64,
        y: i64,
        restrict_px: Option<&[f64; 4]>,
        cache: &mut TileNodeCache,
    ) -> bool {
        let Some(bboxes) = self.node_bbox(z, x, y, cache) else {
            return false;
        };
        let [rx0, ry0, rx1, ry1] =
            restrict_px
                .copied()
                .unwrap_or([0.0, 0.0, self.plot_width, self.plot_height]);
        let hits = |[min_x, min_y, max_x, max_y]: [f64; 4]| {
            // One-tile margin keeps boundary tiles whose curved interior
            // bulges into the plot even when all samples fall outside.
            let margin = 0.25 * (max_x - min_x).max(max_y - min_y).max(64.0);
            max_x >= rx0 - margin
                && min_x <= rx1 + margin
                && max_y >= ry0 - margin
                && min_y <= ry1 + margin
        };
        hits(bboxes.primary) || bboxes.secondary.is_some_and(hits)
    }

    /// Projected footprint of the tile's 3×3 boundary samples; `None`
    /// when nothing projects finitely. Antimeridian-straddling tiles are
    /// split into per-side cluster bboxes (see [`NodeBBoxes`]).
    pub(crate) fn node_bbox(
        &self,
        z: u8,
        x: i64,
        y: i64,
        cache: &mut TileNodeCache,
    ) -> Option<NodeBBoxes> {
        if let Some(bbox) = cache.bbox.get(&(z, x, y)) {
            return *bbox;
        }
        let n = MercatorTileGrid::tile_count(z);
        let wrapped_x = x.rem_euclid(n);
        let [[west, south], [east, north]] = MercatorTileGrid.tile_bounds_lonlat(z, wrapped_x, y);
        let mut samples: Vec<(f64, f64)> = Vec::with_capacity(9);
        for i in 0..=2 {
            for j in 0..=2 {
                let lon = west + (east - west) * f64::from(i) / 2.0;
                let lat = south + (north - south) * f64::from(j) / 2.0;
                if let Some((px, py)) = self.projector.project(lon, lat) {
                    if px.is_finite() && py.is_finite() {
                        samples.push((px, py));
                    }
                }
            }
        }
        let bboxes = self.cluster_bboxes(&samples);
        cache.bbox.insert((z, x, y), bboxes);
        bboxes
    }

    /// One bbox for compact sample sets; two per-side bboxes when the
    /// samples straddle the projection cut. The split only triggers for
    /// footprints far larger than the plot (so ordinary tiles — including
    /// legitimately huge coarse tiles whose samples spread smoothly — are
    /// untouched: their largest x-gap stays well under half the span).
    fn cluster_bboxes(&self, samples: &[(f64, f64)]) -> Option<NodeBBoxes> {
        fn bbox_of(samples: &[(f64, f64)]) -> Option<[f64; 4]> {
            let mut iter = samples.iter();
            let &(x0, y0) = iter.next()?;
            let mut bbox = [x0, y0, x0, y0];
            for &(x, y) in iter {
                bbox[0] = bbox[0].min(x);
                bbox[1] = bbox[1].min(y);
                bbox[2] = bbox[2].max(x);
                bbox[3] = bbox[3].max(y);
            }
            Some(bbox)
        }

        let primary = bbox_of(samples)?;
        let span_x = primary[2] - primary[0];
        let span_y = primary[3] - primary[1];
        let huge = span_x.max(span_y) > 2.0 * (self.plot_width + self.plot_height);
        if huge && samples.len() >= 4 {
            let mut xs: Vec<f64> = samples.iter().map(|&(x, _)| x).collect();
            xs.sort_by(f64::total_cmp);
            let mut gap = 0.0_f64;
            let mut split_at = f64::NAN;
            for pair in xs.windows(2) {
                if pair[1] - pair[0] > gap {
                    gap = pair[1] - pair[0];
                    split_at = (pair[0] + pair[1]) / 2.0;
                }
            }
            if gap > 0.5 * span_x {
                let left: Vec<_> = samples
                    .iter()
                    .copied()
                    .filter(|&(x, _)| x < split_at)
                    .collect();
                let right: Vec<_> = samples
                    .iter()
                    .copied()
                    .filter(|&(x, _)| x >= split_at)
                    .collect();
                if let (Some(left_bbox), Some(right_bbox)) = (bbox_of(&left), bbox_of(&right)) {
                    return Some(NodeBBoxes {
                        primary: left_bbox,
                        secondary: Some(right_bbox),
                    });
                }
            }
        }
        Some(NodeBBoxes {
            primary,
            secondary: None,
        })
    }

    /// Projected pixel center of a tile, when the projection defines it:
    /// exact rect midpoint on the identity fast path, projected lon/lat
    /// midpoint otherwise. Feeds distance-to-focus prefetch priorities.
    pub(crate) fn tile_screen_center(&self, tile: &VisibleGeoTile) -> Option<[f32; 2]> {
        if let Some(view) = &self.identity_view {
            let [x, y, width, height] = tile_pixel_rect(tile, view);
            return Some([x + width / 2.0, y + height / 2.0]);
        }
        let [[west, south], [east, north]] =
            MercatorTileGrid.tile_bounds_lonlat(tile.z, tile.x, tile.y);
        let lon = (west + east) / 2.0;
        let lat = ((south + north) / 2.0).clamp(-85.0, 85.0);
        let (px, py) = self.projector.project(lon, lat)?;
        (px.is_finite() && py.is_finite()).then_some([px as f32, py as f32])
    }

    /// Target zoom for the region around the tile center: the zoom at
    /// which one tile-grid pixel maps to roughly one screen pixel,
    /// measured by the local derivative of the pixel projector with
    /// respect to *mercator* units (the grid's native space).
    fn local_target_zoom(
        &self,
        z: u8,
        x: i64,
        y: i64,
        tile_size: f64,
        cache: &mut TileNodeCache,
    ) -> i16 {
        if let Some(target) = cache.target_zoom.get(&(z, x, y)) {
            return *target;
        }
        let raw = self.local_target_zoom_uncached(z, x, y, tile_size);
        // Local derivative estimates blow up where the forward-difference
        // step crosses the projection cut or the polar clamp; genuine
        // magnification over the anchored view stays within ~one level.
        let nominal = (self.nominal_zoom_bits - tile_size.log2()).round() as i16;
        let target = raw.min(nominal.saturating_add(2));
        cache.target_zoom.insert((z, x, y), target);
        target
    }

    fn local_target_zoom_uncached(&self, z: u8, x: i64, y: i64, tile_size: f64) -> i16 {
        let n = MercatorTileGrid::tile_count(z);
        let wrapped_x = x.rem_euclid(n);
        let [[west, south], [east, north]] = MercatorTileGrid.tile_bounds_lonlat(z, wrapped_x, y);
        let lon = (west + east) / 2.0;
        let lat = ((south + north) / 2.0).clamp(-85.0, 85.0);
        let Some(scale) = self.pixels_per_mercator_unit(lon, lat) else {
            return 0;
        };
        // Screen size of a zoom-z tile here is (2π / 2^z) · scale pixels;
        // solve for the z where that equals tile_size.
        let zoom = (2.0 * PI * scale / tile_size).log2();
        if zoom.is_finite() {
            zoom.round().clamp(0.0, f64::from(MAX_SUPPORTED_ZOOM)) as i16
        } else {
            0
        }
    }

    /// |d(pixel)/d(mercator units)| at a point, via forward differences
    /// along both grid axes (max of the two, keeping tiles sharp along
    /// their least-compressed axis).
    fn pixels_per_mercator_unit(&self, lon: f64, lat: f64) -> Option<f64> {
        let base = self.projector.project(lon, lat)?;
        let d_lon = 0.05_f64;
        let d_lat = 0.05_f64 * if lat > 0.0 { -1.0 } else { 1.0 };
        let east = self.projector.project(lon + d_lon, lat)?;
        let north = self.projector.project(lon, lat + d_lat)?;
        // Mercator-unit displacements for those steps.
        let merc_dx = d_lon.to_radians();
        let merc_dy = (mercator_y(lat + d_lat) - mercator_y(lat)).abs();
        let px_east = ((east.0 - base.0).hypot(east.1 - base.1)) / merc_dx;
        let px_north = ((north.0 - base.0).hypot(north.1 - base.1)) / merc_dy;
        let scale = px_east.max(px_north);
        (scale.is_finite() && scale > 0.0).then_some(scale)
    }
}

fn mercator_y(lat: f64) -> f64 {
    let lat = lat.clamp(-85.06, 85.06).to_radians();
    (PI / 4.0 + lat / 2.0).tan().ln()
}

// ---------------------------------------------------------------------------
// Mesh generation
// ---------------------------------------------------------------------------

/// A textured mesh for one tile, in plot-relative pixels with normalized
/// tile-image UVs.
#[derive(Clone, Debug, PartialEq)]
pub struct TileMesh {
    pub positions: Vec<[f32; 2]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

/// Build the warped mesh for a tile through `projector` (blend-aware when
/// the caller passes the measurement's view projector). Rows are uniform
/// in mercator y — the tile image's native vertical axis — so UV `v` is
/// uniform per row. Grid density adapts to the resample criterion:
/// doubled until the projected midpoint deviation drops below
/// `precision_px` (capped at 32×32 cells). Returns `None` when the mesh
/// has no finite on-screen triangle.
/// Memoized projection lattice for one tile's mesh construction. Every
/// point the adaptive refinement and the final vertex grid evaluate lies
/// on the dyadic 33×33 lattice (u, v ∈ {k/32}): deviation probes use
/// midpoints with denominators ≤ 32 (cells ≤ 16) and vertex grids use
/// strides 32/cells (cells ≤ 32). Sharing one lattice caps a tile's
/// projections at 33²: each refinement level and the final vertex grid
/// read the same memoized points (dyadic u/v values make the lookups
/// exact, so results match evaluating every level independently).
struct MeshLattice {
    points: Vec<Option<Option<(f64, f64)>>>,
}

const MESH_LATTICE_DIM: usize = (MAX_MESH_CELLS + 1) as usize;

impl MeshLattice {
    fn new() -> Self {
        Self {
            points: vec![None; MESH_LATTICE_DIM * MESH_LATTICE_DIM],
        }
    }

    /// Lattice point at (iu/32, iv/32).
    fn get(
        &mut self,
        iu: u32,
        iv: u32,
        point_at: &impl Fn(f64, f64) -> Option<(f64, f64)>,
    ) -> Option<(f64, f64)> {
        let index = iv as usize * MESH_LATTICE_DIM + iu as usize;
        if let Some(cached) = self.points[index] {
            return cached;
        }
        let value = point_at(
            f64::from(iu) / f64::from(MAX_MESH_CELLS),
            f64::from(iv) / f64::from(MAX_MESH_CELLS),
        );
        self.points[index] = Some(value);
        value
    }
}

pub fn tile_mesh(
    tile: &VisibleGeoTile,
    projector: &Projector,
    precision_px: f64,
    plot_width: f32,
    plot_height: f32,
) -> Option<TileMesh> {
    let [[west, _south], [east, _north]] =
        MercatorTileGrid.tile_bounds_lonlat(tile.z, tile.x, tile.y);
    let merc_top = MercatorTileGrid::tile_mercator_y(tile.z, tile.y);
    let merc_bottom = MercatorTileGrid::tile_mercator_y(tile.z, tile.y + 1);

    let point_at = |u: f64, v: f64| -> Option<(f64, f64)> {
        let lon = west + (east - west) * u;
        let merc = merc_top + (merc_bottom - merc_top) * v;
        let lat = merc.sinh().atan().to_degrees();
        projector
            .project(lon, lat)
            .filter(|(x, y)| x.is_finite() && y.is_finite())
    };

    let mut lattice = MeshLattice::new();
    let mut cells: u32 = 1;
    while cells < MAX_MESH_CELLS {
        if max_midpoint_deviation(&mut lattice, &point_at, cells) <= precision_px {
            break;
        }
        cells *= 2;
    }

    let lattice_stride = MAX_MESH_CELLS / cells;
    let stride = cells + 1;
    let mut positions = Vec::with_capacity((stride * stride) as usize);
    let mut uvs = Vec::with_capacity((stride * stride) as usize);
    let mut finite = vec![false; (stride * stride) as usize];
    for row in 0..stride {
        for col in 0..stride {
            let u = f64::from(col) / f64::from(cells);
            let v = f64::from(row) / f64::from(cells);
            match lattice.get(col * lattice_stride, row * lattice_stride, &point_at) {
                Some((x, y)) => {
                    finite[(row * stride + col) as usize] = true;
                    positions.push([x as f32, y as f32]);
                }
                None => positions.push([0.0, 0.0]),
            }
            uvs.push([u as f32, v as f32]);
        }
    }

    // A straddling cell (the projection's antimeridian cut passes through
    // the tile) projects to a triangle spanning the map; cull any triangle
    // with an implausibly long edge relative to its neighbors.
    let mut max_plausible_edge = 0.0_f64;
    {
        let half = MAX_MESH_CELLS / 2;
        let corners = [
            lattice.get(0, 0, &point_at),
            lattice.get(MAX_MESH_CELLS, 0, &point_at),
            lattice.get(0, MAX_MESH_CELLS, &point_at),
            lattice.get(MAX_MESH_CELLS, MAX_MESH_CELLS, &point_at),
            lattice.get(half, half, &point_at),
        ];
        let pts = corners.into_iter().flatten().collect::<Vec<_>>();
        for a in &pts {
            for b in &pts {
                max_plausible_edge = max_plausible_edge.max((a.0 - b.0).hypot(a.1 - b.1));
            }
        }
        // Per-cell budget with generous headroom for local distortion.
        max_plausible_edge = (max_plausible_edge / f64::from(cells)) * 8.0 + 16.0;
    }

    let mut indices = Vec::new();
    let mut min_x = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for row in 0..cells {
        for col in 0..cells {
            let i00 = row * stride + col;
            let i10 = i00 + 1;
            let i01 = i00 + stride;
            let i11 = i01 + 1;
            for triangle in [[i00, i10, i11], [i00, i11, i01]] {
                if !triangle.iter().all(|i| finite[*i as usize]) {
                    continue;
                }
                let mut plausible = true;
                for pair in [(0, 1), (1, 2), (2, 0)] {
                    let a = positions[triangle[pair.0] as usize];
                    let b = positions[triangle[pair.1] as usize];
                    let edge = f64::from(a[0] - b[0]).hypot(f64::from(a[1] - b[1]));
                    if edge > max_plausible_edge {
                        plausible = false;
                        break;
                    }
                }
                if !plausible {
                    continue;
                }
                for i in triangle {
                    let [x, y] = positions[i as usize];
                    min_x = min_x.min(x);
                    max_x = max_x.max(x);
                    min_y = min_y.min(y);
                    max_y = max_y.max(y);
                }
                indices.extend_from_slice(&triangle);
            }
        }
    }

    if indices.is_empty() || max_x < 0.0 || min_x > plot_width || max_y < 0.0 || min_y > plot_height
    {
        return None;
    }
    Some(TileMesh {
        positions,
        uvs,
        indices,
    })
}

/// Max deviation of segment midpoints from linear interpolation, over
/// the shared lattice. Only called for `cells <= 16`, so midpoint
/// half-strides stay on the 33×33 lattice.
fn max_midpoint_deviation(
    lattice: &mut MeshLattice,
    point_at: &impl Fn(f64, f64) -> Option<(f64, f64)>,
    cells: u32,
) -> f64 {
    let stride = MAX_MESH_CELLS / cells;
    let half = stride / 2;
    let mut max_deviation = 0.0_f64;
    for row in 0..=cells {
        for col in 0..cells {
            // Horizontal segment (and its transpose for the vertical).
            for (a, b, m) in [
                (
                    (col * stride, row * stride),
                    ((col + 1) * stride, row * stride),
                    (col * stride + half, row * stride),
                ),
                (
                    (row * stride, col * stride),
                    (row * stride, (col + 1) * stride),
                    (row * stride, col * stride + half),
                ),
            ] {
                let (Some(pa), Some(pb), Some(pm)) = (
                    lattice.get(a.0, a.1, point_at),
                    lattice.get(b.0, b.1, point_at),
                    lattice.get(m.0, m.1, point_at),
                ) else {
                    continue;
                };
                let mid = ((pa.0 + pb.0) / 2.0, (pa.1 + pb.1) / 2.0);
                max_deviation = max_deviation.max((pm.0 - mid.0).hypot(pm.1 - mid.1));
            }
        }
    }
    max_deviation
}

// ---------------------------------------------------------------------------
// Identity fast path
// ---------------------------------------------------------------------------

/// True when tiles need no warp: the authored projection is unrotated
/// Mercator and the adaptive blend is inactive. On this path tiles render
/// as plain axis-aligned image marks (WebMercator pixel parity).
pub fn is_identity_fast_path(measurement: &GeoCoordMeasurement) -> bool {
    use avenger_geo::raw::ProjectionKind;
    matches!(measurement.projection.kind, ProjectionKind::Mercator)
        && measurement.projection.rotate == [0.0, 0.0, 0.0]
        && measurement.blend_t() == 0.0
}

/// Axis-aligned pixel rectangle `[x, y, width, height]` of a tile under
/// the identity fast path, from the view's raw-unit domains (raw mercator
/// spans `[-π, π]²`).
pub fn tile_pixel_rect(tile: &VisibleGeoTile, view: &crate::view::GeoView) -> [f32; 4] {
    let n = MercatorTileGrid::tile_count(tile.z) as f64;
    let span = 2.0 * PI / n;
    let left = -PI + tile.unwrapped_x as f64 * span;
    let top = PI - tile.y as f64 * span;
    let (x0, x1) = view.x_domain;
    let (y0, y1) = view.y_domain;
    let px = (left - x0) / (x1 - x0) * f64::from(view.plot_width);
    let py = (y1 - top) / (y1 - y0) * f64::from(view.plot_height);
    let width = span / (x1 - x0) * f64::from(view.plot_width);
    let height = span / (y1 - y0) * f64::from(view.plot_height);
    [px as f32, py as f32, width as f32, height as f32]
}

fn nearby_zoom_offsets(below: u8, above: u8) -> Vec<i8> {
    let mut offsets = Vec::new();
    for offset in 1..=below.min(8) {
        offsets.push(-(offset as i8));
    }
    for offset in 1..=above.min(8) {
        offsets.push(offset as i8);
    }
    offsets.sort_unstable();
    offsets
}

fn default_layer_id() -> String {
    DEFAULT_LAYER_ID.to_string()
}

fn default_tile_size() -> u32 {
    DEFAULT_TILE_SIZE
}

fn default_max_zoom() -> u8 {
    DEFAULT_MAX_ZOOM
}

fn default_tile_zindex() -> i32 {
    // Above the sphere fill (-5), below the graticule (-2) and marks (0).
    -4
}

fn default_smooth_zoom_pan_prefetch_margin_tiles() -> u8 {
    DEFAULT_SMOOTH_ZOOM_PAN_PREFETCH_MARGIN_TILES
}

fn default_smooth_zoom_prefetch_coarse_delta() -> Option<u8> {
    DEFAULT_SMOOTH_ZOOM_PREFETCH_COARSE_DELTA
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
    use avenger_geo::projector::Projection;
    use avenger_geo::raw::ProjectionKind;

    use super::*;
    use crate::view::{GeoView, WorldSpan, world_span};

    fn layer() -> RasterTileLayer {
        RasterTileLayer::xyz("https://tiles.example/{z}/{x}/{y}.png").id("base")
    }

    fn measurement(
        projection: Projection,
        center_lonlat: (f64, f64),
        zoom: f64,
        plot: (f32, f32),
    ) -> GeoCoordMeasurement {
        let world = world_span(&projection);
        let (cx, cy) = projection.project_raw_units(center_lonlat.0, center_lonlat.1);
        GeoCoordMeasurement {
            viewport_id: "test".to_string(),
            view: GeoView::from_center_zoom(cx, cy, zoom, plot.0, plot.1, world),
            projection,
            graticule: None,
            sphere: None,
            blend: None,
            tile_layers: Vec::new(),
            zoom_focus: None,
        }
    }

    fn albers_conus() -> Projection {
        Projection::new(ProjectionKind::albers()).with_rotate([96.0, 0.0, 0.0])
    }

    #[test]
    fn grid_bounds_cover_the_world_at_z1() {
        let grid = MercatorTileGrid;
        let [[west, south], [east, north]] = grid.tile_bounds_lonlat(1, 0, 0);
        assert_eq!(west, -180.0);
        assert_eq!(east, 0.0);
        assert!((north - 85.051).abs() < 0.01);
        assert!(south.abs() < 1e-9);
        let [[west, south], [east, north]] = grid.tile_bounds_lonlat(1, 1, 1);
        assert_eq!(west, 0.0);
        assert_eq!(east, 180.0);
        assert!(north.abs() < 1e-9);
        assert!((south + 85.051).abs() < 0.01);
    }

    #[test]
    fn grid_region_enumeration_matches_conus_hand_check() {
        // CONUS bbox: lon -125..-66, lat 24..50 at z4 → x 2..5, y 5..6.
        let tiles = MercatorTileGrid.tiles_for_lonlat_region([[-125.0, 24.0], [-66.0, 50.0]], 4);
        let mut xs = tiles.iter().map(|t| t.0).collect::<Vec<_>>();
        let mut ys = tiles.iter().map(|t| t.1).collect::<Vec<_>>();
        xs.sort_unstable();
        xs.dedup();
        ys.sort_unstable();
        ys.dedup();
        assert_eq!(xs, vec![2, 3, 4, 5]);
        assert_eq!(ys, vec![5, 6]);
        assert_eq!(tiles.len(), 8);
    }

    #[test]
    fn grid_region_enumeration_wraps_the_antimeridian() {
        // Fiji-ish region spanning the antimeridian at z2.
        let tiles = MercatorTileGrid.tiles_for_lonlat_region([[170.0, -30.0], [-170.0, -10.0]], 2);
        assert!(!tiles.is_empty());
        assert!(tiles.iter().any(|t| t.0 == 3), "west side present");
        assert!(
            tiles.iter().any(|t| t.0 == 0),
            "east side present (wrapped)"
        );
        assert!(
            tiles
                .iter()
                .all(|t| (0..4).contains(&t.0) && (0..4).contains(&t.1))
        );
        // Unwrapped columns are contiguous across the seam.
        assert!(tiles.iter().any(|t| t.2 == 4 && t.0 == 0));
    }

    #[test]
    fn albers_conus_discovery_yields_the_fixture_tiles() {
        // The CONUS albers fit at ~z4 target resolution must select exactly
        // the eight checked-in fixture tiles (x 2..5, y 5..6) at z4.
        let measurement = measurement(albers_conus(), (-96.0, 38.0), 4.0, (620.0, 400.0));
        let scope = TileViewScope::new(&measurement);
        let tiles = layer()
            .min_zoom(4)
            .max_zoom(4)
            .visible_tiles(&scope)
            .expect("tiles");
        assert!(!tiles.is_empty());
        assert!(tiles.iter().all(|t| t.z == 4));
        for tile in &tiles {
            assert!(
                (1..=6).contains(&tile.x) && (4..=7).contains(&tile.y),
                "unexpected tile ({}, {})",
                tile.x,
                tile.y
            );
        }
        for (x, y) in [(3, 5), (3, 6), (4, 5), (4, 6)] {
            assert!(
                tiles.iter().any(|t| t.x == x && t.y == y),
                "missing core CONUS tile ({x}, {y})"
            );
        }
    }

    #[test]
    fn deeper_view_selects_deeper_tiles() {
        let wide = measurement(albers_conus(), (-96.0, 38.0), 4.0, (620.0, 400.0));
        let deep = measurement(albers_conus(), (-99.0, 31.5), 6.0, (620.0, 400.0));
        let wide_tiles = layer()
            .visible_tiles(&TileViewScope::new(&wide))
            .expect("wide");
        let deep_tiles = layer()
            .visible_tiles(&TileViewScope::new(&deep))
            .expect("deep");
        let wide_max = wide_tiles.iter().map(|t| t.z).max().unwrap();
        let deep_min = deep_tiles.iter().map(|t| t.z).min().unwrap();
        assert!(
            deep_min >= wide_max + 1,
            "expected deeper tiles: wide max z{wide_max}, deep min z{deep_min}"
        );
    }

    #[test]
    fn world_equal_earth_uses_coarser_tiles_toward_the_map_edge() {
        // On a world Equal Earth view the projection compresses high
        // latitudes and the far east/west; per-region LOD must pick
        // coarser tiles somewhere while the center stays at the target.
        let measurement = measurement(
            Projection::new(ProjectionKind::EqualEarth),
            (0.0, 0.0),
            2.5,
            (900.0, 560.0),
        );
        let scope = TileViewScope::new(&measurement);
        let tiles = layer().visible_tiles(&scope).expect("tiles");
        let zooms = tiles
            .iter()
            .map(|t| t.z)
            .collect::<std::collections::HashSet<_>>();
        assert!(
            zooms.len() >= 2,
            "expected mixed LOD on a world view, got zooms {zooms:?}"
        );
        // The coarser tiles sit at the top/bottom rows (compressed poles).
        let min_z = *zooms.iter().min().unwrap();
        let max_z = *zooms.iter().max().unwrap();
        let coarse_rows = tiles
            .iter()
            .filter(|t| t.z == min_z)
            .map(|t| {
                // Normalize row position to [0, 1] at that zoom.
                t.y as f64 / MercatorTileGrid::tile_count(min_z) as f64
            })
            .collect::<Vec<_>>();
        assert!(
            coarse_rows.iter().all(|row| *row < 0.35 || *row > 0.65),
            "coarse tiles should hug the poles: {coarse_rows:?} (z{min_z}..z{max_z})"
        );
    }

    #[test]
    fn mesh_density_grows_as_precision_tightens() {
        let measurement = measurement(
            Projection::new(ProjectionKind::EqualEarth),
            (0.0, 0.0),
            1.0,
            (600.0, 400.0),
        );
        let projector = measurement.view_projector();
        let tile = layer().visible_tile(1, 0, 0, 0);
        let coarse = tile_mesh(&tile, &projector, 64.0, 600.0, 400.0).expect("coarse mesh");
        let fine = tile_mesh(&tile, &projector, 0.25, 600.0, 400.0).expect("fine mesh");
        assert!(
            fine.positions.len() > coarse.positions.len(),
            "expected more vertices at tighter precision: {} vs {}",
            fine.positions.len(),
            coarse.positions.len()
        );
        assert!(fine.indices.len() % 3 == 0 && !fine.indices.is_empty());
        // UVs stay normalized.
        assert!(
            fine.uvs
                .iter()
                .all(|uv| (0.0..=1.0).contains(&uv[0]) && (0.0..=1.0).contains(&uv[1]))
        );
    }

    #[test]
    fn identity_fast_path_detection() {
        let mercator = measurement(
            Projection::new(ProjectionKind::Mercator),
            (0.0, 0.0),
            1.0,
            (512.0, 256.0),
        );
        assert!(is_identity_fast_path(&mercator));

        let rotated = measurement(
            Projection::new(ProjectionKind::Mercator).with_rotate([30.0, 0.0, 0.0]),
            (0.0, 0.0),
            1.0,
            (512.0, 256.0),
        );
        assert!(!is_identity_fast_path(&rotated));

        let albers = measurement(albers_conus(), (-96.0, 38.0), 4.0, (620.0, 400.0));
        assert!(!is_identity_fast_path(&albers));
    }

    #[test]
    fn identity_pixel_rects_tile_the_world_view() {
        let world = WorldSpan {
            width: 2.0 * PI,
            height: 2.0 * PI,
        };
        let view = GeoView::from_center_zoom(0.0, 0.0, 0.0, 256.0, 256.0, world);
        let tile = layer().visible_tile(0, 0, 0, 0);
        let [x, y, width, height] = tile_pixel_rect(&tile, &view);
        assert!((x - 0.0).abs() < 1e-3 && (y - 0.0).abs() < 1e-3);
        assert!((width - 256.0).abs() < 1e-3 && (height - 256.0).abs() < 1e-3);
    }

    #[test]
    fn tile_plan_smooth_zoom_adds_fallbacks_and_prefetch() {
        let measurement = measurement(albers_conus(), (-96.0, 38.0), 4.0, (620.0, 400.0));
        let plan = layer()
            .min_zoom(2)
            .max_zoom(6)
            .smooth_zoom()
            .tile_plan(&measurement, [0.0, 0.0])
            .expect("plan");
        let first_target = plan
            .rendered_tiles
            .iter()
            .position(|tile| tile.is_target)
            .expect("target tiles");
        assert!(
            plan.rendered_tiles[..first_target]
                .iter()
                .all(|tile| !tile.is_target),
            "fallbacks render before targets"
        );
        assert!(
            plan.rendered_tiles
                .iter()
                .all(|tile| tile.unavailable_policy == PlannedTileUnavailablePolicy::Skip)
        );
        assert!(!plan.prefetch_requests.is_empty());
        assert!(
            plan.prefetch_requests
                .iter()
                .all(|request| request.purpose == ResourceRequestPurpose::Prefetch)
        );
        // No prefetch duplicates a TARGET tile (fallback-zoom tiles are
        // render-only, so prefetch legitimately overlaps them — that is
        // how their pixels get into the cache).
        let target_keys = plan
            .rendered_tiles
            .iter()
            .filter(|tile| tile.is_target)
            .map(|tile| tile.tile.resource_key.clone())
            .collect::<HashSet<_>>();
        assert!(
            plan.prefetch_requests
                .iter()
                .all(|request| !target_keys.contains(&request.key))
        );
        // Prefetch requests carry retarget metadata and focus-distance
        // priorities (0 at the focus, negative farther out).
        assert!(plan.prefetch_requests.iter().all(|request| {
            request.prefetch_scope == Some(PrefetchScope::new("geo/test/base"))
                && request.priority <= 0.0
        }));
        assert!(
            plan.prefetch_requests
                .iter()
                .any(|request| request.screen_center.is_some())
        );
    }

    #[test]
    fn anchored_zoom_rect_tracks_the_focus() {
        // Centered focus, one level in: the centered half-rect.
        let rect = anchored_zoom_rect([300.0, 200.0], 600.0, 400.0, 1);
        assert_eq!(rect, [150.0, 100.0, 450.0, 300.0]);
        // Corner focus: the corner quadrant.
        let rect = anchored_zoom_rect([0.0, 0.0], 600.0, 400.0, 1);
        assert_eq!(rect, [0.0, 0.0, 300.0, 200.0]);
        // Two levels in converges toward the focus point.
        let rect = anchored_zoom_rect([600.0, 400.0], 600.0, 400.0, 2);
        assert_eq!(rect, [450.0, 300.0, 600.0, 400.0]);
        // Zoom out expands around the focus; the focus keeps its relative
        // position (here 1/3 across).
        let rect = anchored_zoom_rect([200.0, 200.0], 600.0, 400.0, -1);
        assert_eq!(rect, [-200.0, -200.0, 1000.0, 600.0]);
    }

    #[test]
    fn identity_prefetch_cover_restricts_to_the_cursor_quadrant() {
        // Whole-world identity Mercator view: plot 512×512 at zoom 0 shows
        // all four z1 tiles as targets (256 px each).
        let measurement = measurement(
            Projection::new(ProjectionKind::Mercator),
            (0.0, 0.0),
            1.0,
            (512.0, 512.0),
        );
        assert!(is_identity_fast_path(&measurement));
        let scope = TileViewScope::new(&measurement);
        let layer = layer().min_zoom(0).max_zoom(4);
        let targets = layer.visible_tiles(&scope).expect("targets");
        assert!(targets.iter().all(|tile| tile.z == 1));
        assert_eq!(targets.len(), 4);

        // Cursor at the top-left corner: the z2 cover is exactly the four
        // tiles of the top-left quadrant (a full-viewport cover would be
        // all sixteen).
        let corner = layer.visible_tiles_in_rect(
            &scope,
            1,
            [0.0, 0.0, 256.0, 256.0],
            &mut TileNodeCache::default(),
        );
        let mut corner_keys = corner
            .iter()
            .map(|tile| (tile.z, tile.x, tile.y))
            .collect::<Vec<_>>();
        corner_keys.sort_unstable();
        assert_eq!(
            corner_keys,
            vec![(2, 0, 0), (2, 0, 1), (2, 1, 0), (2, 1, 1)]
        );

        // Cursor at the center: the central 2×2 block instead.
        let center = layer.visible_tiles_in_rect(
            &scope,
            1,
            [128.0, 128.0, 384.0, 384.0],
            &mut TileNodeCache::default(),
        );
        let mut center_keys = center
            .iter()
            .map(|tile| (tile.z, tile.x, tile.y))
            .collect::<Vec<_>>();
        center_keys.sort_unstable();
        assert_eq!(
            center_keys,
            vec![(2, 1, 1), (2, 1, 2), (2, 2, 1), (2, 2, 2)]
        );
    }

    #[test]
    fn warped_restricted_cover_is_a_subset_of_the_full_cover() {
        let measurement = measurement(albers_conus(), (-96.0, 38.0), 4.0, (620.0, 400.0));
        let scope = TileViewScope::new(&measurement);
        let layer = layer().min_zoom(2).max_zoom(6);
        let full = layer
            .visible_tiles_in_rect(
                &scope,
                1,
                [0.0, 0.0, 620.0, 400.0],
                &mut TileNodeCache::default(),
            )
            .iter()
            .map(|tile| tile.resource_key.clone())
            .collect::<HashSet<_>>();
        let restricted = layer
            .visible_tiles_in_rect(
                &scope,
                1,
                [0.0, 0.0, 155.0, 100.0],
                &mut TileNodeCache::default(),
            )
            .iter()
            .map(|tile| tile.resource_key.clone())
            .collect::<HashSet<_>>();
        assert!(!restricted.is_empty());
        assert!(restricted.is_subset(&full));
        assert!(
            restricted.len() < full.len(),
            "corner rect should select fewer tiles: {} vs {}",
            restricted.len(),
            full.len()
        );
    }

    #[test]
    fn prefetch_requests_follow_the_focus() {
        let measurement = measurement(
            Projection::new(ProjectionKind::Mercator),
            (0.0, 0.0),
            1.0,
            (512.0, 512.0),
        );
        let scope = TileViewScope::new(&measurement);
        let layer = layer()
            .min_zoom(0)
            .max_zoom(4)
            .loading_policy(TileLoadingPolicy::SmoothZoom {
                fallback_below: 0,
                fallback_above: 0,
                prefetch_below: 0,
                prefetch_above: 1,
                pan_prefetch_margin_tiles: 0,
                prefetch_coarse_delta: None,
                max_rendered_fallback_tiles: 128,
                max_prefetch_tiles: 128,
            });
        let targets = layer.visible_tiles(&scope).expect("targets");

        let corner =
            layer.plan_prefetch_requests(&scope, "map", &targets, Some([0.0, 0.0]), [0.0, 0.0]);
        let corner_keys = corner
            .iter()
            .map(|request| request.key.clone())
            .collect::<HashSet<_>>();
        assert_eq!(corner_keys.len(), 4);
        assert!(corner_keys.contains(&ResourceKey::new("geo/base/2/0/0/256")));
        assert!(!corner_keys.contains(&ResourceKey::new("geo/base/2/2/2/256")));

        let opposite =
            layer.plan_prefetch_requests(&scope, "map", &targets, Some([512.0, 512.0]), [0.0, 0.0]);
        let opposite_keys = opposite
            .iter()
            .map(|request| request.key.clone())
            .collect::<HashSet<_>>();
        assert!(opposite_keys.contains(&ResourceKey::new("geo/base/2/3/3/256")));
        assert!(!opposite_keys.contains(&ResourceKey::new("geo/base/2/0/0/256")));

        // No focus → plot-center fallback; out-of-plot focus is ignored too.
        let center = layer.plan_prefetch_requests(&scope, "map", &targets, None, [0.0, 0.0]);
        let off_plot =
            layer.plan_prefetch_requests(&scope, "map", &targets, Some([-50.0, 900.0]), [0.0, 0.0]);
        let center_keys = center
            .iter()
            .map(|request| request.key.clone())
            .collect::<HashSet<_>>();
        let off_plot_keys = off_plot
            .iter()
            .map(|request| request.key.clone())
            .collect::<HashSet<_>>();
        assert_eq!(center_keys, off_plot_keys);
        assert!(center_keys.contains(&ResourceKey::new("geo/base/2/1/1/256")));

        // All requests carry the scope and focus-relative priorities; the
        // nearest tile to the focus outranks the farthest.
        let nearest = corner
            .iter()
            .max_by(|a, b| a.priority.total_cmp(&b.priority))
            .expect("nonempty");
        assert_eq!(nearest.key, ResourceKey::new("geo/base/2/0/0/256"));
        assert!(
            corner
                .iter()
                .all(|request| request.prefetch_scope == Some(PrefetchScope::new("geo/map/base")))
        );
    }

    /// Frame contract: `screen_center` is canvas px (plot center +
    /// plot origin), and the hover-retarget planner — which receives a
    /// canvas-px cursor — produces the same request set as a direct plan
    /// at the equivalent plot-relative focus.
    #[test]
    fn screen_center_is_canvas_frame_and_planner_agrees() {
        let measurement = measurement(
            Projection::new(ProjectionKind::Mercator),
            (0.0, 0.0),
            1.0,
            (512.0, 512.0),
        );
        let scope = TileViewScope::new(&measurement);
        let layer = layer()
            .min_zoom(0)
            .max_zoom(4)
            .loading_policy(TileLoadingPolicy::SmoothZoom {
                fallback_below: 0,
                fallback_above: 0,
                prefetch_below: 0,
                prefetch_above: 1,
                pan_prefetch_margin_tiles: 1,
                prefetch_coarse_delta: None,
                max_rendered_fallback_tiles: 128,
                max_prefetch_tiles: 128,
            });
        let targets = layer.visible_tiles(&scope).expect("targets");
        let plot_origin = [100.0_f32, 50.0_f32];
        let plot_focus = [64.0_f32, 32.0_f32];

        let at_origin =
            layer.plan_prefetch_requests(&scope, "map", &targets, Some(plot_focus), [0.0, 0.0]);
        let offset =
            layer.plan_prefetch_requests(&scope, "map", &targets, Some(plot_focus), plot_origin);
        assert_eq!(at_origin.len(), offset.len());
        for (a, b) in at_origin.iter().zip(&offset) {
            assert_eq!(a.key, b.key);
            assert_eq!(a.priority, b.priority, "priorities are frame-independent");
            let (ca, cb) = (a.screen_center.unwrap(), b.screen_center.unwrap());
            assert_eq!(cb[0], ca[0] + plot_origin[0]);
            assert_eq!(cb[1], ca[1] + plot_origin[1]);
        }

        // Planner with the same origin, given the equivalent CANVAS
        // cursor, reproduces the offset plan exactly.
        let planner = GeoZoomPrefetchPlanner::from_snapshot(
            layer.clone(),
            &measurement,
            targets.clone(),
            plot_origin,
        );
        let planned = avenger_resource::PrefetchRetargetPlanner::plan(
            &planner,
            [
                plot_focus[0] + plot_origin[0],
                plot_focus[1] + plot_origin[1],
            ],
        )
        .expect("cursor inside plot");
        assert_eq!(planned.len(), offset.len());
        for (a, b) in planned.iter().zip(&offset) {
            assert_eq!(a.key, b.key);
            assert_eq!(a.screen_center, b.screen_center);
            assert_eq!(a.priority, b.priority);
        }
    }

    /// Deep-zoom cost probe: run explicitly with
    /// `cargo test --release -p avenger-chart-geo --lib probe_deep_zoom -- --ignored --nocapture`
    #[test]
    #[ignore = "perf probe, not a correctness test"]
    fn probe_deep_zoom_costs() {
        use std::time::Instant;
        for zoom in [4.0, 6.0, 8.0, 10.0, 12.0, 14.0] {
            let mut m = measurement(albers_conus(), (-99.0, 31.5), zoom, (900.0, 600.0));
            m.blend = Some(crate::coord::BlendConfig {
                z0: 4.0,
                z1: 12.0,
                force_t: None,
            });
            let layer = layer().max_zoom(19).smooth_zoom();
            let plan = layer.tile_plan(&m, [0.0, 0.0]).expect("plan");
            let iters = 20;
            let start = Instant::now();
            for _ in 0..iters {
                let _ = layer.tile_plan(&m, [0.0, 0.0]).expect("plan");
            }
            let plan_ms = start.elapsed().as_secs_f64() * 1000.0 / iters as f64;

            let projector = m.view_projector();
            let mut mesh_count = 0usize;
            let mut vert_count = 0usize;
            let start = Instant::now();
            for _ in 0..iters {
                mesh_count = 0;
                vert_count = 0;
                for planned in &plan.rendered_tiles {
                    if let Some(mesh) = tile_mesh(
                        &planned.tile,
                        &projector,
                        m.projection.precision,
                        900.0,
                        600.0,
                    ) {
                        mesh_count += 1;
                        vert_count += mesh.positions.len();
                    }
                }
            }
            let mesh_ms = start.elapsed().as_secs_f64() * 1000.0 / iters as f64;

            // Structure diagnostics: unique nodes visited and the raw
            // (pre-truncation) enumeration.
            let scope = TileViewScope::new(&m);
            let mut diag_cache = TileNodeCache::default();
            let raw_targets = layer.enumerate_tiles(&scope, 0, None, &mut diag_cache);
            let mut z_hist: std::collections::BTreeMap<u8, usize> = Default::default();
            for tile in &raw_targets {
                *z_hist.entry(tile.z).or_default() += 1;
            }
            println!(
                "         nodes={} raw_targets={} z_hist={:?}",
                diag_cache.bbox.len(),
                raw_targets.len(),
                z_hist
            );
            if let Some((&max_z, _)) = z_hist.iter().next_back() {
                let outliers: Vec<_> = raw_targets
                    .iter()
                    .filter(|tile| tile.z == max_z)
                    .take(3)
                    .map(|tile| {
                        let n = MercatorTileGrid::tile_count(tile.z) as f64;
                        let bbox =
                            scope.node_bbox(tile.z, tile.unwrapped_x, tile.y, &mut diag_cache);
                        (
                            format!("{:.3},{:.3}", tile.x as f64 / n, tile.y as f64 / n),
                            bbox.map(|b| {
                                format!(
                                    "[{:.0},{:.0}..{:.0},{:.0}]{}",
                                    b.primary[0],
                                    b.primary[1],
                                    b.primary[2],
                                    b.primary[3],
                                    if b.secondary.is_some() { "+split" } else { "" }
                                )
                            }),
                        )
                    })
                    .collect();
                println!("         deepest-z samples (x/n,y/n bbox-px): {outliers:?}");
            }

            // Per-call projection cost through the view projector.
            let start = Instant::now();
            let mut sink = 0.0f64;
            for i in 0..10_000 {
                let lon = -110.0 + (i % 100) as f64 * 0.3;
                let lat = 25.0 + (i / 100) as f64 * 0.2;
                if let Some((px, py)) = projector.project(lon, lat) {
                    sink += px + py;
                }
            }
            let ns_per_call = start.elapsed().as_nanos() as f64 / 10_000.0;
            std::hint::black_box(sink);
            println!("         projector.project: {ns_per_call:.0} ns/call");
            println!(
                "zoom {zoom:>5}: blend_t={:.2} rendered={:>3} prefetch={:>3} plan={plan_ms:7.3}ms mesh={mesh_ms:7.3}ms ({mesh_count} meshes, {vert_count} verts)",
                m.blend_t(),
                plan.rendered_tiles.len(),
                plan.prefetch_requests.len(),
            );
        }
    }

    #[test]
    fn url_template_and_resource_key_expansion() {
        let tile = layer().visible_tile(3, 1, 2, 9);
        assert_eq!(tile.url, "https://tiles.example/3/1/2.png");
        assert_eq!(tile.resource_key, ResourceKey::new("geo/base/3/1/2/256"));
        assert_eq!(tile.unwrapped_x, 9);
    }

    #[test]
    fn raster_tile_layer_survives_bincode() {
        let layer = layer()
            .min_zoom(1)
            .max_zoom(5)
            .attribution("Tile contributors")
            .smooth_zoom();
        let bytes = bincode::serialize(&layer).expect("serialize");
        let restored: RasterTileLayer = bincode::deserialize(&bytes).expect("deserialize");
        assert_eq!(restored, layer);
    }
}
