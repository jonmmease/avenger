#[path = "support/pdf_raster.rs"]
mod pdf_raster;

use std::path::Path;

use avenger_common::value::ScalarOrArray;
use avenger_pdf::{PdfRenderOptions, PdfRenderer};
use avenger_render_fixtures::{raster, scene};
use avenger_scenegraph::{
    marks::{
        group::{Clip, SceneGroup},
        image::SceneImageMark,
        mark::SceneMark,
        mesh_image::{MeshImage, SceneMeshImageMark},
    },
    scene_graph::SceneGraph,
};
use avenger_typst_label::{
    EngineOptions, FontOptions, LabelEngine, LabelWidth, MissingFontPolicy, RegisteredFont,
};

fn renderer() -> PdfRenderer {
    PdfRenderer::new().with_options(PdfRenderOptions {
        background: None,
        ..Default::default()
    })
}

/// An engine with the gallery's fonts.
fn engine() -> LabelEngine {
    LabelEngine::new(EngineOptions {
        fonts: scene::fonts(),
    })
}

/// An engine with the gallery's fonts, Hebrew and Devanagari faces, and Caveat from a font
/// directory, where a missing family is an error.
fn text_engine() -> LabelEngine {
    let mut fonts = FontOptions {
        extra_font_dirs: vec![Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../avenger-vega-test-data/fonts/Caveat/static")],
        missing_font: MissingFontPolicy::Error,
        ..scene::fonts()
    };
    for compressed in [
        include_bytes!("../../avenger-typst-label/tests/fixtures/fonts/NotoSansHebrew.ttf.br")
            .as_slice(),
        include_bytes!("../../avenger-typst-label/tests/fixtures/fonts/NotoSansDevanagari.ttf.br")
            .as_slice(),
    ] {
        let font = avenger_fonts::decompress(compressed);
        fonts.registered_fonts.push(RegisteredFont::new(font));
    }
    LabelEngine::new(EngineOptions { fonts })
}

#[test]
fn preserves_small_page_dimensions_and_rejects_invalid_sizes() {
    for width in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        assert!(renderer()
            .render_scene_graph(
                &SceneGraph {
                    width,
                    height: 50.0,
                    origin: [0.0, 0.0],
                    marks: Vec::<SceneMark>::new()
                },
                &engine(),
            )
            .is_err());
    }
    let pdf = renderer()
        .render_scene_graph(
            &SceneGraph {
                width: 1.0,
                height: 2.0,
                origin: [0.0, 0.0],
                marks: Vec::new(),
            },
            &engine(),
        )
        .unwrap();
    let text = String::from_utf8_lossy(&pdf);
    assert!(text.contains("/MediaBox[0 0 1 2]"), "{text}");
}

#[test]
fn faces_whose_license_forbids_embedding_or_subsetting_are_errors() {
    for (fs_type, action) in [(0x0002, "embedding"), (0x0100, "subsetting")] {
        let mut fonts = scene::fonts();
        for font in &mut fonts.registered_fonts {
            font.data = scene::with_embedding_flags(&font.data, fs_type).into();
        }
        let engine = LabelEngine::new(EngineOptions { fonts });
        let scene = SceneGraph {
            width: 100.0,
            height: 50.0,
            origin: [0.0, 0.0],
            marks: vec![scene::text("Label", 10.0, 30.0, 12.0).into()],
        };
        let error = renderer()
            .render_scene_graph(&scene, &engine)
            .unwrap_err()
            .to_string();
        assert!(error.contains(&format!("forbids {action} it")), "{error}");
    }
}

#[test]
fn rejects_malformed_images() {
    let graph = image_scene(vec![avenger_image::RgbaImage {
        width: 2,
        height: 2,
        data: vec![0; 3],
    }]);
    assert!(renderer()
        .render_scene_graph(&graph, &engine())
        .unwrap_err()
        .to_string()
        .contains("invalid RGBA"));
}

#[test]
#[ignore = "requires PDFium 7763; see avenger-pdf/README.md"]
fn gallery_visual_regression_and_svg_comparison() {
    let scene = scene::gallery();
    let pdf = renderer()
        .with_options(PdfRenderOptions::default())
        .render_scene_graph(&scene, &engine())
        .unwrap();
    let png = pdf_raster::pdf_to_png(&pdf, scene.width, scene.height);
    raster::assert_baseline(env!("CARGO_MANIFEST_DIR"), &png, "gallery");
    let svg = avenger_svg::SvgRenderer::new()
        .render_scene_graph(&scene, &engine())
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
fn rotated_markup_cuts_to_its_width_before_placement() {
    for angle in [0.0f32, 28.0, -28.0] {
        let mut text = scene::text("*Bold* $sqrt(x^2+y^2)$ tail", 10.0, 40.0, 22.0);
        text.width = ScalarOrArray::new_scalar(LabelWidth::Max(70.0));
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
        let pdf = renderer().render_scene_graph(&scene, &engine()).unwrap();
        let extracted = pdf_extract::extract_text_from_mem(&pdf).unwrap();
        assert!(
            extracted.contains('…') && !extracted.contains("tail"),
            "{extracted:?}"
        );
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
            assert!(local_x <= 71.0, "{angle}: overflow at {x}, {y}");
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
                    image: ScalarOrArray::new_scalar(image.into()),
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
    let pdf = renderer().render_scene_graph(&graph, &engine()).unwrap();
    let png = pdf_raster::pdf_to_png(&pdf, graph.width, graph.height);
    assert_eq!(png.get_pixel(170, 50).0[..3], [0, 0, 255]);
    assert_eq!(png.get_pixel(280, 50).0[..3], [255, 0, 0]);
    assert_eq!(png.get_pixel(280, 170).0[..3], [0, 0, 255]);
    let SceneMark::Image(mark) = &mut graph.marks[0] else {
        panic!()
    };
    std::sync::Arc::make_mut(mark).smooth = true;
    let pdf = renderer().render_scene_graph(&graph, &engine()).unwrap();
    assert!(String::from_utf8_lossy(&pdf).contains("/Interpolate true"));
    let smooth = pdf_raster::pdf_to_png(&pdf, graph.width, graph.height);
    assert_ne!(png, smooth);
}

#[test]
fn text_extracts_as_the_characters_it_shows() {
    for (source, font, parts) in [
        ("Hello PDF", "Lato", &["HelloPDF"][..]),
        ("*Bold label*", "Lato", &["Boldlabel"]),
        ("#upper[caption]", "Lato", &["CAPTION"]),
        ("#smallcaps[Smallcaps]", "Lato", &["Smallcaps"]),
        ("H#sub[2]O", "Lato", &["H2O"]),
        ("score $R^2$ = 0.94", "Lato", &["score", "=0.94"]),
        ("Mood #emoji.face", "Lato", &["Mood😀"]),
        ("שלום नमस्ते", "Lato", &["שלום", "नमस्ते"]),
        ("Caveat", "Caveat", &["Caveat"]),
    ] {
        let mut mark = scene::text(source, 10.0, 40.0, 20.0);
        mark.font = ScalarOrArray::new_scalar(font.into());
        let graph = SceneGraph {
            width: 240.0,
            height: 70.0,
            origin: [0.0, 0.0],
            marks: vec![mark.into()],
        };
        let pdf = renderer()
            .render_scene_graph(&graph, &text_engine())
            .unwrap();
        let text = pdf_extract::extract_text_from_mem(&pdf).unwrap();
        // The extractor puts whitespace between separately placed runs.
        let characters: String = text.chars().filter(|ch| !ch.is_whitespace()).collect();
        assert!(
            parts.iter().all(|part| characters.contains(part))
                && !characters.contains(['*', '_', '#', '$']),
            "{source}: {text:?}"
        );
    }
}

#[test]
#[ignore = "requires PDFium 7763; see avenger-pdf/README.md"]
fn text_matches_svg() {
    for source in [
        "H#sub[2]O",
        "H#super[2]O",
        "H#sub(typographic: false)[2]O",
        "H#super(typographic: false)[2]O",
        "#smallcaps[Smallcaps]",
        "H#smallcaps(all: true)[CAPS]O",
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
            .render_scene_graph(&graph, &text_engine())
            .unwrap();
        let svg = avenger_svg::SvgRenderer::new()
            .render_scene_graph(&graph, &text_engine())
            .unwrap();
        let actual = pdf_raster::pdf_to_png(&pdf, graph.width, graph.height);
        let expected = raster::svg_to_png(&svg, 2.0);
        // Only inked pixels count, so that a misplaced glyph can't hide in the page average.
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

/// A mesh image mark's instances export as one image, as many image objects as one instance
/// exports.
#[test]
fn mesh_image_instances_export_as_one_image() {
    let quad = |x: f32| MeshImage {
        image: avenger_image::RgbaImage {
            width: 1,
            height: 1,
            data: vec![255, 0, 0, 128],
        }
        .into(),
        positions: vec![[x, 10.0], [x + 50.0, 10.0], [x + 50.0, 60.0], [x, 60.0]],
        uvs: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
        indices: vec![0, 1, 2, 0, 2, 3],
    };
    let images = |instances: Vec<MeshImage>| {
        let graph = SceneGraph {
            width: 120.0,
            height: 70.0,
            origin: [0.0, 0.0],
            marks: vec![SceneMeshImageMark {
                instances,
                ..Default::default()
            }
            .into()],
        };
        let pdf = renderer().render_scene_graph(&graph, &engine()).unwrap();
        String::from_utf8_lossy(&pdf)
            .matches("/Subtype/Image")
            .count()
    };
    let one = images(vec![quad(10.0)]);
    assert!(one > 0);
    assert_eq!(images(vec![quad(10.0), quad(60.0)]), one);
}

/// Map tiles export to PDF without cracks: flat-colored tiles draw one color inside them, and
/// tiles of a gradient draw it as each tile samples it.
#[test]
#[ignore = "requires PDFium 7763; see avenger-pdf/README.md"]
fn map_tiles_show_no_seams() {
    use avenger_render_fixtures::tiles;
    let background = [1.0, 0.0, 1.0, 1.0];
    // PDFium draws at twice the scene's scale, so a margin of 4 pixels is 2 units.
    let margin = 4;
    for case in tiles::seam_cases() {
        let render = |graph: &SceneGraph, tiles: &tiles::Tiles| {
            let pdf = renderer()
                .with_images(tiles.clone())
                .render_scene_graph(graph, &engine())
                .unwrap();
            pdf_raster::pdf_to_png(&pdf, graph.width, graph.height)
        };
        let color = [40, 120, 200, 255];
        let flat = tiles::flat_tiles(&case.tiles(), 16, color);
        tiles::assert_flat(
            &case,
            &render(&case.scene(&flat, background, false), &flat),
            color,
            margin,
        );
        let gradient = tiles::gradient_tiles(&case.tiles(), 16);
        let image = render(&case.scene(&gradient, background, false), &gradient);
        tiles::assert_smooth(&case, &image, 16, 3.0, margin);
    }
}

/// OpenStreetMap tiles on an orthographic globe, under its outline.
#[test]
#[ignore = "requires PDFium 7763; see avenger-pdf/README.md"]
fn map_tiles_match_the_baseline() {
    use avenger_render_fixtures::tiles;
    let case = tiles::cases()
        .into_iter()
        .find(|case| case.name == "globe")
        .unwrap();
    let osm = tiles::osm_tiles(&case.tiles());
    let graph = case.scene(&osm, tiles::OSM_WATER, true);
    let pdf = renderer()
        .with_images(osm)
        .render_scene_graph(&graph, &engine())
        .unwrap();
    let png = pdf_raster::pdf_to_png(&pdf, graph.width, graph.height);
    raster::assert_baseline(env!("CARGO_MANIFEST_DIR"), &png, "geo-tiles-globe");
}

/// Under the strict option, an image resource that isn't ready fails the export.
#[test]
fn the_strict_option_fails_on_unready_images() {
    use std::{collections::HashMap, sync::Arc};

    use avenger_pdf::AvengerPdfError;
    use avenger_render_fixtures::resources::{pixel, resource_scene, IMAGE_URL, MESH_URL};
    let images: Arc<HashMap<String, Arc<avenger_image::RgbaImage>>> = Arc::new(HashMap::from([(
        IMAGE_URL.to_string(),
        pixel([255, 0, 0, 255]),
    )]));
    let strict = PdfRenderer::new().with_options(PdfRenderOptions {
        strict_images: true,
        ..Default::default()
    });
    let scene = resource_scene();
    let unready = |renderer: PdfRenderer| match renderer.render_scene_graph(&scene, &engine()) {
        Err(AvengerPdfError::ImagesNotReady(urls)) => urls,
        Err(error) => panic!("expected unready images, got {error}"),
        Ok(_) => panic!("expected unready images"),
    };
    assert_eq!(
        unready(strict.clone().with_images(images.clone())),
        [MESH_URL]
    );
    assert_eq!(unready(strict.clone()), [IMAGE_URL, MESH_URL]);
    let mut all = (*images).clone();
    all.insert(MESH_URL.to_string(), pixel([0, 0, 255, 255]));
    assert!(strict
        .with_images(Arc::new(all))
        .render_scene_graph(&scene, &engine())
        .is_ok());
}

/// A resource draws the image that the renderer's resolver holds, or else its placeholder.
#[test]
#[ignore = "requires PDFium 7763; see avenger-pdf/README.md"]
fn resources_draw_their_resolved_images_or_placeholders() {
    use std::{collections::HashMap, sync::Arc};

    use avenger_render_fixtures::resources::{pixel, resource_scene, IMAGE_URL, PLACEHOLDER};
    let red = [255, 0, 0, 255];
    let images: Arc<HashMap<String, Arc<avenger_image::RgbaImage>>> =
        Arc::new(HashMap::from([(IMAGE_URL.to_string(), pixel(red))]));
    let scene = resource_scene();
    let pdf = renderer()
        .with_images(images)
        .render_scene_graph(&scene, &engine())
        .unwrap();
    // PDFium draws at twice the scene's scale.
    let png = pdf_raster::pdf_to_png(&pdf, scene.width, scene.height);
    assert_eq!(png.get_pixel(10, 10).0, red);
    assert_eq!(png.get_pixel(30, 10).0, PLACEHOLDER);
}
