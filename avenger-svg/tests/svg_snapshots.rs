use avenger_color::{ColorOrGradient, Gradient, GradientStop, LinearGradient, RadialGradient};
use avenger_common::{
    types::{AreaOrientation, ImageAlign, ImageBaseline, PathTransform},
    value::ScalarOrArray,
};
use avenger_image::RgbaImage;
use avenger_scenegraph::{
    marks::{
        arc::SceneArcMark,
        area::SceneAreaMark,
        group::SceneGroup,
        image::{SceneImageMark, SceneImageSource},
        line::SceneLineMark,
        path::ScenePathMark,
        rect::SceneRectMark,
        rule::SceneRuleMark,
        symbol::SceneSymbolMark,
        text::SceneTextMark,
        trail::SceneTrailMark,
    },
    scene_graph::SceneGraph,
};
use avenger_svg::{SvgBackground, SvgFontEmbedding, SvgRenderOptions, SvgRenderer};
use lyon_path::math::point;

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

    let svg = render_transparent(scene_graph);

    assert!(usvg::Tree::from_str(&svg, &usvg::Options::default()).is_ok());
    assert!(!svg.contains("<path "));
    assert!(!svg.contains("<line "));
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
                path: ScalarOrArray::new_scalar(triangle_path()),
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
            image_mark(image, 96.0, true).into(),
        ],
    };

    let svg = render_transparent(scene_graph);

    assert!(usvg::Tree::from_str(&svg, &usvg::Options::default()).is_ok());
    assert!(svg.matches("<path ").count() >= 8);
    assert_eq!(svg.matches("<line ").count(), 1);
    assert_eq!(svg.matches("<text ").count(), 1);
    assert_eq!(svg.matches("<image ").count(), 1);
    assert!(svg.contains(r##"fill="#0000ff" fill-opacity="0.2""##));
    assert!(svg.contains(r##"fill="#ff0000""##));
    assert!(svg.contains(r##"fill="#00ff00""##));
    assert!(svg.contains(r##"fill="#ff00ff" fill-opacity="0.5""##));
    assert!(svg.contains(r##"fill="#00ffff" fill-opacity="0.5""##));
    assert!(svg.contains(r#"stroke-dasharray="2 1""#));
    assert!(svg.contains(">All marks</text>"));
    assert!(svg.contains(" A"));
}

#[test]
fn non_square_radial_gradient_pattern_parses_and_rasterizes() {
    let scene_graph = SceneGraph {
        width: 40.0,
        height: 20.0,
        origin: [0.0, 0.0],
        marks: vec![SceneRectMark {
            len: 1,
            gradients: vec![Gradient::RadialGradient(RadialGradient {
                x0: 0.5,
                y0: 0.5,
                x1: 0.5,
                y1: 0.5,
                r0: 0.0,
                r1: 0.5,
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
            x: ScalarOrArray::new_scalar(4.0),
            y: ScalarOrArray::new_scalar(5.0),
            width: Some(ScalarOrArray::new_scalar(30.0)),
            height: Some(ScalarOrArray::new_scalar(8.0)),
            fill: ScalarOrArray::new_scalar(ColorOrGradient::GradientIndex(0)),
            ..Default::default()
        }
        .into()],
    };

    let svg = render_transparent(scene_graph);

    assert!(svg.contains("<pattern "));
    assert!(svg.contains("<radialGradient "));
    assert!(svg.contains(r#"fill="url(#svg-gradient-0)""#));
    assert_svg_parses_and_rasterizes(&svg);
}

#[test]
fn linear_gradient_inside_clip_snapshot_parses() {
    let scene_graph = SceneGraph {
        width: 32.0,
        height: 20.0,
        origin: [0.0, 0.0],
        marks: vec![SceneGroup {
            clip: avenger_scenegraph::marks::group::Clip::Rect {
                x: 2.0,
                y: 2.0,
                width: 24.0,
                height: 12.0,
            },
            marks: vec![SceneRectMark {
                len: 1,
                clip: true,
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
                x: ScalarOrArray::new_scalar(0.0),
                y: ScalarOrArray::new_scalar(0.0),
                width: Some(ScalarOrArray::new_scalar(30.0)),
                height: Some(ScalarOrArray::new_scalar(18.0)),
                fill: ScalarOrArray::new_scalar(ColorOrGradient::GradientIndex(0)),
                ..Default::default()
            }
            .into()],
            ..Default::default()
        }
        .into()],
    };

    let svg = render_transparent(scene_graph);

    assert!(svg.contains("<clipPath "));
    assert!(svg.contains("<linearGradient "));
    assert!(svg.contains(r#"fill="url(#svg-gradient-0)""#));
    assert!(svg.contains(r#"clip-path="url(#svg-clip-0)""#));
    assert!(usvg::Tree::from_str(&svg, &usvg::Options::default()).is_ok());
}

#[test]
fn group_clip_ordering_and_undefined_breaks_are_stable() {
    let scene_graph = SceneGraph {
        width: 50.0,
        height: 30.0,
        origin: [0.0, 0.0],
        marks: vec![SceneGroup {
            name: "panel".to_string(),
            origin: [1.0, 2.0],
            clip: avenger_scenegraph::marks::group::Clip::Rect {
                x: 0.0,
                y: 0.0,
                width: 42.0,
                height: 18.0,
            },
            fill: Some(ColorOrGradient::Color([0.0, 0.0, 1.0, 1.0])),
            stroke: Some(ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0])),
            stroke_width: Some(1.0),
            marks: vec![
                SceneLineMark {
                    len: 5,
                    x: ScalarOrArray::new_array(vec![0.0, 10.0, 20.0, 30.0, 40.0]),
                    y: ScalarOrArray::new_array(vec![3.0, 3.0, 3.0, 10.0, 10.0]),
                    defined: ScalarOrArray::new_array(vec![true, true, false, true, true]),
                    stroke: ColorOrGradient::Color([0.0, 0.5, 0.0, 1.0]),
                    ..Default::default()
                }
                .into(),
                SceneAreaMark {
                    len: 5,
                    orientation: AreaOrientation::Vertical,
                    x: ScalarOrArray::new_array(vec![0.0, 10.0, 20.0, 30.0, 40.0]),
                    y: ScalarOrArray::new_array(vec![12.0, 10.0, 8.0, 6.0, 4.0]),
                    y2: ScalarOrArray::new_array(vec![16.0, 16.0, 16.0, 16.0, 16.0]),
                    defined: ScalarOrArray::new_array(vec![true, true, false, true, true]),
                    fill: ColorOrGradient::Color([1.0, 0.0, 0.0, 0.5]),
                    stroke: ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0]),
                    stroke_dash: Some(vec![2.0, 1.0]),
                    ..Default::default()
                }
                .into(),
            ],
            ..Default::default()
        }
        .into()],
    };

    let svg = render_transparent(scene_graph);

    let panel_fill = svg.find(r##"fill="#0000ff""##).unwrap();
    let line_stroke = svg.find(r##"stroke="#008000""##).unwrap();
    assert!(panel_fill < line_stroke);
    assert!(svg.contains(r#"<clipPath id="svg-clip-0""#));
    assert!(svg.contains(r#"clip-path="url(#svg-clip-0)""#));
    assert!(svg.contains(r#"stroke-dasharray="2 1""#));
    assert!(svg.matches(" M").count() >= 2);
    assert!(usvg::Tree::from_str(&svg, &usvg::Options::default()).is_ok());
}

#[test]
fn text_limit_snapshot_uses_default_ellipsis() {
    let scene_graph = SceneGraph {
        width: 80.0,
        height: 20.0,
        origin: [0.0, 0.0],
        marks: vec![SceneTextMark {
            text: ScalarOrArray::new_scalar("Long label text".to_string()),
            x: ScalarOrArray::new_scalar(4.0),
            y: ScalarOrArray::new_scalar(12.0),
            font: ScalarOrArray::new_scalar("Lato".to_string()),
            font_size: ScalarOrArray::new_scalar(10.0),
            limit: ScalarOrArray::new_scalar(35.0),
            ..Default::default()
        }
        .into()],
    };

    let svg = render_transparent(scene_graph);

    assert!(svg.contains("\u{2026}</text>"));
    assert!(!svg.contains("Long label text</text>"));
    assert!(usvg::Tree::from_str(&svg, &usvg::Options::default()).is_ok());
}

#[test]
fn native_text_rasterizes_with_controlled_avenger_fontdb() {
    let scene_graph = SceneGraph {
        width: 80.0,
        height: 24.0,
        origin: [0.0, 0.0],
        marks: vec![SceneTextMark {
            text: ScalarOrArray::new_scalar("Embedded".to_string()),
            x: ScalarOrArray::new_scalar(4.0),
            y: ScalarOrArray::new_scalar(16.0),
            font: ScalarOrArray::new_scalar("Lato".to_string()),
            font_size: ScalarOrArray::new_scalar(14.0),
            ..Default::default()
        }
        .into()],
    };

    let svg = SvgRenderer::new()
        .with_options(SvgRenderOptions {
            background: SvgBackground::Transparent,
            font_embedding: SvgFontEmbedding::None,
            ..Default::default()
        })
        .render_scene_graph(&scene_graph)
        .unwrap();
    let mut options = usvg::Options::default();
    options.fontdb = std::sync::Arc::new(avenger_text::fonts::build_fontdb(
        &avenger_text::FontResolutionOptions::default(),
    ));

    assert!(svg.contains("<text "));
    assert!(!svg.contains("data:font/woff2;base64,"));
    assert_svg_parses_and_rasterizes_with_options(&svg, &options);
}

#[test]
fn image_smoothing_snapshot_marks_only_unsmoothed_images_pixelated() {
    let image = RgbaImage {
        width: 1,
        height: 1,
        data: vec![255, 0, 0, 255],
    };
    let scene_graph = SceneGraph {
        width: 30.0,
        height: 12.0,
        origin: [0.0, 0.0],
        marks: vec![
            image_mark(image.clone(), 2.0, true).into(),
            image_mark(image, 14.0, false).into(),
        ],
    };

    let svg = render_transparent(scene_graph);

    assert_eq!(svg.matches("<image ").count(), 2);
    assert_eq!(svg.matches(r#"image-rendering="pixelated""#).count(), 1);
    assert!(usvg::Tree::from_str(&svg, &usvg::Options::default()).is_ok());
}

#[test]
fn renders_warped_image_mark_as_embedded_raster() {
    use avenger_scenegraph::marks::warped_image::SceneWarpedImageMark;

    let scene_graph = SceneGraph {
        width: 30.0,
        height: 24.0,
        origin: [0.0, 0.0],
        marks: vec![SceneWarpedImageMark {
            smooth: false,
            image: SceneImageSource::Inline(RgbaImage {
                width: 2,
                height: 2,
                data: vec![
                    255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
                ],
            }),
            positions: vec![[2.0, 2.0], [22.0, 4.0], [24.0, 20.0], [4.0, 18.0]],
            uvs: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            indices: vec![0, 1, 2, 0, 2, 3],
            ..Default::default()
        }
        .into()],
    };

    let svg = render_transparent(scene_graph);

    assert_eq!(svg.matches("<image ").count(), 1);
    assert!(svg.contains("data:image/png;base64,"));
    assert!(usvg::Tree::from_str(&svg, &usvg::Options::default()).is_ok());
}

fn render_transparent(scene_graph: SceneGraph) -> String {
    SvgRenderer::new()
        .with_options(SvgRenderOptions {
            background: SvgBackground::Transparent,
            font_embedding: SvgFontEmbedding::None,
            ..Default::default()
        })
        .render_scene_graph(&scene_graph)
        .unwrap()
}

fn image_mark(image: RgbaImage, x: f32, smooth: bool) -> SceneImageMark {
    SceneImageMark {
        len: 1,
        aspect: false,
        smooth,
        image: ScalarOrArray::new_scalar(SceneImageSource::Inline(image)),
        x: ScalarOrArray::new_scalar(x),
        y: ScalarOrArray::new_scalar(2.0),
        width: ScalarOrArray::new_scalar(10.0),
        height: ScalarOrArray::new_scalar(8.0),
        align: ScalarOrArray::new_scalar(ImageAlign::Left),
        baseline: ScalarOrArray::new_scalar(ImageBaseline::Top),
        ..Default::default()
    }
}

fn triangle_path() -> lyon_path::Path {
    let mut builder = lyon_path::Path::builder();
    builder.begin(point(0.0, 0.0));
    builder.line_to(point(8.0, 0.0));
    builder.line_to(point(4.0, 8.0));
    builder.close();
    builder.build()
}

fn assert_svg_parses_and_rasterizes(svg: &str) {
    assert_svg_parses_and_rasterizes_with_options(svg, &usvg::Options::default());
}

fn assert_svg_parses_and_rasterizes_with_options(svg: &str, options: &usvg::Options<'_>) {
    let tree = usvg::Tree::from_str(svg, options).unwrap();
    let mut pixmap = tiny_skia::Pixmap::new(
        tree.size().width().ceil() as u32,
        tree.size().height().ceil() as u32,
    )
    .unwrap();
    resvg::render(
        &tree,
        tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );

    assert!(pixmap.data().chunks_exact(4).any(|rgba| rgba[3] != 0));
}
