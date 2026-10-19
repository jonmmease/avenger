//! A tile layer draws its view's tiles as image resources, nearest the view's center first, over
//! the coarser tiles that its resolver holds, and clips to the projection's outline.

use std::sync::Arc;

use avenger_geo::paths::geometry_path;
use avenger_geo::projections::{Mercator, Orthographic};
use avenger_geo::tiles::TileId;
use avenger_geo::{Projection, Projector};
use avenger_guides_geo::{TileLayer, TileSource};
use avenger_image::{ImageResolver, ImageState, RgbaImage};
use avenger_render_fixtures::resources::TestResolver;
use avenger_scenegraph::marks::group::{Clip, SceneGroup};
use avenger_scenegraph::marks::image::SceneImageSource;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::mesh_image::MeshImage;
use avenger_scenegraph::scene_graph::SceneGraph;

/// The texels a side of the test tiles.
const SIZE: u32 = 4;

/// Answer each of `tiles`, as zoom, column, and row, with an image of its own, and return the
/// images.
fn hold(resolver: &TestResolver, tiles: &[(u8, u32, u32)]) -> Vec<Arc<RgbaImage>> {
    tiles
        .iter()
        .map(|&(z, x, y)| {
            let image = Arc::new(RgbaImage {
                width: SIZE,
                height: SIZE,
                data: [z, x as u8, y as u8, 255].repeat((SIZE * SIZE) as usize),
            });
            resolver.set(&format!("t/{z}/{x}/{y}"), ImageState::Ready(image.clone()));
            image
        })
        .collect()
}

fn layer(resolver: &Arc<TestResolver>) -> TileLayer {
    TileLayer::new(
        TileSource::new("t/{z}/{x}/{y}", resolver.clone())
            .unwrap()
            .with_tile_size(SIZE),
    )
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

/// The URLs of the resource instances, and the inline instances' images, in order.
fn split(instances: &[MeshImage]) -> (Vec<String>, Vec<Arc<RgbaImage>>) {
    let mut resources = Vec::new();
    let mut inline = Vec::new();
    for instance in instances {
        match &instance.image {
            SceneImageSource::Resource(resource) => resources.push(resource.url.clone()),
            SceneImageSource::Inline(image) => {
                assert!(
                    resources.is_empty(),
                    "an inline tile draws over a visible one"
                );
                inline.push(image.clone());
            }
        }
    }
    (resources, inline)
}

/// The visible tiles are resources of the source's size, with texture coordinates over the whole
/// image, nearest the view's center first. The layer itself requests none of them.
#[test]
fn visible_tiles_are_resources_nearest_the_center_first() {
    let resolver = Arc::new(TestResolver::default());
    let drawn = instances(&layer(&resolver).group(&web_mercator(2), 2).unwrap());
    let (resources, inline) = split(&drawn);
    assert!(inline.is_empty());
    assert_eq!(resources.len(), 16);
    let mut center = resources[..4].to_vec();
    center.sort();
    assert_eq!(center, ["t/2/1/1", "t/2/1/2", "t/2/2/1", "t/2/2/2"]);
    for instance in &drawn {
        let SceneImageSource::Resource(resource) = &instance.image else {
            unreachable!()
        };
        assert_eq!([resource.width, resource.height], [SIZE, SIZE]);
        assert!(resource.placeholder.is_none());
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
    assert!(resolver.requested().is_empty());
}

/// Beneath each visible tile that isn't ready, the nearest coarser tile that the resolver holds
/// draws, once, as the resolver's own image. A ready tile gets none.
#[test]
fn a_pending_tile_draws_over_its_nearest_ready_ancestor() {
    let resolver = Arc::new(TestResolver::default());
    let held = hold(&resolver, &[(1, 0, 0), (2, 1, 1), (3, 2, 2)]);
    let group = layer(&resolver).group(&web_mercator(3), 3).unwrap();
    let (resources, inline) = split(&instances(&group));
    assert_eq!(resources.len(), 64);
    // Tile 3/2/2 is ready, and its siblings draw over 2/1/1. The rest of 1/0/0's tiles draw over
    // it, and the tiles of the other three zoom 1 tiles have no ready ancestor finer than the
    // cover, tile 0/0/0. The coarser fallback draws first, beneath the finer one.
    let found: Vec<_> = inline
        .iter()
        .map(|image| {
            held.iter()
                .position(|held| Arc::ptr_eq(held, image))
                .expect("a held image")
        })
        .collect();
    assert_eq!(found, [0, 1]);
    // The coarser tiles are inline, so the scene's resources are the visible tiles alone.
    let scene = SceneGraph {
        marks: vec![group.into()],
        width: 32.0,
        height: 32.0,
        origin: [0.0; 2],
    };
    assert_eq!(scene.image_urls().len(), 64);
}

/// The coarse cover, four zooms up, is prefetched, held until the next group, and drawn beneath
/// every other tile where the resolver holds it.
#[test]
fn the_coarse_cover_is_prefetched_held_and_drawn_beneath() {
    let resolver = Arc::new(TestResolver::default());
    let held = hold(&resolver, &[(1, 0, 0), (2, 0, 0)]);
    let layer = layer(&resolver);
    let (_, inline) = split(&instances(&layer.group(&web_mercator(5), 5).unwrap()));
    let mut cover = resolver.prefetched();
    cover.sort();
    assert_eq!(cover, ["t/1/0/0", "t/1/0/1", "t/1/1/0", "t/1/1/1"]);
    assert!(cover.iter().all(|url| resolver.held(url) == 1));
    // The cover draws first, and 2/0/0 draws over it beneath its pending tiles.
    assert_eq!(inline.len(), 2);
    assert!(Arc::ptr_eq(&inline[0], &held[0]) && Arc::ptr_eq(&inline[1], &held[1]));

    // The next group holds its own cover, of zoom 0, and releases the last one.
    layer.group(&web_mercator(4), 4).unwrap();
    assert!(cover.iter().all(|url| resolver.held(url) == 0));
    assert_eq!(resolver.held("t/0/0/0"), 1);
    assert!(resolver.requested().is_empty());
}

/// A prefetch requests the tiles of each view as prefetches and holds them until the next one.
#[test]
fn a_prefetch_holds_its_views_until_the_next() {
    let resolver = Arc::new(TestResolver::default());
    let layer = layer(&resolver);
    layer.prefetch(&[(&web_mercator(1), 1)]);
    let first: Vec<String> = resolver.prefetched();
    assert_eq!(first.len(), 4);
    assert!(first.iter().all(|url| resolver.held(url) == 1));
    layer.prefetch(&[(&web_mercator(2), 2), (&web_mercator(1), 0)]);
    assert!(first.iter().all(|url| resolver.held(url) == 0));
    assert_eq!(resolver.held("t/2/3/3"), 1);
    assert_eq!(resolver.held("t/0/0/0"), 1);
    assert!(resolver.requested().is_empty());
}

#[test]
fn a_source_names_its_tiles_and_zooms() {
    let resolver: Arc<dyn ImageResolver> = Arc::new(TestResolver::default());
    assert!(TileSource::new("t/{z}/{x}", resolver.clone()).is_err());
    let source = TileSource::new("https://a.test/{z}/{x}/{y}.png", resolver.clone()).unwrap();
    assert_eq!(
        source.url(TileId::new(3, 5, 2).unwrap()),
        "https://a.test/3/5/2.png"
    );
    assert_eq!(source.zooms(), 0..=19);
    assert_eq!(source.tile_size(), 256);
    #[allow(clippy::reversed_empty_ranges)]
    let empty = 5..=3;
    assert!(source.clone().with_zooms(empty).is_err());
    assert!(source.clone().with_zooms(0..=31).is_err());
}

/// The zoom is the tile zoom rounded and clamped to the source's zooms, and the lowest zoom
/// where the view's center misses the map.
#[test]
fn the_zoom_rounds_and_clamps_to_the_source() {
    let resolver = Arc::new(TestResolver::default());
    let source = TileSource::new("t/{z}/{x}/{y}", resolver)
        .unwrap()
        .with_tile_size(SIZE);
    assert_eq!(source.zoom(&web_mercator(2)), 2);
    let zoom = |zooms| {
        source
            .clone()
            .with_zooms(zooms)
            .unwrap()
            .zoom(&web_mercator(2))
    };
    assert_eq!(zoom(3..=19), 3);
    assert_eq!(zoom(0..=1), 1);
    let missed = Projection::new(Orthographic)
        .with_view(100.0, [0.0, 0.0])
        .with_clip_extent(Some([[300.0, 300.0], [400.0, 400.0]]))
        .build()
        .unwrap();
    assert_eq!(source.with_zooms(2..=19).unwrap().zoom(&missed), 2);
}

#[test]
fn the_layer_clips_to_the_outline() {
    let resolver = Arc::new(TestResolver::default());
    let globe = Projection::new(Orthographic)
        .with_view(100.0, [120.0, 120.0])
        .with_rotate([-20.0, -40.0, 0.0])
        .build()
        .unwrap();
    let group = layer(&resolver).group(&globe, 2).unwrap();
    assert_eq!(
        group.clip,
        Clip::Path(geometry_path(&globe.project_sphere()))
    );
    // Tiles on the far side of the globe have no triangles and no instance.
    let drawn = instances(&group).len();
    assert!((1..16).contains(&drawn), "{drawn}");
    let planar = Projection::planar(false)
        .with_view(1.0, [0.0, 0.0])
        .build()
        .unwrap();
    assert!(layer(&resolver).group(&planar, 1).is_err());
}

/// A group prefetches the border, the tiles one beyond the visible ones, and holds them until
/// the next group.
#[test]
fn the_border_is_prefetched_and_held() {
    let resolver = Arc::new(TestResolver::default());
    let layer = layer(&resolver);
    // Web Mercator at zoom 3, clipped to a 2-tile square in the middle of the map.
    let side = f64::from(SIZE << 3);
    let middle = Projection::new(Mercator)
        .with_view(side / std::f64::consts::TAU, [side / 2.0, side / 2.0])
        .with_clip_extent(Some([[12.5, 12.5], [19.5, 19.5]]))
        .build()
        .unwrap();
    let (visible, _) = split(&instances(&layer.group(&middle, 3).unwrap()));
    let mut shown = visible.clone();
    shown.sort();
    assert_eq!(shown, ["t/3/3/3", "t/3/3/4", "t/3/4/3", "t/3/4/4"]);
    let prefetched: Vec<String> = resolver.prefetched();
    let border: Vec<&String> = prefetched
        .iter()
        .filter(|url| url.starts_with("t/3/"))
        .collect();
    // The 4 × 4 square around the visible 2 × 2.
    assert_eq!(border.len(), 12);
    assert!(border
        .iter()
        .all(|url| !visible.contains(url) && resolver.held(url) == 1));
    layer.group(&web_mercator(2), 2).unwrap();
    assert!(border.iter().all(|url| resolver.held(url) == 0));
}
