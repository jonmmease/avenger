//! Raster tiles: Web Mercator tile images, fetched by URL and drawn on the map's projection.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::ops::RangeInclusive;
use std::sync::{Arc, Mutex, PoisonError};

use avenger_geo::paths::geometry_path;
use avenger_geo::tiles::{tile_zoom, visible_tiles, TileId, TileMesher, MAX_ZOOM};
use avenger_geo::Projector;
use avenger_image::{ImageLease, ImageResolver};
use avenger_scenegraph::marks::group::{Clip, SceneGroup};
use avenger_scenegraph::marks::image::{SceneImageResource, SceneImageSource};
use avenger_scenegraph::marks::mesh_image::{MeshImage, SceneMeshImageMark};

use crate::error::AvengerGuidesGeoError;

/// How many zooms above the view's zoom the coarse cover is.
const COVER_ZOOMS: u8 = 4;

/// The offsets of a tile's eight neighbors.
const NEIGHBORS: [(i64, i64); 8] = [
    (-1, -1),
    (0, -1),
    (1, -1),
    (-1, 0),
    (1, 0),
    (-1, 1),
    (0, 1),
    (1, 1),
];

/// Where a tile layer's tiles come from: a URL template, the resolver that loads the tiles, the
/// zooms that the server has, and the tiles' size.
#[derive(Clone)]
pub struct TileSource {
    template: String,
    resolver: Arc<dyn ImageResolver>,
    zooms: RangeInclusive<u8>,
    tile_size: u32,
}

impl TileSource {
    /// Tiles at the URLs of `template`, loaded through `resolver`. The template names a tile's
    /// zoom, column, and row with `{z}`, `{x}`, and `{y}`, as in
    /// `https://tile.openstreetmap.org/{z}/{x}/{y}.png`. The source has zooms 0 through 19, and
    /// tiles of 256 pixels a side.
    ///
    /// # Errors
    /// Returns an error for a template without `{z}`, `{x}`, or `{y}`.
    pub fn new(
        template: impl Into<String>,
        resolver: Arc<dyn ImageResolver>,
    ) -> Result<Self, AvengerGuidesGeoError> {
        let template = template.into();
        for name in ["{z}", "{x}", "{y}"] {
            if !template.contains(name) {
                return Err(AvengerGuidesGeoError::InvalidConfig(format!(
                    "the tile URL template {template} has no {name}"
                )));
            }
        }
        Ok(Self {
            template,
            resolver,
            zooms: 0..=19,
            tile_size: 256,
        })
    }

    /// The zooms that the server has.
    ///
    /// # Errors
    /// Returns an error for an empty range or one beyond [`MAX_ZOOM`].
    pub fn with_zooms(mut self, zooms: RangeInclusive<u8>) -> Result<Self, AvengerGuidesGeoError> {
        if zooms.is_empty() || *zooms.end() > MAX_ZOOM {
            return Err(AvengerGuidesGeoError::InvalidConfig(format!(
                "tile zooms {zooms:?} must be a range within 0 to {MAX_ZOOM}"
            )));
        }
        self.zooms = zooms;
        Ok(self)
    }

    /// The tiles' size in pixels.
    pub fn with_tile_size(mut self, tile_size: u32) -> Self {
        self.tile_size = tile_size.max(1);
        self
    }

    pub fn zooms(&self) -> RangeInclusive<u8> {
        self.zooms.clone()
    }

    pub fn tile_size(&self) -> u32 {
        self.tile_size
    }

    /// A tile's URL.
    pub fn url(&self, tile: TileId) -> String {
        self.template
            .replace("{z}", &tile.z().to_string())
            .replace("{x}", &tile.x().to_string())
            .replace("{y}", &tile.y().to_string())
    }

    /// The zoom whose tiles draw about one texel per output unit at the view's center:
    /// avenger-geo's `tile_zoom` rounded, within the source's zooms. Where the view's center
    /// misses the map, it's the source's lowest zoom.
    pub fn zoom(&self, projector: &Projector) -> u8 {
        let (min, max) = (*self.zooms.start(), *self.zooms.end());
        tile_zoom(projector, f64::from(self.tile_size)).map_or(min, |zoom| {
            zoom.round().clamp(f64::from(min), f64::from(max)) as u8
        })
    }
}

impl fmt::Debug for TileSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TileSource")
            .field("template", &self.template)
            .field("zooms", &self.zooms)
            .field("tile_size", &self.tile_size)
            .finish_non_exhaustive()
    }
}

/// A layer of Web Mercator tiles from a [`TileSource`], which draws on a map's projection frame
/// after frame. [`TileLayer::group`] draws the tiles that a view shows at a zoom as one mesh
/// image mark whose instances are the tiles, each on its mesh from avenger-geo's `TileMesher`.
///
/// The visible tiles are image resources, which the renderer loads through the source's
/// resolver, so a renderer that draws the layer needs that resolver. A tile draws on the frame
/// after it arrives, without a new scene. Until then, the nearest coarser tile that the resolver
/// holds draws beneath it, so a zoom shows the last zoom's tiles until the new ones arrive.
/// Beneath every tile, the layer draws the coarse cover, the tiles four zooms up that cover the
/// view, which it prefetches, so a fast zoom out or pan shows coarse tiles instead of the
/// sphere's fill. The coarser tiles draw with the resolver's own images. The layer also
/// prefetches the border, the tiles one beyond the visible ones, so a pan or a turn finds the
/// tiles that come into view already fetched.
///
/// The coarser tiles draw beneath every tile, so the layer suits opaque tiles, such as a
/// basemap's. Through a translucent tile, the coarser tiles beneath it would show.
///
/// The tiles meet without cracks: their meshes share their edges' vertices. Each tile draws its
/// own image as it was fetched, and smooth sampling repeats the tile's edge texels at its border.
/// Where the map magnifies tiles past one texel per output unit, a border can show the step
/// between two tiles' edge texels that a stitched image would blend.
///
/// Clones share the layer's leases.
#[derive(Debug, Clone)]
pub struct TileLayer {
    source: TileSource,
    /// Holds the coarse cover and the border until the next group.
    held: Arc<Mutex<ImageLease>>,
    /// Holds the last prefetch's tiles until the next prefetch.
    prefetched: Arc<Mutex<ImageLease>>,
}

impl TileLayer {
    pub fn new(source: TileSource) -> Self {
        Self {
            source,
            held: Arc::default(),
            prefetched: Arc::default(),
        }
    }

    pub fn source(&self) -> &TileSource {
        &self.source
    }

    /// The tiles of `zoom` that the view through `projector` shows: a group with one mesh image
    /// mark, or no marks when no tile draws. The instances run from the coarse cover, to the
    /// coarser tiles beneath the visible tiles that aren't ready, to the visible tiles nearest
    /// the view's center first, which the renderer requests first. The layer prefetches the coarse
    /// cover and the border, and holds them until the next group. The group clips to the
    /// projection's outline, which [`crate::make_sphere_marks`] also draws, so the layer ends at
    /// the globe's horizon, the antimeridian's cut, a circle or region clip, or the clip extent.
    /// The tiles take no hits, as the sphere and the graticule don't. Draw the group after the
    /// sphere and before the map's data, and credit the tile source, for example with an
    /// attribution guide.
    ///
    /// The zoom clamps to the source's zooms.
    ///
    /// # Errors
    /// Returns an error for a planar projection.
    pub fn group(
        &self,
        projector: &Projector,
        zoom: u8,
    ) -> Result<SceneGroup, AvengerGuidesGeoError> {
        let zoom = zoom.clamp(*self.source.zooms.start(), *self.source.zooms.end());
        let mut group = SceneGroup {
            name: "tile_layer".to_string(),
            interactive: Some(false),
            ..Default::default()
        };
        let mut meshers = HashMap::new();
        meshers.insert(zoom, TileMesher::new(projector, zoom)?);
        let outline = geometry_path(&projector.project_sphere());
        if outline.iter().next().is_none() {
            return Ok(group);
        }
        let resolver = &self.source.resolver;
        let visible = by_distance(projector, visible_tiles(projector, zoom)?);

        // The coarse cover, which the layer prefetches and holds until the next group.
        let cover_zoom = zoom
            .saturating_sub(COVER_ZOOMS)
            .max(*self.source.zooms.start());
        let mut cover = Vec::new();
        if cover_zoom < zoom {
            let mut seen = HashSet::new();
            for tile in &visible {
                let ancestor = ancestor(*tile, cover_zoom);
                if seen.insert(ancestor) {
                    cover.push(ancestor);
                }
            }
        }
        let cover_urls: Vec<String> = cover.iter().map(|tile| self.source.url(*tile)).collect();

        // The border, the tiles one beyond the visible ones at their zoom, nearest the view's
        // center first, so a pan or a turn shows tiles that are already fetched.
        let shown: HashSet<TileId> = visible.iter().copied().collect();
        let mut seen = HashSet::new();
        let mut border = Vec::new();
        for tile in &visible {
            for (dx, dy) in NEIGHBORS {
                if let Some(neighbor) = tile.neighbor(dx, dy) {
                    if !shown.contains(&neighbor) && seen.insert(neighbor) {
                        border.push(self.source.url(neighbor));
                    }
                }
            }
        }
        for url in cover_urls.iter().chain(&border) {
            resolver.prefetch(url);
        }
        let urls: Vec<&str> = cover_urls
            .iter()
            .chain(&border)
            .map(String::as_str)
            .collect();
        *lock(&self.held) = resolver.retain(&urls);

        // The nearest ready ancestor, finer than the cover, beneath each visible tile that isn't
        // ready.
        let mut fallbacks = Vec::new();
        let mut fallen = HashSet::new();
        for tile in &visible {
            if resolver.ready(&self.source.url(*tile)).is_some() {
                continue;
            }
            let mut ancestor = *tile;
            while let Some(parent) = ancestor.parent().filter(|parent| parent.z() > cover_zoom) {
                ancestor = parent;
                if let Some(image) = resolver.ready(&self.source.url(ancestor)) {
                    if fallen.insert(ancestor) {
                        fallbacks.push((ancestor, image));
                    }
                    break;
                }
            }
        }

        // Coarser tiles draw first, so a finer fallback draws over a coarser one.
        fallbacks.sort_by_key(|(tile, _)| tile.z());

        let mut instances = Vec::new();
        let mut push =
            |tile: TileId, image: SceneImageSource| -> Result<(), AvengerGuidesGeoError> {
                let mesher = match meshers.entry(tile.z()) {
                    std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        entry.insert(TileMesher::new(projector, tile.z())?)
                    }
                };
                let mesh = mesher.mesh(tile)?;
                if !mesh.indices.is_empty() {
                    instances.push(MeshImage {
                        image,
                        positions: mesh.positions,
                        uvs: mesh.uvs,
                        indices: mesh.indices,
                    });
                }
                Ok(())
            };
        for (tile, url) in cover.iter().zip(&cover_urls) {
            if let Some(image) = resolver.ready(url) {
                push(*tile, image.into())?;
            }
        }
        for (tile, image) in fallbacks {
            push(tile, image.into())?;
        }
        for tile in visible {
            let resource = SceneImageResource {
                url: self.source.url(tile),
                width: self.source.tile_size,
                height: self.source.tile_size,
                placeholder: None,
            };
            push(tile, resource.into())?;
        }
        if !instances.is_empty() {
            group.clip = Clip::Path(outline);
            group.marks.push(
                SceneMeshImageMark {
                    name: "tiles".to_string(),
                    instances,
                    ..Default::default()
                }
                .into(),
            );
        }
        Ok(group)
    }

    /// Prefetch the tiles that each view shows at its zoom, behind every visible tile, and hold
    /// them until the next call, which drops the queued prefetches that it no longer names. An
    /// app passes the views that zooming in and out at the cursor would show, so the tiles of
    /// either zoom are ready when it happens.
    ///
    /// Each zoom clamps to the source's zooms.
    pub fn prefetch(&self, views: &[(&Projector, u8)]) {
        let resolver = &self.source.resolver;
        let mut urls = Vec::new();
        for &(projector, zoom) in views {
            let zoom = zoom.clamp(*self.source.zooms.start(), *self.source.zooms.end());
            // The source's zooms are within the deepest zoom, so the tiles always come.
            let tiles = visible_tiles(projector, zoom).unwrap_or_default();
            for tile in by_distance(projector, tiles) {
                urls.push(self.source.url(tile));
            }
        }
        for url in &urls {
            resolver.prefetch(url);
        }
        let urls: Vec<&str> = urls.iter().map(String::as_str).collect();
        *lock(&self.prefetched) = resolver.retain(&urls);
    }
}

/// The tile's ancestor at zoom `z`, or the tile itself at its own zoom or deeper.
fn ancestor(mut tile: TileId, z: u8) -> TileId {
    while tile.z() > z {
        match tile.parent() {
            Some(parent) => tile = parent,
            None => break,
        }
    }
    tile
}

/// The tiles in order of their centers' distance from the view's center, nearest first. A tile
/// whose center doesn't project comes last.
fn by_distance(projector: &Projector, tiles: Vec<TileId>) -> Vec<TileId> {
    let [cx, cy] = projector.view_center();
    let mut keyed: Vec<(f64, TileId)> = tiles
        .into_iter()
        .map(|tile| {
            let [[west, south], [east, north]] = tile.bounds();
            let distance = projector
                .project_point((west + east) / 2.0, (south + north) / 2.0)
                .map_or(f64::INFINITY, |(x, y)| (x - cx).hypot(y - cy));
            (distance, tile)
        })
        .collect();
    keyed.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    keyed.into_iter().map(|(_, tile)| tile).collect()
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
