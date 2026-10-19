//! A tile layer draws each tile as an instance whose image carries its neighbors' edges, keeps
//! those images from frame to frame, and clips to the projection's outline.

use std::sync::Arc;

use avenger_geo::paths::geometry_path;
use avenger_geo::projections::{Mercator, Orthographic};
use avenger_geo::tiles::TileId;
use avenger_geo::{Projection, Projector};
use avenger_guides_geo::TileLayer;
use avenger_image::RgbaImage;
use avenger_scenegraph::marks::group::Clip;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::mesh_image::MeshImage;

/// The texels a side of the test tiles.
const SIZE: u32 = 4;

/// A tile whose texels name the tile and their own place in it.
fn tile_image(tile: TileId) -> Arc<RgbaImage> {
    let mut data = Vec::new();
    for y in 0..SIZE {
        for x in 0..SIZE {
            data.extend([(tile.x() * 16 + tile.y()) as u8, x as u8, y as u8, 255]);
        }
    }
    Arc::new(RgbaImage {
        width: SIZE,
        height: SIZE,
        data,
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

fn instances(layer: &avenger_scenegraph::marks::group::SceneGroup) -> Vec<MeshImage> {
    match layer.marks.as_slice() {
        [SceneMark::MeshImage(mark)] => mark.instances.clone(),
        other => panic!("{other:?}"),
    }
}

/// The texel at `x` and `y` of an image with its apron, from -1 to `SIZE`.
fn texel(image: &RgbaImage, x: i64, y: i64) -> [u8; 4] {
    let start = (((y + 1) * i64::from(image.width) + x + 1) * 4) as usize;
    image.data[start..start + 4].try_into().unwrap()
}

/// What a texel of a tile names: the tile, and its place in that tile.
fn named(tile: (u32, u32), x: u32, y: u32) -> [u8; 4] {
    [(tile.0 * 16 + tile.1) as u8, x as u8, y as u8, 255]
}

#[test]
fn aprons_copy_the_neighbors_edges() {
    let layer = tiles(&[(1, 0, 0), (1, 0, 1), (1, 1, 0), (1, 1, 1)])
        .group(&web_mercator(1), 1)
        .unwrap();
    let instances = instances(&layer);
    assert_eq!(instances.len(), 4);
    // Tile 1/1/0, the northeast quarter, wraps east to tile 1/0/0.
    let image = &instances[2].image;
    assert_eq!((image.width, image.height), (SIZE + 2, SIZE + 2));
    let last = SIZE - 1;
    let end = i64::from(SIZE);
    assert_eq!(texel(image, 0, 0), named((1, 0), 0, 0));
    assert_eq!(texel(image, -1, 2), named((0, 0), last, 2));
    assert_eq!(texel(image, end, 2), named((0, 0), 0, 2));
    assert_eq!(texel(image, 2, end), named((1, 1), 2, 0));
    assert_eq!(texel(image, -1, end), named((0, 1), last, 0));
    assert_eq!(texel(image, end, end), named((0, 1), 0, 0));
    // The north edge has no neighbor, so it repeats the tile's own edge, and its corners take
    // the top texels of the neighbors to the west and east.
    assert_eq!(texel(image, 2, -1), named((1, 0), 2, 0));
    assert_eq!(texel(image, -1, -1), named((0, 0), last, 0));
    assert_eq!(texel(image, end, -1), named((0, 0), 0, 0));
}

#[test]
fn missing_neighbors_repeat_the_tiles_edge() {
    let layer = tiles(&[(2, 1, 1), (2, 2, 1)])
        .group(&web_mercator(2), 2)
        .unwrap();
    let instances = instances(&layer);
    let image = &instances[0].image;
    let (last, end) = (SIZE - 1, i64::from(SIZE));
    assert_eq!(texel(image, end, 1), named((2, 1), 0, 1));
    assert_eq!(texel(image, -1, 1), named((1, 1), 0, 1));
    assert_eq!(texel(image, 1, -1), named((1, 1), 1, 0));
    assert_eq!(texel(image, 1, end), named((1, 1), 1, last));
    // Without the neighbor across a corner, the neighbor across an edge gives the corner.
    assert_eq!(texel(image, end, end), named((2, 1), 0, last));
    assert_eq!(texel(image, -1, end), named((1, 1), 0, last));
}

/// A tile's texture coordinates reach the tile's own texels' outer edges, inside the apron.
#[test]
fn uvs_span_the_inner_texels() {
    let layer = tiles(&[(1, 0, 0)]).group(&web_mercator(1), 1).unwrap();
    let instance = &instances(&layer)[0];
    let side = (SIZE + 2) as f32;
    let (low, high) = (1.0 / side, (SIZE + 1) as f32 / side);
    for axis in 0..2 {
        let values: Vec<f32> = instance.uvs.iter().map(|uv| uv[axis]).collect();
        let min = values.iter().copied().fold(f32::INFINITY, f32::min);
        let max = values.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        assert!(
            (min - low).abs() < 1e-6 && (max - high).abs() < 1e-6,
            "{min} to {max}"
        );
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
    let mut layer = tiles(&[(1, 0, 0), (1, 1, 0), (2, 0, 0)]);
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

/// A tile draws with the same image every frame, until the tile or a neighbor changes.
#[test]
fn tiles_keep_their_images_between_frames() {
    let mut layer = tiles(&[(2, 0, 0), (2, 1, 0), (2, 3, 3)]);
    let images = |layer: &mut TileLayer| -> Vec<Arc<RgbaImage>> {
        instances(&layer.group(&web_mercator(2), 2).unwrap())
            .into_iter()
            .map(|instance| instance.image)
            .collect()
    };
    let first = images(&mut layer);
    let second = images(&mut layer);
    assert!(first.iter().zip(&second).all(|(a, b)| Arc::ptr_eq(a, b)));
    // A new tile east of 2/1/0 changes that tile's apron, and leaves the far tile's alone.
    let east = TileId::new(2, 2, 0).unwrap();
    layer.insert(east, tile_image(east));
    let third = images(&mut layer);
    assert_eq!(third.len(), 4);
    assert!(Arc::ptr_eq(&first[0], &third[0]));
    assert!(!Arc::ptr_eq(&first[1], &third[1]));
    assert_eq!(texel(&third[1], i64::from(SIZE), 1), named((2, 0), 0, 1));
    assert!(Arc::ptr_eq(&first[2], &third[3]));
    // Removing it changes the apron back.
    assert!(layer.remove(east).is_some() && !layer.contains(east));
    let fourth = images(&mut layer);
    assert_eq!(
        texel(&fourth[1], i64::from(SIZE), 1),
        named((1, 0), SIZE - 1, 1)
    );
}
