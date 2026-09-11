#[path = "support/pdf_raster.rs"]
mod pdf_raster;
#[path = "../../tests/render_fixtures/raster.rs"]
mod raster;
#[path = "../../tests/render_fixtures/scene.rs"]
mod scene;

use avenger_common::value::ScalarOrArray;
use avenger_pdf::{PdfBackground, PdfRenderOptions, PdfRenderer};
use avenger_scenegraph::{
    marks::{
        group::{Clip, SceneGroup},
        image::{SceneImageMark, SceneImageSource},
        mark::SceneMark,
    },
    scene_graph::SceneGraph,
};

fn renderer() -> PdfRenderer {
    PdfRenderer::new().with_options(PdfRenderOptions {
        font_resolution: scene::fonts(),
        background: PdfBackground::Transparent,
        ..Default::default()
    })
}

#[test]
fn preserves_small_page_dimensions_and_rejects_invalid_sizes() {
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
    let pdf = renderer()
        .render_scene_graph(&SceneGraph {
            width: 1.0,
            height: 2.0,
            origin: [0.0, 0.0],
            marks: Vec::new(),
        })
        .unwrap();
    let text = String::from_utf8_lossy(&pdf);
    assert!(text.contains("/MediaBox[0 0 1 2]"), "{text}");
}

#[test]
fn supplied_engine_takes_precedence_over_renderer_font_options() {
    let engine = avenger_text::TextEngine::with_font_resolution(&scene::fonts()).unwrap();
    let mut mark = scene::text("*Radius* $sqrt(x^2+y^2)$", 10.0, 35.0, 20.0);
    mark.limit = ScalarOrArray::new_scalar(60.0);
    let graph = SceneGraph {
        width: 200.0,
        height: 70.0,
        origin: [0.0, 0.0],
        marks: vec![mark.into()],
    };
    let pdf = PdfRenderer::new()
        .with_options(PdfRenderOptions {
            font_resolution: avenger_text::FontResolutionOptions {
                load_system_fonts: false,
                ..Default::default()
            },
            ..Default::default()
        })
        .with_text_engine(engine)
        .render_scene_graph(&graph)
        .unwrap();
    let text = pdf_extract::extract_text_from_mem(&pdf).unwrap();
    assert!(text.contains("Radius"));
    assert!(
        (text.contains('x') || text.contains('𝑥')) && (text.contains('y') || text.contains('𝑦')),
        "{text:?}"
    );
}

#[test]
fn unicode_does_not_enable_system_fonts() {
    let mut mark = scene::text("Label é", 10.0, 35.0, 20.0);
    mark.font = ScalarOrArray::new_scalar("Arial".into());
    let graph = SceneGraph {
        width: 200.0,
        height: 70.0,
        origin: [0.0, 0.0],
        marks: vec![mark.into()],
    };
    let error = renderer()
        .render_scene_graph(&graph)
        .unwrap_err()
        .to_string();
    assert!(error.contains("Arial"));
}

#[test]
fn rejects_malformed_inline_images() {
    let graph = image_scene(vec![avenger_image::RgbaImage {
        width: 2,
        height: 2,
        data: vec![0; 3],
    }]);
    assert!(renderer()
        .render_scene_graph(&graph)
        .unwrap_err()
        .to_string()
        .contains("invalid RGBA"));
}

#[test]
#[ignore = "requires PDFium 7763; see avenger-pdf/README.md"]
fn rectangular_gradients_match_svg_coordinates() {
    use avenger_color::{ColorOrGradient, Gradient, GradientStop, LinearGradient, RadialGradient};
    use avenger_scenegraph::marks::rect::SceneRectMark;
    let stops = vec![
        GradientStop {
            offset: 0.0,
            color: [1.0, 0.0, 0.0, 1.0],
        },
        GradientStop {
            offset: 1.0,
            color: [0.0, 0.0, 1.0, 0.5],
        },
    ];
    for gradient in [
        Gradient::LinearGradient(LinearGradient {
            x0: 0.0,
            y0: 0.0,
            x1: 1.0,
            y1: 1.0,
            stops: stops.clone(),
        }),
        Gradient::RadialGradient(RadialGradient {
            x0: 0.5,
            y0: 0.5,
            x1: 0.5,
            y1: 0.5,
            r0: 0.0,
            r1: 0.5,
            stops,
        }),
    ] {
        for (width, height) in [(200.0, 60.0), (60.0, 200.0)] {
            let graph = SceneGraph {
                width: width + 20.0,
                height: height + 20.0,
                origin: [0.0, 0.0],
                marks: vec![SceneRectMark {
                    x: ScalarOrArray::new_scalar(10.0),
                    y: ScalarOrArray::new_scalar(10.0),
                    width: Some(ScalarOrArray::new_scalar(width)),
                    height: Some(ScalarOrArray::new_scalar(height)),
                    fill: ScalarOrArray::new_scalar(ColorOrGradient::GradientIndex(0)),
                    gradients: vec![gradient.clone()],
                    ..Default::default()
                }
                .into()],
            };
            let pdf = renderer().render_scene_graph(&graph).unwrap();
            let actual = pdf_raster::pdf_to_png(&pdf, graph.width, graph.height);
            let svg = avenger_svg::SvgRenderer::new()
                .render_scene_graph(&graph)
                .unwrap();
            let expected = raster::svg_to_png(&svg, 2.0);
            for y in (24..((height + 8.0) * 2.0) as u32).step_by(13) {
                for x in (24..((width + 8.0) * 2.0) as u32).step_by(13) {
                    let a = actual.get_pixel(x, y);
                    let b = expected.get_pixel(x, y);
                    for channel in 0..3 {
                        assert!(
                            a[channel].abs_diff(b[channel]) <= 4,
                            "{gradient:?}, {width}x{height}, ({x},{y}): {a:?} != {b:?}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn zero_area_gradient_geometry_does_not_panic() {
    use avenger_color::{ColorOrGradient, Gradient, GradientStop, LinearGradient};
    use avenger_scenegraph::marks::rule::SceneRuleMark;
    let graph = SceneGraph {
        width: 100.0,
        height: 100.0,
        origin: [0.0, 0.0],
        marks: vec![SceneRuleMark {
            x: ScalarOrArray::new_scalar(10.0),
            x2: ScalarOrArray::new_scalar(90.0),
            y: ScalarOrArray::new_scalar(50.0),
            y2: ScalarOrArray::new_scalar(50.0),
            stroke_width: ScalarOrArray::new_scalar(3.0),
            stroke: ScalarOrArray::new_scalar(ColorOrGradient::GradientIndex(0)),
            gradients: vec![Gradient::LinearGradient(LinearGradient {
                x0: 0.0,
                y0: 0.0,
                x1: 1.0,
                y1: 0.0,
                stops: vec![
                    GradientStop {
                        offset: 0.0,
                        color: [1.0, 0.0, 0.0, 1.0],
                    },
                    GradientStop {
                        offset: 1.0,
                        color: [0.0, 0.0, 1.0, 1.0],
                    },
                ],
            })],
            ..Default::default()
        }
        .into()],
    };
    assert!(renderer()
        .render_scene_graph(&graph)
        .unwrap()
        .starts_with(b"%PDF-"));
}

#[test]
#[ignore = "requires PDFium 7763; see avenger-pdf/README.md"]
fn pattern_operations_apply_ink_opacity_once() {
    use avenger_color::ColorOrGradient;
    use avenger_scenegraph::marks::{
        pattern::{
            PatternAnchor, PatternFill, PatternInk, PatternLayer, PatternLayerOperation,
            StripePatternLayer,
        },
        rect::SceneRectMark,
    };
    for operation in [
        PatternLayerOperation::Add,
        PatternLayerOperation::Subtract,
        PatternLayerOperation::Xor,
    ] {
        let first = StripePatternLayer::new(0.0, 20.0, 8.0);
        let mut second = StripePatternLayer::new(90.0, 20.0, 8.0);
        second.operation = operation;
        let graph = SceneGraph {
            width: 100.0,
            height: 100.0,
            origin: [0.0, 0.0],
            marks: vec![SceneRectMark {
                width: Some(ScalarOrArray::new_scalar(100.0)),
                height: Some(ScalarOrArray::new_scalar(100.0)),
                fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([1.0; 4])),
                fill_pattern: ScalarOrArray::new_scalar(Some(PatternFill {
                    anchor: PatternAnchor::Mark,
                    ink: PatternInk::Solid {
                        color: [0.0, 0.0, 0.0, 1.0],
                        opacity: 0.5,
                    },
                    layers: vec![PatternLayer::Stripe(first), PatternLayer::Stripe(second)],
                })),
                ..Default::default()
            }
            .into()],
        };
        let actual = pdf_raster::pdf_to_png(
            &renderer().render_scene_graph(&graph).unwrap(),
            100.0,
            100.0,
        );
        let expected = raster::svg_to_png(
            &avenger_svg::SvgRenderer::new()
                .render_scene_graph(&graph)
                .unwrap(),
            2.0,
        );
        let mut ink_pixels = 0;
        for y in (2..196).step_by(10) {
            for x in (2..196).step_by(10) {
                let a = actual.get_pixel(x, y)[0];
                let b = expected.get_pixel(x, y)[0];
                assert!(
                    a >= 125,
                    "{operation:?}: ink opacity accumulated at {x},{y}: {a}"
                );
                assert!(
                    a.abs_diff(b) <= 2,
                    "{operation:?}: coverage differs at {x},{y}: {a} != {b}"
                );
                ink_pixels += usize::from(a < 200);
            }
        }
        assert!(ink_pixels > 20);
    }
}

#[test]
#[ignore = "requires PDFium 7763; see avenger-pdf/README.md"]
fn gallery_visual_regression_and_svg_comparison() {
    let scene = scene::gallery();
    let pdf = renderer()
        .with_options(PdfRenderOptions {
            font_resolution: scene::fonts(),
            ..Default::default()
        })
        .render_scene_graph(&scene)
        .unwrap();
    let png = pdf_raster::pdf_to_png(&pdf, scene.width, scene.height);
    raster::assert_baseline(
        &png,
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/baselines/gallery.png"),
    );
    let svg = avenger_svg::SvgRenderer::new()
        .with_options(avenger_svg::SvgRenderOptions {
            font_resolution: scene::fonts(),
            ..Default::default()
        })
        .render_scene_graph(&scene)
        .unwrap();
    let reference = raster::svg_to_png(&svg, 2.0);
    let mean = png
        .as_raw()
        .iter()
        .zip(reference.as_raw())
        .map(|(a, b)| a.abs_diff(*b) as f64)
        .sum::<f64>()
        / png.as_raw().len() as f64;
    assert!(mean < 3.0, "PDF / SVG mean channel difference: {mean}");
    let extracted = pdf_extract::extract_text_from_mem(&pdf).unwrap();
    assert!(
        extracted.contains("Scene graph to vector document"),
        "{extracted:?}"
    );
    assert!(extracted.contains("Italic"));
}

#[test]
#[ignore = "requires PDFium 7763; see avenger-pdf/README.md"]
fn script_and_smallcaps_glyphs_match_svg() {
    for source in [
        "H#sub[2]O",
        "H#super[2]O",
        "H#sub(typographic: false)[2]O",
        "H#super(typographic: false)[2]O",
        "#smallcaps[Smallcaps]",
        "H#smallcaps(all: true)[CAPS]O",
    ] {
        let graph = SceneGraph {
            width: 280.0,
            height: 70.0,
            origin: [0.0, 0.0],
            marks: vec![scene::text(source, 10.0, 45.0, 40.0).into()],
        };
        let pdf = renderer().render_scene_graph(&graph).unwrap();
        let actual = pdf_raster::pdf_to_png(&pdf, graph.width, graph.height);
        let svg = avenger_svg::SvgRenderer::new()
            .with_options(avenger_svg::SvgRenderOptions {
                font_resolution: scene::fonts(),
                ..Default::default()
            })
            .render_scene_graph(&graph)
            .unwrap();
        let expected = raster::svg_to_png(&svg, 2.0);
        let mut error = 0u64;
        let mut ink_pixels = 0u64;
        for (a, b) in actual.pixels().zip(expected.pixels()) {
            // Exclude blank space so a misplaced script cannot hide in the page average.
            if a[0] < 240 || b[0] < 240 {
                ink_pixels += 1;
                error += (0..3)
                    .map(|channel| a[channel].abs_diff(b[channel]) as u64)
                    .sum::<u64>();
            }
        }
        assert!(ink_pixels > 100);
        let mean = error as f64 / (ink_pixels * 3) as f64;
        assert!(mean < 12.0, "{source}: mean ink difference {mean}");
    }
}

#[test]
#[ignore = "requires PDFium 7763; see avenger-pdf/README.md"]
fn rotated_markup_clips_before_placement_and_preserves_semantic_text() {
    for angle in [0.0f32, 28.0, -28.0] {
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
        let pdf = renderer().render_scene_graph(&scene).unwrap();
        assert!(pdf_extract::extract_text_from_mem(&pdf)
            .unwrap()
            .contains("tail"));
        let png = pdf_raster::pdf_to_png(&pdf, scene.width, scene.height);
        let mut visible = 0;
        for (x, y, pixel) in png.enumerate_pixels() {
            if pixel[0] > 245 && pixel[1] > 245 && pixel[2] > 245 {
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

fn image_scene(images: Vec<avenger_image::RgbaImage>) -> SceneGraph {
    SceneGraph {
        width: 240.0,
        height: 120.0,
        origin: [0.0, 0.0],
        marks: images
            .into_iter()
            .enumerate()
            .map(|(i, image)| {
                SceneImageMark {
                    image: ScalarOrArray::new_scalar(SceneImageSource::Inline(image)),
                    x: ScalarOrArray::new_scalar(10.0 + i as f32 * 110.0),
                    y: ScalarOrArray::new_scalar(10.0),
                    width: ScalarOrArray::new_scalar(100.0),
                    height: ScalarOrArray::new_scalar(100.0),
                    aspect: false,
                    smooth: false,
                    ..Default::default()
                }
                .into()
            })
            .collect(),
    }
}

#[test]
#[ignore = "requires PDFium 7763; see avenger-pdf/README.md"]
fn image_identity_includes_dimensions_and_smoothing() {
    let data = vec![255, 0, 0, 255, 0, 0, 255, 255];
    let mut graph = image_scene(vec![
        avenger_image::RgbaImage {
            width: 2,
            height: 1,
            data: data.clone(),
        },
        avenger_image::RgbaImage {
            width: 1,
            height: 2,
            data,
        },
    ]);
    let pdf = renderer().render_scene_graph(&graph).unwrap();
    let png = pdf_raster::pdf_to_png(&pdf, graph.width, graph.height);
    assert_eq!(png.get_pixel(170, 50).0[..3], [0, 0, 255]);
    assert_eq!(png.get_pixel(280, 50).0[..3], [255, 0, 0]);
    assert_eq!(png.get_pixel(280, 170).0[..3], [0, 0, 255]);
    let SceneMark::Image(mark) = &mut graph.marks[0] else {
        panic!()
    };
    std::sync::Arc::make_mut(mark).smooth = true;
    let pdf = renderer().render_scene_graph(&graph).unwrap();
    assert!(String::from_utf8_lossy(&pdf).contains("/Interpolate true"));
    let smooth = pdf_raster::pdf_to_png(&pdf, graph.width, graph.height);
    assert_ne!(png, smooth);
}

#[test]
fn styled_text_extracts_semantic_characters_without_markup() {
    for (source, expected) in [
        ("*Bold label*", "Bold label"),
        ("_Italic label_", "Italic label"),
        ("#upper[caption]", "CAPTION"),
        ("#underline[Decorated]", "Decorated"),
        ("#smallcaps[Smallcaps]", "Smallcaps"),
        ("#smallcaps(all: true)[CAPS]", "CAPS"),
    ] {
        let graph = SceneGraph {
            width: 240.0,
            height: 70.0,
            origin: [0.0, 0.0],
            marks: vec![scene::text(source, 10.0, 30.0, 20.0).into()],
        };
        let pdf = renderer().render_scene_graph(&graph).unwrap();
        let text = pdf_extract::extract_text_from_mem(&pdf).unwrap();
        assert_eq!(text.trim(), expected, "{source}");
    }
}

#[test]
fn script_glyphs_preserve_semantic_characters() {
    for source in ["H#sub[2]O", "H#super[2]O"] {
        let graph = SceneGraph {
            width: 120.0,
            height: 70.0,
            origin: [0.0, 0.0],
            marks: vec![scene::text(source, 10.0, 45.0, 40.0).into()],
        };
        let pdf = renderer().render_scene_graph(&graph).unwrap();
        let text = pdf_extract::extract_text_from_mem(&pdf).unwrap();
        // The extractor inserts whitespace between separately positioned text runs.
        let characters: String = text.chars().filter(|ch| !ch.is_whitespace()).collect();
        assert_eq!(characters, "H2O", "{source}: {text:?}");
    }
}

#[test]
#[ignore = "requires PDFium 7763; see avenger-pdf/README.md"]
fn empty_group_clip_hides_its_descendants() {
    let graph = SceneGraph {
        width: 100.0,
        height: 60.0,
        origin: [0.0, 0.0],
        marks: vec![SceneGroup {
            clip: Clip::Rect {
                x: 0.0,
                y: 0.0,
                width: 0.0,
                height: 40.0,
            },
            marks: vec![{
                let mut text = scene::text("Hidden", 0.0, 25.0, 20.0);
                text.clip = true;
                text.into()
            }],
            ..Default::default()
        }
        .into()],
    };
    let pdf = renderer().render_scene_graph(&graph).unwrap();
    let png = pdf_raster::pdf_to_png(&pdf, graph.width, graph.height);
    assert!(png.pixels().all(|p| p.0[..3] == [255, 255, 255]));
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

#[test]
#[ignore = "requires PDFium 7763; see avenger-pdf/README.md"]
fn audited_typst_layout_matches_svg_with_variable_fonts_and_decorations() {
    let mut fonts = scene::fonts();
    for (id, data) in [
        (
            3001,
            include_bytes!("fonts/NotoSansHebrew/NotoSansHebrew.ttf").as_slice(),
        ),
        (
            3002,
            include_bytes!("fonts/NotoSansDevanagari/NotoSansDevanagari.ttf").as_slice(),
        ),
    ] {
        fonts
            .registered_fonts
            .push(avenger_text::RegisteredFont::new(
                avenger_text::MathFontBytesId(id),
                std::sync::Arc::<[u8]>::from(data),
            ));
    }
    for source in [
        "אבג #strong[דהו] אבג",
        "abc #underline[हिन्दी] xyz",
        "abc #underline[אבג 123] xyz",
        "$\"हिन्दी\" \"אבג\"$",
        "$sqrt(frac(1,x^2))_n^m$",
        "#underline[a #strike[b] c]",
        "#underline(evade: false, offset: -10pt, stroke: 3pt + red, background: true)[abc]",
        "#underline(evade: false, offset: -10pt, stroke: 3pt + red, background: false)[abc]",
    ] {
        let graph = SceneGraph {
            width: 500.0,
            height: 180.0,
            origin: [0.0, 0.0],
            marks: vec![scene::text(source, 20.0, 90.0, 32.0).into()],
        };
        let pdf = PdfRenderer::new()
            .with_options(PdfRenderOptions {
                font_resolution: fonts.clone(),
                ..Default::default()
            })
            .render_scene_graph(&graph)
            .unwrap();
        let svg = avenger_svg::SvgRenderer::new()
            .with_options(avenger_svg::SvgRenderOptions {
                font_resolution: fonts.clone(),
                ..Default::default()
            })
            .render_scene_graph(&graph)
            .unwrap();
        let actual = pdf_raster::pdf_to_png(&pdf, graph.width, graph.height);
        let expected = raster::svg_to_png(&svg, 2.0);
        let mut error = 0u64;
        let mut ink = 0u64;
        for (a, b) in actual.pixels().zip(expected.pixels()) {
            if a[0] < 240 || b[0] < 240 {
                ink += 1;
                error += (0..3).map(|c| a[c].abs_diff(b[c]) as u64).sum::<u64>();
            }
        }
        assert!(ink > 100, "{source}");
        let mean = error as f64 / (ink * 3) as f64;
        assert!(mean < 12.0, "{source}: mean ink difference {mean}");
    }
}
