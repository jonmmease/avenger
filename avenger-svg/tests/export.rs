#[path = "../../tests/render_fixtures/raster.rs"]
mod raster;
#[path = "../../tests/render_fixtures/scene.rs"]
mod scene;

use avenger_common::value::ScalarOrArray;
use avenger_scenegraph::{
    marks::{
        group::{Clip, SceneGroup},
        mark::SceneMark,
    },
    scene_graph::SceneGraph,
};
use avenger_svg::{SvgBackground, SvgFontEmbedding, SvgRenderOptions, SvgRenderer};

fn renderer() -> SvgRenderer {
    SvgRenderer::new().with_options(SvgRenderOptions {
        font_resolution: scene::fonts(),
        background: SvgBackground::Transparent,
        ..Default::default()
    })
}

#[test]
fn gallery_visual_regression() {
    let svg = renderer()
        .with_options(SvgRenderOptions {
            font_resolution: scene::fonts(),
            ..Default::default()
        })
        .render_scene_graph(&scene::gallery())
        .unwrap();
    let png = raster::svg_to_png(&svg, 2.0);
    raster::assert_baseline(
        &png,
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/baselines/gallery.png"),
    );
}

#[test]
fn clips_complete_markup_in_label_coordinates_and_scene_clip_outside_rotation() {
    for angle in [0.0, 28.0, -28.0] {
        let mut text = scene::text("*Bold* $sqrt(x^2+y^2)$ tail", 10.0, 40.0, 22.0);
        text.limit = ScalarOrArray::new_scalar(70.0);
        text.angle = ScalarOrArray::new_scalar(angle);
        text.clip = true;
        let scene = SceneGraph {
            width: 240.0,
            height: 120.0,
            origin: [0.0, 0.0],
            marks: vec![SceneGroup {
                clip: Clip::Rect {
                    x: 20.0,
                    y: 10.0,
                    width: 100.0,
                    height: 70.0,
                },
                marks: vec![text.into()],
                ..Default::default()
            }
            .into()],
        };
        let svg = renderer().render_scene_graph(&scene).unwrap();
        assert!(
            svg.contains("Bold"),
            "{}",
            svg.split("</defs>").last().unwrap()
        );
        assert!(svg.contains("tail</text>"));
        assert!(!svg.contains("…"));
        let png = raster::svg_to_png(&svg, 2.0);
        let mut visible = 0;
        for (x, y, pixel) in png.enumerate_pixels() {
            if pixel[3] == 0 {
                continue;
            }
            visible += 1;
            let (x, y) = (x as f32 / 2.0, y as f32 / 2.0);
            assert!((19.5..120.5).contains(&x) && (9.5..80.5).contains(&y));
            let radians = angle.to_radians();
            let local_x = (x - 10.0) * radians.cos() + (y - 40.0) * radians.sin();
            assert!(local_x <= 70.5, "{angle}: overflow at {x}, {y}");
        }
        assert!(visible > 100);
    }
}

#[test]
fn uses_supplied_engine_fonts_and_text_colors() {
    let mut options = scene::fonts();
    options.default_sans_serif_family = Some("DejaVu Sans Mono".into());
    let engine = avenger_text::TextEngine::with_font_resolution(&options).unwrap();
    let mut mark = scene::text("R", 10.0, 30.0, 20.0);
    mark.font = ScalarOrArray::new_scalar("sans-serif".into());
    mark.color =
        ScalarOrArray::new_scalar(avenger_color::ColorOrGradient::Color([1.0, 0.0, 0.0, 1.0]));
    let mut blue = mark.clone();
    blue.text = ScalarOrArray::new_scalar("B".into());
    blue.x = ScalarOrArray::new_scalar(35.0);
    blue.color =
        ScalarOrArray::new_scalar(avenger_color::ColorOrGradient::Color([0.0, 0.0, 1.0, 1.0]));
    let scene = SceneGraph {
        width: 100.0,
        height: 50.0,
        origin: [0.0, 0.0],
        marks: vec![mark.into(), blue.into()],
    };
    let svg = renderer()
        .with_text_engine(engine)
        .render_scene_graph(&scene)
        .unwrap();
    assert!(
        svg.contains("#ff0000"),
        "{}",
        svg.split("</defs>").last().unwrap()
    );
    assert!(svg.contains("#0000ff"));
    let png = raster::svg_to_png(&svg, 2.0);
    assert!(png.pixels().any(|p| p[0] > 200 && p[2] < 20 && p[3] > 200));
    assert!(png.pixels().any(|p| p[2] > 200 && p[0] < 20 && p[3] > 200));
    let uri = svg
        .split("src: url(\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    let (header, payload) = uri.split_once(',').unwrap();
    use base64::Engine;
    let bytes = base64::prelude::BASE64_STANDARD.decode(payload).unwrap();
    let sfnt = if header.contains("woff2") {
        font_subset::FontReader::new(&bytes)
            .unwrap()
            .read()
            .unwrap()
            .to_opentype()
    } else {
        bytes
    };
    let face = ttf_parser::Face::parse(&sfnt, 0).unwrap();
    assert!(face.is_monospaced());
}

#[test]
fn missing_fonts_error_even_without_embedding() {
    let mut mark = scene::text("Label", 10.0, 20.0, 12.0);
    mark.font = ScalarOrArray::new_scalar("Missing font fixture".into());
    let scene = SceneGraph {
        width: 100.0,
        height: 50.0,
        origin: [0.0, 0.0],
        marks: vec![mark.into()],
    };
    for embedding in [SvgFontEmbedding::None, SvgFontEmbedding::EmbedSubsetWoff2] {
        let result = renderer()
            .with_options(SvgRenderOptions {
                font_resolution: scene::fonts(),
                font_embedding: embedding,
                ..Default::default()
            })
            .render_scene_graph(&scene);
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Missing font fixture"));
    }
}

#[test]
fn rejects_invalid_page_dimensions() {
    for width in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        assert!(renderer()
            .render_scene_graph(&SceneGraph {
                width,
                height: 50.0,
                origin: [0.0, 0.0],
                marks: Vec::<SceneMark>::new()
            })
            .is_err());
    }
}

#[test]
fn exports_ready_resources_for_ordinary_and_warped_images() {
    use avenger_image::{ImageResourceResolver, ImageResourceState};
    use avenger_resource::ResourceKey;
    use avenger_scenegraph::{
        image_resources::resolve_ready_image_resources,
        marks::image::{SceneImageResource, SceneImageSource},
    };
    struct Ready;
    impl ImageResourceResolver for Ready {
        fn image_state(&self, _key: &ResourceKey) -> ImageResourceState {
            ImageResourceState::Ready(std::sync::Arc::new(scene::checker()))
        }
    }
    let mut graph = scene::gallery();
    let reference = SceneImageSource::Resource(SceneImageResource {
        key: ResourceKey::new("gallery/checker"),
        intrinsic_width: 8,
        intrinsic_height: 8,
        fallback_key: None,
    });
    for mark in &mut graph.marks {
        match mark {
            SceneMark::Image(mark) => {
                std::sync::Arc::make_mut(mark).image = ScalarOrArray::new_scalar(reference.clone())
            }
            SceneMark::WarpedImage(mark) => {
                std::sync::Arc::make_mut(mark).image = reference.clone()
            }
            _ => {}
        }
    }
    assert!(renderer().render_scene_graph(&graph).is_err());
    let resolved = resolve_ready_image_resources(&graph, &Ready).unwrap();
    assert_eq!(
        renderer().render_scene_graph(&resolved).unwrap(),
        renderer().render_scene_graph(&scene::gallery()).unwrap()
    );
}
