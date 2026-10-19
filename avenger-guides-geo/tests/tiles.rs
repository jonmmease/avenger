//! A tile layer draws each tile as an instance with the tile's own image, every frame, and clips
//! to the projection's outline.

use std::sync::Arc;

use avenger_geo::paths::geometry_path;
use avenger_geo::projections::{Mercator, Orthographic};
use avenger_geo::tiles::TileId;
use avenger_geo::{Projection, Projector};
use avenger_guides_geo::TileLayer;
use avenger_image::RgbaImage;
use avenger_scenegraph::marks::group::{Clip, SceneGroup};
use avenger_scenegraph::marks::image::SceneImageSource;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::mesh_image::MeshImage;

/// The texels a side of the test tiles.
const SIZE: u32 = 4;

/// A tile whose texels name the tile.
fn tile_image(tile: TileId) -> Arc<RgbaImage> {
    let texel = [(tile.x() * 16 + tile.y()) as u8, 0, 0, 255];
    Arc::new(RgbaImage {
        width: SIZE,
        height: SIZE,
        data: texel.repeat((SIZE * SIZE) as usize),
    })
}

fn tiles(ids: &[(u8, u32, u32)]) -> TileLayer {
    ids.iter()
        .map(|&(z, x, y)| {
            let tile = TileId::new(z, x, y).unwrap();
            (tile, tile_image(tile))
        })
        .collect()
}

/// Web Mercator at a tile zoom, showing the whole square.
fn web_mercator(zoom: u8) -> Projector {
    let side = f64::from(SIZE << zoom);
    Projection::new(Mercator)
        .with_view(side / std::f64::consts::TAU, [side / 2.0, side / 2.0])
        .build()
        .unwrap()
}

fn instances(layer: &SceneGroup) -> Vec<MeshImage> {
    match layer.marks.as_slice() {
        [SceneMark::MeshImage(mark)] => mark.instances.clone(),
        other => panic!("{other:?}"),
    }
}

/// Each instance draws its tile's own image, the same one every frame, with texture coordinates
/// over the whole image.
#[test]
fn tiles_draw_their_own_images() {
    let ids = [(1, 0, 0), (1, 0, 1), (1, 1, 0), (1, 1, 1)];
    let images: Vec<Arc<RgbaImage>> = ids
        .iter()
        .map(|&(z, x, y)| tile_image(TileId::new(z, x, y).unwrap()))
        .collect();
    let layer: TileLayer = ids
        .iter()
        .zip(&images)
        .map(|(&(z, x, y), image)| (TileId::new(z, x, y).unwrap(), image.clone()))
        .collect();
    for _ in 0..2 {
        let drawn = instances(&layer.group(&web_mercator(1), 1).unwrap());
        assert_eq!(drawn.len(), 4);
        for (instance, image) in drawn.iter().zip(&images) {
            let SceneImageSource::Inline(drawn) = &instance.image else {
                panic!("expected an inline image");
            };
            assert!(Arc::ptr_eq(drawn, image));
            for axis in 0..2 {
                let values = instance.uvs.iter().map(|uv| uv[axis]);
                let min = values.clone().fold(f32::INFINITY, f32::min);
                let max = values.fold(f32::NEG_INFINITY, f32::max);
                assert!(
                    min.abs() < 1e-6 && (max - 1.0).abs() < 1e-6,
                    "{min} to {max}"
                );
            }
        }
    }
}

#[test]
fn the_layer_clips_to_the_outline() {
    let globe = Projection::new(Orthographic)
        .with_view(100.0, [120.0, 120.0])
        .with_rotate([-20.0, -40.0, 0.0])
        .build()
        .unwrap();
    let ids: Vec<(u8, u32, u32)> = (0..4)
        .flat_map(|x| (0..4).map(move |y| (2, x, y)))
        .collect();
    let layer = tiles(&ids).group(&globe, 2).unwrap();
    assert_eq!(
        layer.clip,
        Clip::Path(geometry_path(&globe.project_sphere()))
    );
    // Tiles on the far side of the globe have no triangles and no instance.
    let drawn = instances(&layer).len();
    assert!((1..16).contains(&drawn), "{drawn}");
}

/// A layer holds tiles of any zoom and draws the ones of the zoom it's asked for, on a
/// spherical projection.
#[test]
fn a_layer_draws_one_zoom_on_a_spherical_projection() {
    let layer = tiles(&[(1, 0, 0), (1, 1, 0), (2, 0, 0)]);
    assert_eq!(
        instances(&layer.group(&web_mercator(1), 1).unwrap()).len(),
        2
    );
    assert_eq!(
        instances(&layer.group(&web_mercator(2), 2).unwrap()).len(),
        1
    );
    assert!(layer.group(&web_mercator(3), 3).unwrap().marks.is_empty());
    let planar = Projection::planar(false)
        .with_view(1.0, [0.0, 0.0])
        .build()
        .unwrap();
    assert!(layer.group(&planar, 1).is_err());
}
