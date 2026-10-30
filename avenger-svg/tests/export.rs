use avenger_color::ColorOrGradient;
use avenger_common::{
    types::{AreaOrientation, PathTransform},
    value::ScalarOrArray,
};
use avenger_image::RgbaImage;
use avenger_render_fixtures::{raster, scene};
use avenger_scenegraph::{
    marks::{
        arc::SceneArcMark,
        area::SceneAreaMark,
        group::{Clip, SceneGroup},
        image::SceneImageMark,
        line::SceneLineMark,
        mark::SceneMark,
        path::ScenePathMark,
        rect::SceneRectMark,
        rule::SceneRuleMark,
        symbol::SceneSymbolMark,
        text::SceneTextMark,
        trail::SceneTrailMark,
    },
    scene_graph::SceneGraph,
};
use avenger_svg::{SvgRenderOptions, SvgRenderer};
use avenger_typst_label::{EngineOptions, FontOptions, LabelEngine, MissingFontPolicy};
use resvg::usvg;

fn engine() -> LabelEngine {
    LabelEngine::new(EngineOptions {
        fonts: scene::fonts(),
    })
}

fn renderer() -> SvgRenderer {
    SvgRenderer::new().with_options(SvgRenderOptions {
        background: None,
        ..Default::default()
    })
}

/// A transparent document whose text names its fonts instead of embedding them.
fn render_without_fonts(scene_graph: &SceneGraph) -> String {
    renderer()
        .with_options(SvgRenderOptions {
            background: None,
            embed_fonts: false,
            ..Default::default()
        })
        .render_scene_graph(scene_graph, &engine())
        .unwrap()
}

#[test]
fn gallery_visual_regression() {
    let svg = renderer()
        .with_options(SvgRenderOptions::default())
        .render_scene_graph(&scene::gallery(), &engine())
        .unwrap();
    let png = raster::svg_to_png(&svg, 2.0);
    raster::assert_baseline(env!("CARGO_MANIFEST_DIR"), &png, "gallery");
}

#[test]
fn contrast_labels_visual_regression() {
    let svg = renderer()
        .with_options(SvgRenderOptions::default())
        .render_scene_graph(&scene::contrast_labels(), &engine())
        .unwrap();
    let png = raster::svg_to_png(&svg, 2.0);
    raster::assert_baseline(env!("CARGO_MANIFEST_DIR"), &png, "contrast-labels");
}

/// Text along a curve, after Konva's TextPath demo.
#[test]
fn konva_text_path_visual_regression() {
    let svg = renderer()
        .with_options(SvgRenderOptions::default())
        .render_scene_graph(&scene::konva_text_path(), &engine())
        .unwrap();
    let png = raster::svg_to_png(&svg, 2.0);
    raster::assert_baseline(env!("CARGO_MANIFEST_DIR"), &png, "konva-text-path");
}

/// Text along a path stays text: a `textPath` along a defined path, which viewers select.
#[test]
fn text_along_a_path_stays_text() {
    let svg = renderer()
        .render_scene_graph(&scene::konva_text_path(), &engine())
        .unwrap();
    assert!(svg.contains(r#"<path id="svg-text-path-0" d="M"#), "{svg}");
    let text = "All the world's a stage, and all the men and women merely players.";
    let run = format!(r##"<textPath href="#svg-text-path-0" startOffset="1">{text}</textPath>"##);
    assert!(svg.contains(&run), "{svg}");
}

#[test]
fn typographic_script_glyphs_do_not_touch_adjacent_letters() {
    for source in ["H#sub[2]O", "H#super[2]O"] {
        let graph = SceneGraph {
            width: 120.0,
            height: 70.0,
            origin: [0.0, 0.0],
            marks: vec![scene::text(source, 10.0, 45.0, 40.0).into()],
        };
        let svg = renderer().render_scene_graph(&graph, &engine()).unwrap();
        let png = raster::svg_to_png(&svg, 2.0);
        let mut intervals = Vec::new();
        let mut previous_visible = false;
        for x in 0..png.width() {
            let visible = (0..png.height()).any(|y| png.get_pixel(x, y)[3] > 128);
            if visible && !previous_visible {
                intervals.push(x..x + 1);
            } else if visible {
                intervals.last_mut().unwrap().end = x + 1;
            }
            previous_visible = visible;
        }
        assert_eq!(intervals.len(), 3, "{source}: glyphs touch: {intervals:?}");
        assert!(
            intervals[1].end + 1 < intervals[2].start,
            "{source}: {intervals:?}"
        );
    }
}

#[test]
fn cuts_markup_to_its_width_and_clips_the_scene_outside_rotation() {
    for angle in [0.0, 28.0, -28.0] {
        let mut text = scene::text("*Bold* $sqrt(x^2+y^2)$ tail", 10.0, 40.0, 22.0);
        text.width = ScalarOrArray::new_scalar(avenger_typst_label::LabelWidth::Max(70.0));
        text.wrap = false;
        text.ellipsis = true;
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
        let svg = renderer().render_scene_graph(&scene, &engine()).unwrap();
        assert!(
            svg.contains("Bold"),
            "{}",
            svg.split("</defs>").last().unwrap()
        );
        assert!(svg.contains(">…</text>"));
        assert!(!svg.contains("tail"));
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
            assert!(local_x <= 71.0, "{angle}: overflow at {x}, {y}");
        }
        assert!(visible > 100);
    }
}

#[test]
fn uses_supplied_engine_fonts_and_text_colors() {
    let mut options = scene::fonts();
    options.default_sans_serif_family = Some("DejaVu Sans Mono".into());
    let engine = LabelEngine::new(EngineOptions { fonts: options });
    let mut mark = scene::text("R", 10.0, 30.0, 20.0);
    mark.font = ScalarOrArray::new_scalar("sans-serif".into());
    mark.color = ScalarOrArray::new_scalar([1.0, 0.0, 0.0, 1.0]);
    let mut blue = mark.clone();
    blue.text = ScalarOrArray::new_scalar("B".into());
    blue.x = ScalarOrArray::new_scalar(35.0);
    blue.color = ScalarOrArray::new_scalar([0.0, 0.0, 1.0, 1.0]);
    let scene = SceneGraph {
        width: 100.0,
        height: 50.0,
        origin: [0.0, 0.0],
        marks: vec![mark.into(), blue.into()],
    };
    let svg = renderer().render_scene_graph(&scene, &engine).unwrap();
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
fn faces_whose_license_forbids_embedding_name_their_family() {
    let mut fonts = scene::fonts();
    for font in &mut fonts.registered_fonts {
        font.data = scene::with_embedding_flags(&font.data, 0x0002).into();
    }
    let engine = LabelEngine::new(EngineOptions { fonts });
    let scene = SceneGraph {
        width: 100.0,
        height: 50.0,
        origin: [0.0, 0.0],
        marks: vec![scene::text("Label", 10.0, 30.0, 12.0).into()],
    };
    let svg = renderer().render_scene_graph(&scene, &engine).unwrap();
    assert!(!svg.contains("@font-face"));
    assert!(svg.contains(r#"font-family="Lato""#), "{svg}");
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
    for embed_fonts in [false, true] {
        let engine = LabelEngine::new(EngineOptions {
            fonts: FontOptions {
                missing_font: MissingFontPolicy::Error,
                ..scene::fonts()
            },
        });
        let result = renderer()
            .with_options(SvgRenderOptions {
                embed_fonts,
                ..Default::default()
            })
            .render_scene_graph(&scene, &engine);
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
            .render_scene_graph(
                &SceneGraph {
                    width,
                    height: 50.0,
                    origin: [0.0, 0.0],
                    marks: Vec::<SceneMark>::new()
                },
                &engine()
            )
            .is_err());
    }
}

#[test]
fn pattern_symbols_union_instances_before_operations() {
    use avenger_color::ColorOrGradient;
    use avenger_common::pattern::{
        PatternAnchor, PatternFill, PatternInk, PatternLayer, PatternLayerOperation, PatternSymbol,
        PatternSymbolShape, StripeLayer, SymbolLattice2d, SymbolLayer, SymbolPaint,
    };
    use avenger_scenegraph::marks::rect::SceneRectMark;

    // 40 px squares on lattices close enough for neighbors to overlap.
    for (paint, spacing, ink_pixel, gap_pixel) in [
        (SymbolPaint::Filled, 30.0, [65, 50], [50, 20]),
        (
            SymbolPaint::Open { stroke_width: 8.0 },
            38.0,
            [69, 50],
            [50, 50],
        ),
    ] {
        for operation in [
            PatternLayerOperation::Add,
            PatternLayerOperation::Subtract,
            PatternLayerOperation::Xor,
        ] {
            let mut layers = Vec::new();
            if operation != PatternLayerOperation::Add {
                layers.push(StripeLayer::new(0.0, 1000.0, 1000.0).into());
            }
            let symbols = SymbolLayer {
                lattice: SymbolLattice2d {
                    u_spacing: spacing,
                    v_spacing: 100.0,
                    u_angle: 0.0,
                    v_angle: 90.0,
                    u_phase: 50.0,
                    v_phase: 50.0,
                },
                symbol: PatternSymbol {
                    shape: PatternSymbolShape::Square,
                    size: 1600.0,
                    rotation: 0.0,
                },
                paint: paint.clone(),
            };
            layers.push(PatternLayer::from(symbols).with_operation(operation));
            let mark = SceneRectMark {
                width: Some(100.0.into()),
                height: Some(100.0.into()),
                fill: ColorOrGradient::Color([1.0; 4]).into(),
                fill_pattern: Some(PatternFill {
                    anchor: PatternAnchor::Mark,
                    ink: PatternInk::Solid {
                        color: [0.0, 0.0, 0.0, 1.0],
                        opacity: 0.5,
                    },
                    layers,
                })
                .into(),
                ..Default::default()
            };
            let graph = SceneGraph {
                width: 100.0,
                height: 100.0,
                origin: [0.0; 2],
                marks: vec![mark.into()],
            };
            let image = raster::svg_to_png(
                &SvgRenderer::new()
                    .render_scene_graph(&graph, &engine())
                    .unwrap(),
                1.0,
            );
            for (pixel, covered) in [(ink_pixel, true), (gap_pixel, false)] {
                let covered = covered == (operation == PatternLayerOperation::Add);
                let value = image.get_pixel(pixel[0], pixel[1]).0[0];
                let expected = if covered { 127 } else { 255 };
                assert!(value.abs_diff(expected) <= 1,
                            "{operation:?}, {paint:?}, spacing={spacing}, pixel={pixel:?}: {value} != {expected}");
            }
        }
    }
}

#[test]
fn empty_item_sets_emit_no_drawable_svg_elements() {
    let scene_graph = SceneGraph {
        width: 40.0,
        height: 20.0,
        origin: [0.0, 0.0],
        marks: vec![
            SceneArcMark {
                len: 0,
                ..Default::default()
            }
            .into(),
            SceneAreaMark {
                len: 0,
                ..Default::default()
            }
            .into(),
            ScenePathMark {
                len: 0,
                ..Default::default()
            }
            .into(),
            SceneSymbolMark {
                len: 0,
                ..Default::default()
            }
            .into(),
            SceneLineMark {
                len: 0,
                ..Default::default()
            }
            .into(),
            SceneTrailMark {
                len: 0,
                ..Default::default()
            }
            .into(),
            SceneRectMark {
                len: 0,
                ..Default::default()
            }
            .into(),
            SceneRuleMark {
                len: 0,
                ..Default::default()
            }
            .into(),
            SceneTextMark {
                len: 0,
                ..Default::default()
            }
            .into(),
            SceneImageMark {
                len: 0,
                ..Default::default()
            }
            .into(),
            SceneGroup::default().into(),
        ],
    };

    let svg = render_without_fonts(&scene_graph);

    assert!(usvg::Tree::from_str(&svg, &usvg::Options::default()).is_ok());
    assert!(!svg.contains("<path "));
    assert!(!svg.contains("<text "));
    assert!(!svg.contains("<image "));
    assert_eq!(svg.matches("<rect ").count(), 0);
}

#[test]
fn renders_one_snapshot_covering_every_scene_mark_type() {
    let image = RgbaImage {
        width: 1,
        height: 1,
        data: vec![255, 0, 0, 255],
    };
    let scene_graph = SceneGraph {
        width: 120.0,
        height: 90.0,
        origin: [0.0, 0.0],
        marks: vec![
            SceneGroup {
                origin: [2.0, 3.0],
                clip: avenger_scenegraph::marks::group::Clip::Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 108.0,
                    height: 78.0,
                },
                fill: Some(ColorOrGradient::Color([0.0, 0.0, 1.0, 0.2])),
                stroke: Some(ColorOrGradient::Color([0.0, 0.0, 1.0, 1.0])),
                stroke_width: Some(1.0),
                marks: vec![SceneRectMark {
                    len: 1,
                    x: ScalarOrArray::new_scalar(4.0),
                    y: ScalarOrArray::new_scalar(5.0),
                    width: Some(ScalarOrArray::new_scalar(12.0)),
                    height: Some(ScalarOrArray::new_scalar(8.0)),
                    fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([1.0, 0.0, 0.0, 1.0])),
                    ..Default::default()
                }
                .into()],
                ..Default::default()
            }
            .into(),
            SceneArcMark {
                x: ScalarOrArray::new_scalar(30.0),
                y: ScalarOrArray::new_scalar(15.0),
                start_angle: ScalarOrArray::new_scalar(0.0),
                end_angle: ScalarOrArray::new_scalar(std::f32::consts::PI),
                inner_radius: ScalarOrArray::new_scalar(3.0),
                outer_radius: ScalarOrArray::new_scalar(8.0),
                fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 1.0, 0.0, 1.0])),
                ..Default::default()
            }
            .into(),
            SceneAreaMark {
                len: 3,
                orientation: AreaOrientation::Vertical,
                x: ScalarOrArray::new_array(vec![10.0, 20.0, 30.0]),
                y: ScalarOrArray::new_array(vec![44.0, 38.0, 42.0]),
                y2: ScalarOrArray::new_array(vec![52.0, 52.0, 52.0]),
                fill: ColorOrGradient::Color([1.0, 0.0, 1.0, 0.5]),
                ..Default::default()
            }
            .into(),
            ScenePathMark {
                path: ScalarOrArray::new_scalar({
                    let mut builder = lyon_path::Path::builder();
                    builder.begin(lyon_path::math::point(0.0, 0.0));
                    builder.line_to(lyon_path::math::point(8.0, 0.0));
                    builder.line_to(lyon_path::math::point(4.0, 8.0));
                    builder.close();
                    builder.build()
                }),
                fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 1.0, 1.0, 0.5])),
                stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0])),
                stroke_width: Some(1.0),
                transform: ScalarOrArray::new_scalar(PathTransform::translation(40.0, 35.0)),
                ..Default::default()
            }
            .into(),
            SceneSymbolMark {
                len: 1,
                x: ScalarOrArray::new_scalar(72.0),
                y: ScalarOrArray::new_scalar(18.0),
                size: ScalarOrArray::new_scalar(36.0),
                fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.5, 0.0, 1.0, 1.0])),
                stroke_width: Some(1.0),
                ..Default::default()
            }
            .into(),
            SceneLineMark {
                len: 3,
                x: ScalarOrArray::new_array(vec![60.0, 72.0, 84.0]),
                y: ScalarOrArray::new_array(vec![40.0, 45.0, 36.0]),
                stroke: ColorOrGradient::Color([1.0, 0.5, 0.0, 1.0]),
                ..Default::default()
            }
            .into(),
            SceneTrailMark {
                len: 3,
                x: ScalarOrArray::new_array(vec![56.0, 70.0, 88.0]),
                y: ScalarOrArray::new_array(vec![62.0, 66.0, 58.0]),
                size: ScalarOrArray::new_array(vec![4.0, 8.0, 12.0]),
                stroke: ColorOrGradient::Color([0.0, 0.0, 0.0, 0.6]),
                ..Default::default()
            }
            .into(),
            SceneRuleMark {
                x: ScalarOrArray::new_scalar(96.0),
                y: ScalarOrArray::new_scalar(12.0),
                x2: ScalarOrArray::new_scalar(108.0),
                y2: ScalarOrArray::new_scalar(24.0),
                stroke_dash: Some(ScalarOrArray::new_scalar(vec![2.0, 1.0])),
                ..Default::default()
            }
            .into(),
            SceneTextMark {
                text: ScalarOrArray::new_scalar("All marks".to_string()),
                x: ScalarOrArray::new_scalar(8.0),
                y: ScalarOrArray::new_scalar(76.0),
                ..Default::default()
            }
            .into(),
            SceneImageMark {
                len: 1,
                image: ScalarOrArray::new_scalar(image.into()),
                x: ScalarOrArray::new_scalar(96.0),
                y: ScalarOrArray::new_scalar(2.0),
                width: ScalarOrArray::new_scalar(10.0),
                height: ScalarOrArray::new_scalar(8.0),
                ..Default::default()
            }
            .into(),
        ],
    };

    let svg = render_without_fonts(&scene_graph);

    assert!(usvg::Tree::from_str(&svg, &usvg::Options::default()).is_ok());
    assert!(svg.matches("<path ").count() >= 8);
    assert_eq!(svg.matches("<text ").count(), 1);
    assert_eq!(svg.matches("<image ").count(), 1);
    assert!(svg.contains(r##"fill="#0000ff" fill-opacity="0.2""##));
    assert!(svg.contains(r##"fill="#ff0000""##));
    assert!(svg.contains(r##"fill="#00ff00""##));
    assert!(svg.contains(r##"fill="#ff00ff" fill-opacity="0.5""##));
    assert!(svg.contains(r##"fill="#00ffff" fill-opacity="0.5""##));
    assert!(svg.contains(r#"stroke-dasharray="2 1""#));
    assert!(svg.contains(">All marks</text>"));
}

/// An image resource that isn't ready fails the export, which names the URLs that aren't ready.
/// When the options allow unready images, a resource draws the image that the renderer's
/// resolver holds, or else its placeholder.
#[test]
fn unready_images_fail_the_export_unless_allowed() {
    use std::{collections::HashMap, sync::Arc};

    use avenger_render_fixtures::resources::{
        pixel, resource_scene, IMAGE_URL, MESH_URL, PLACEHOLDER,
    };
    use avenger_svg::AvengerSvgError;
    let red = [255, 0, 0, 255];
    let images: Arc<HashMap<String, Arc<RgbaImage>>> =
        Arc::new(HashMap::from([(IMAGE_URL.to_string(), pixel(red))]));
    let scene = resource_scene();
    let unready = |renderer: SvgRenderer| match renderer.render_scene_graph(&scene, &engine()) {
        Err(AvengerSvgError::ImagesNotReady(urls)) => urls,
        result => panic!("expected unready images, got {result:?}"),
    };
    assert_eq!(unready(renderer().with_images(images.clone())), [MESH_URL]);
    assert_eq!(unready(renderer()), [IMAGE_URL, MESH_URL]);

    let svg = SvgRenderer::new()
        .with_options(SvgRenderOptions {
            allow_unready_images: true,
            ..Default::default()
        })
        .with_images(images)
        .render_scene_graph(&scene, &engine())
        .unwrap();
    let png = raster::svg_to_png(&svg, 1.0);
    assert_eq!(png.get_pixel(5, 5).0, red);
    assert_eq!(png.get_pixel(15, 5).0, PLACEHOLDER);
}
