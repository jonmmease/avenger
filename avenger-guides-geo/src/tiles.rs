//! Raster tiles: Web Mercator tile images drawn on the map's projection.

use std::collections::BTreeMap;
use std::sync::Arc;

use avenger_geo::paths::geometry_path;
use avenger_geo::tiles::{visible_tiles, TileId, TileMesher};
use avenger_geo::Projector;
use avenger_image::RgbaImage;
use avenger_scenegraph::marks::group::{Clip, SceneGroup};
use avenger_scenegraph::marks::mesh_image::{MeshImage, SceneMeshImageMark};

use crate::error::AvengerGuidesGeoError;

/// A layer of Web Mercator tiles that draws on a map's projection, frame after frame. It holds
/// the tiles' images by address, and [`TileLayer::group`] draws the ones a view shows at a zoom
/// as one mesh image mark whose instances are the tiles. Fetching the images stays with the
/// caller, which adds them with [`TileLayer::insert`] as they arrive and removes the ones it no
/// longer keeps.
///
/// The tiles meet without cracks or filtering steps. Their meshes share their edges' vertices,
/// and each instance's image surrounds the tile's texels with a one-texel apron copied from its
/// neighbors in the layer, so smooth sampling blends across each border as one stitched image
/// would. Where a neighbor is missing or has another size, the apron repeats the tile's own
/// edge, and a corner without its diagonal neighbor takes the neighbor across an edge. The
/// layer keeps each tile's image in its apron until the tile or a neighbor changes, so a view
/// that pans, zooms, or turns only meshes the tiles again, and each tile draws with the same
/// image every frame.
#[derive(Debug, Clone, Default)]
pub struct TileLayer {
    tiles: BTreeMap<TileId, Tile>,
}

#[derive(Debug, Clone)]
struct Tile {
    image: Arc<RgbaImage>,
    /// The image in its apron, once the layer has drawn the tile with its current neighbors.
    apron: Option<Arc<RgbaImage>>,
}

impl TileLayer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a tile's image, or replace it and return the one it had. The tile's neighbors take
    /// their aprons from it the next time they draw.
    pub fn insert(&mut self, tile: TileId, image: Arc<RgbaImage>) -> Option<Arc<RgbaImage>> {
        self.forget_aprons_around(tile);
        self.tiles
            .insert(tile, Tile { image, apron: None })
            .map(|old| old.image)
    }

    /// Remove a tile's image and return it.
    pub fn remove(&mut self, tile: TileId) -> Option<Arc<RgbaImage>> {
        let removed = self.tiles.remove(&tile)?;
        self.forget_aprons_around(tile);
        Some(removed.image)
    }

    /// Whether the layer has a tile's image.
    pub fn contains(&self, tile: TileId) -> bool {
        self.tiles.contains_key(&tile)
    }

    /// The tiles of `zoom` that the view through `projector` shows and the layer has: a group
    /// with one mesh image mark whose instances are the tiles, in the order of their addresses,
    /// or no marks when none of them shows. The group clips to the projection's outline, which
    /// [`crate::make_sphere_marks`] also draws, so the layer ends at the globe's horizon, the
    /// antimeridian's cut, a circle or region clip, or the clip extent. Draw it after the
    /// sphere and before the map's data, and credit the tile source, for example with an
    /// attribution guide.
    ///
    /// # Errors
    /// Returns an error for a planar projection and for a zoom beyond
    /// [`avenger_geo::tiles::MAX_ZOOM`].
    pub fn group(
        &mut self,
        projector: &Projector,
        zoom: u8,
    ) -> Result<SceneGroup, AvengerGuidesGeoError> {
        let mut group = SceneGroup {
            name: "tile_layer".to_string(),
            ..Default::default()
        };
        let mut mesher = TileMesher::new(projector, zoom)?;
        let outline = geometry_path(&projector.project_sphere());
        if outline.iter().next().is_none() {
            return Ok(group);
        }
        let mut shown = visible_tiles(projector, zoom)?;
        shown.retain(|tile| self.tiles.get(tile).is_some_and(|it| it.image.has_pixels()));
        shown.sort();
        let mut instances = Vec::new();
        for tile in shown {
            let mesh = mesher.mesh(tile)?;
            if mesh.indices.is_empty() {
                continue;
            }
            let image = self.apron(tile);
            let (width, height) = ((image.width - 2) as f32, (image.height - 2) as f32);
            let uvs = mesh
                .uvs
                .iter()
                .map(|&[u, v]| {
                    [
                        (1.0 + u * width) / (width + 2.0),
                        (1.0 + v * height) / (height + 2.0),
                    ]
                })
                .collect();
            instances.push(MeshImage {
                image,
                positions: mesh.positions,
                uvs,
                indices: mesh.indices,
            });
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

    /// A tile's image in its apron, made once for its current neighbors.
    fn apron(&mut self, tile: TileId) -> Arc<RgbaImage> {
        if let Some(apron) = self.tiles.get(&tile).and_then(|it| it.apron.clone()) {
            return apron;
        }
        let image = &self.tiles[&tile].image;
        let apron = Arc::new(with_apron(tile, image, &self.tiles));
        if let Some(entry) = self.tiles.get_mut(&tile) {
            entry.apron = Some(apron.clone());
        }
        apron
    }

    /// Forget the aprons of a tile and its neighbors, which take texels from it.
    fn forget_aprons_around(&mut self, tile: TileId) {
        for dy in -1..=1 {
            for dx in -1..=1 {
                if let Some(entry) = tile.neighbor(dx, dy).and_then(|it| self.tiles.get_mut(&it)) {
                    entry.apron = None;
                }
            }
        }
    }
}

impl FromIterator<(TileId, Arc<RgbaImage>)> for TileLayer {
    fn from_iter<I: IntoIterator<Item = (TileId, Arc<RgbaImage>)>>(tiles: I) -> Self {
        let mut layer = Self::new();
        for (tile, image) in tiles {
            layer.insert(tile, image);
        }
        layer
    }
}

/// A tile's image inside a one-texel apron: the facing edges of its eight neighbors, or the
/// tile's own edge where a neighbor is missing or has another size.
fn with_apron(tile: TileId, image: &RgbaImage, tiles: &BTreeMap<TileId, Tile>) -> RgbaImage {
    let (width, height) = (image.width as usize, image.height as usize);
    let (last_x, last_y) = (width - 1, height - 1);
    let neighbor = |dx: i64, dy: i64| {
        tile.neighbor(dx, dy)
            .and_then(|id| tiles.get(&id))
            .map(|it| it.image.as_ref())
            .filter(|other| {
                other.width == image.width && other.height == image.height && other.has_pixels()
            })
    };
    let texel = |image: &RgbaImage, x: usize, y: usize| {
        let start = (y * width + x) * 4;
        <[u8; 4]>::try_from(&image.data[start..start + 4]).expect("four channels")
    };
    // The apron texel beside the tile's texel at `x` and `y`, toward the neighbor `dx` columns
    // east and `dy` rows south: the neighbor's texel on its facing side. Across a corner without
    // a neighbor, the neighbor across either edge stands in, as clamping a stitched image of the
    // tiles would give, and without either the tile repeats its own texel.
    let beside = |dx: i64, dy: i64, x: usize, y: usize| {
        let facing = [(dx, dy), (dx, 0), (0, dy)]
            .into_iter()
            .filter(|&step| step != (0, 0))
            .find_map(|(dx, dy)| Some((dx, dy, neighbor(dx, dy)?)));
        match facing {
            Some((dx, dy, other)) => {
                let x = match dx {
                    -1 => last_x,
                    1 => 0,
                    _ => x,
                };
                let y = match dy {
                    -1 => last_y,
                    1 => 0,
                    _ => y,
                };
                texel(other, x, y)
            }
            None => texel(image, x, y),
        }
    };
    let mut data = Vec::with_capacity((width + 2) * (height + 2) * 4);
    let mut push = |west: [u8; 4], middle: &[u8], east: [u8; 4]| {
        data.extend_from_slice(&west);
        data.extend_from_slice(middle);
        data.extend_from_slice(&east);
    };
    let north = neighbor(0, -1).map_or(row(image, 0), |other| row(other, last_y));
    push(beside(-1, -1, 0, 0), north, beside(1, -1, last_x, 0));
    for y in 0..height {
        push(beside(-1, 0, 0, y), row(image, y), beside(1, 0, last_x, y));
    }
    let south = neighbor(0, 1).map_or(row(image, last_y), |other| row(other, 0));
    push(
        beside(-1, 1, 0, last_y),
        south,
        beside(1, 1, last_x, last_y),
    );
    RgbaImage {
        width: image.width + 2,
        height: image.height + 2,
        data,
    }
}

/// A row of an image's texels.
fn row(image: &RgbaImage, y: usize) -> &[u8] {
    let width = image.width as usize * 4;
    &image.data[y * width..(y + 1) * width]
}
