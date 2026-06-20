use std::path::{Path, PathBuf};

use avenger_color::{ColorOrGradient, Gradient, GradientStop, LinearGradient};
use avenger_common::{canvas::CanvasDimensions, value::ScalarOrArray};
use avenger_scenegraph::{
    marks::{
        group::{Clip, SceneGroup},
        rect::SceneRectMark,
        text::SceneTextMark,
    },
    scene_graph::SceneGraph,
};
use avenger_svg::{SvgFontEmbedding, SvgRenderOptions, SvgRenderer};
use avenger_text::FontResolutionOptions;
use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};
use image::RgbaImage;

const SCALE: f32 = 2.0;

#[tokio::test]
async fn svg_wgpu_subpixel_clip_parity_is_within_tolerance() {
    let scene_graph = SceneGraph {
        width: 80.0,
        height: 50.0,
        origin: [0.0, 0.0],
        marks: vec![
            SceneGroup {
                clip: Clip::Rect {
                    x: 13.25,
                    y: 10.5,
                    width: 42.5,
                    height: 24.75,
                },
                marks: vec![
                    SceneRectMark {
                        len: 1,
                        clip: true,
                        x: ScalarOrArray::new_scalar(6.0),
                        y: ScalarOrArray::new_scalar(7.0),
                        width: Some(ScalarOrArray::new_scalar(62.0)),
                        height: Some(ScalarOrArray::new_scalar(34.0)),
                        fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([
                            0.1, 0.42, 0.74, 1.0,
                        ])),
                        ..Default::default()
                    }
                    .into(),
                ],
                ..Default::default()
            }
            .into(),
        ],
    };

    // SVG clips in logical floating-point coordinates; WGPU clips through
    // physical pixel scissor state, so subpixel clip edges differ at the seam.
    assert_svg_wgpu_similarity("subpixel_clip", &scene_graph, 0.985).await;
}

#[tokio::test]
async fn svg_wgpu_linear_gradient_parity_is_within_tolerance() {
    let scene_graph = SceneGraph {
        width: 96.0,
        height: 42.0,
        origin: [0.0, 0.0],
        marks: vec![
            SceneRectMark {
                len: 1,
                gradients: vec![Gradient::LinearGradient(LinearGradient {
                    x0: 0.0,
                    y0: 0.0,
                    x1: 1.0,
                    y1: 0.0,
                    stops: vec![
                        GradientStop {
                            offset: 0.0,
                            color: [0.08, 0.16, 0.65, 1.0],
                        },
                        GradientStop {
                            offset: 0.5,
                            color: [0.1, 0.72, 0.45, 1.0],
                        },
                        GradientStop {
                            offset: 1.0,
                            color: [1.0, 0.82, 0.18, 1.0],
                        },
                    ],
                })],
                x: ScalarOrArray::new_scalar(8.0),
                y: ScalarOrArray::new_scalar(8.0),
                width: Some(ScalarOrArray::new_scalar(80.0)),
                height: Some(ScalarOrArray::new_scalar(26.0)),
                fill: ScalarOrArray::new_scalar(ColorOrGradient::GradientIndex(0)),
                ..Default::default()
            }
            .into(),
        ],
    };

    // SVG gradients are continuous; WGPU samples gradients through a finite
    // texture, so small interpolation differences are expected.
    assert_svg_wgpu_similarity("linear_gradient", &scene_graph, 0.985).await;
}

#[tokio::test]
async fn svg_wgpu_native_text_parity_is_within_tolerance() {
    let scene_graph = SceneGraph {
        width: 150.0,
        height: 42.0,
        origin: [0.0, 0.0],
        marks: vec![
            SceneTextMark {
                text: ScalarOrArray::new_scalar("Native text".to_string()),
                x: ScalarOrArray::new_scalar(8.0),
                y: ScalarOrArray::new_scalar(28.0),
                font: ScalarOrArray::new_scalar("Atkinson Hyperlegible Next".to_string()),
                font_size: ScalarOrArray::new_scalar(20.0),
                color: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0])),
                ..Default::default()
            }
            .into(),
        ],
    };

    // SVG/resvg and WGPU/cosmic-text use different text rasterizers. This keeps
    // the native text path honest without requiring pixel-identical glyph AA.
    assert_svg_wgpu_similarity("native_text", &scene_graph, 0.9).await;
}

async fn assert_svg_wgpu_similarity(name: &str, scene_graph: &SceneGraph, threshold: f64) {
    let svg = SvgRenderer::new()
        .with_options(SvgRenderOptions {
            font_embedding: SvgFontEmbedding::None,
            ..Default::default()
        })
        .render_scene_graph(scene_graph)
        .expect("render SVG scene");
    let svg_image = rasterize_svg_with_avenger_fonts(&svg);
    let wgpu_image = render_wgpu(scene_graph).await;

    assert_eq!(svg_image.dimensions(), wgpu_image.dimensions());

    let comparison = image_compare::rgba_hybrid_compare(&svg_image, &wgpu_image)
        .expect("compare SVG and WGPU images");
    println!(
        "{name} SVG/WGPU similarity {:.6} (threshold {:.6})",
        comparison.score, threshold
    );
    if comparison.score < threshold {
        save_failure_pair(
            name,
            &svg_image,
            &wgpu_image,
            &comparison.image.to_color_map().into_rgba8(),
        )
        .expect("save SVG/WGPU parity failures");
    }

    assert!(
        comparison.score >= threshold,
        "{name} SVG/WGPU similarity {:.6} was below threshold {:.6}",
        comparison.score,
        threshold
    );
}

fn rasterize_svg_with_avenger_fonts(svg: &str) -> RgbaImage {
    let mut options = usvg::Options::default();
    options.fontdb = std::sync::Arc::new(avenger_text::fonts::build_fontdb(
        &FontResolutionOptions::default(),
    ));
    let tree = usvg::Tree::from_str(svg, &options).expect("parse SVG");
    let width = (tree.size().width() * SCALE).ceil() as u32;
    let height = (tree.size().height() * SCALE).ceil() as u32;
    let mut pixmap = tiny_skia::Pixmap::new(width, height).expect("allocate SVG pixmap");

    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(SCALE, SCALE),
        &mut pixmap.as_mut(),
    );

    RgbaImage::from_raw(width, height, pixmap.data().to_vec()).expect("convert SVG pixmap")
}

async fn render_wgpu(scene_graph: &SceneGraph) -> RgbaImage {
    let mut canvas = PngCanvas::new(
        CanvasDimensions {
            size: [scene_graph.width, scene_graph.height],
            scale: SCALE,
        },
        CanvasConfig::default(),
    )
    .await
    .expect("create WGPU PNG canvas");
    canvas.set_scene(scene_graph).expect("set WGPU scene");
    canvas.render().await.expect("render WGPU PNG")
}

fn save_failure_pair(
    name: &str,
    svg_image: &RgbaImage,
    wgpu_image: &RgbaImage,
    diff_image: &RgbaImage,
) -> Result<(), String> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("failures_svg_wgpu");
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("failed to create {}: {e}", dir.display()))?;

    save_image(svg_image, &dir.join(format!("{name}_svg.png")))?;
    save_image(wgpu_image, &dir.join(format!("{name}_wgpu.png")))?;
    save_image(diff_image, &dir.join(format!("{name}_diff.png")))?;
    Ok(())
}

fn save_image(image: &RgbaImage, path: &Path) -> Result<(), String> {
    image
        .save(path)
        .map_err(|e| format!("failed to save {}: {e}", path.display()))
}
