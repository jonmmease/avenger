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
/// The tiles meet without cracks: their meshes share their edges' vertices. Each tile draws its
/// own image as it was fetched, and smooth sampling repeats the tile's edge texels at its border.
/// Where the map magnifies tiles past one texel per output unit, a border can show the step
/// between two tiles' edge texels that a stitched image would blend. Each tile draws with the
/// same image every frame, so a renderer that keeps textures between frames uploads it once.
#[derive(Debug, Clone, Default)]
pub struct TileLayer {
    tiles: BTreeMap<TileId, Arc<RgbaImage>>,
}

impl TileLayer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a tile's image, or replace it and return the one it had.
    pub fn insert(&mut self, tile: TileId, image: Arc<RgbaImage>) -> Option<Arc<RgbaImage>> {
        self.tiles.insert(tile, image)
    }

    /// Remove a tile's image and return it.
    pub fn remove(&mut self, tile: TileId) -> Option<Arc<RgbaImage>> {
        self.tiles.remove(&tile)
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
        &self,
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
        shown.retain(|tile| self.tiles.get(tile).is_some_and(|image| image.has_pixels()));
        shown.sort();
        let mut instances = Vec::new();
        for tile in shown {
            let mesh = mesher.mesh(tile)?;
            if mesh.indices.is_empty() {
                continue;
            }
            instances.push(MeshImage {
                image: self.tiles[&tile].clone().into(),
                positions: mesh.positions,
                uvs: mesh.uvs,
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
}

impl FromIterator<(TileId, Arc<RgbaImage>)> for TileLayer {
    fn from_iter<I: IntoIterator<Item = (TileId, Arc<RgbaImage>)>>(tiles: I) -> Self {
        Self {
            tiles: tiles.into_iter().collect(),
        }
    }
}
