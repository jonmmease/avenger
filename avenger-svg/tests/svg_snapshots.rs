use avenger_color::{ColorOrGradient, Gradient, GradientStop, RadialGradient};
use avenger_common::{
    types::{AreaOrientation, ImageAlign, ImageBaseline},
    value::ScalarOrArray,
};
use avenger_image::RgbaImage;
use avenger_scenegraph::{
    marks::{
        arc::SceneArcMark, area::SceneAreaMark, group::SceneGroup, image::SceneImageMark,
        line::SceneLineMark, path::ScenePathMark, rect::SceneRectMark, rule::SceneRuleMark,
        symbol::SceneSymbolMark, text::SceneTextMark, trail::SceneTrailMark,
    },
    scene_graph::SceneGraph,
};
use avenger_svg::{SvgBackground, SvgFontEmbedding, SvgRenderOptions, SvgRenderer};

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
        image: ScalarOrArray::new_scalar(image),
        x: ScalarOrArray::new_scalar(x),
        y: ScalarOrArray::new_scalar(2.0),
        width: ScalarOrArray::new_scalar(10.0),
        height: ScalarOrArray::new_scalar(8.0),
        align: ScalarOrArray::new_scalar(ImageAlign::Left),
        baseline: ScalarOrArray::new_scalar(ImageBaseline::Top),
        ..Default::default()
    }
}

fn assert_svg_parses_and_rasterizes(svg: &str) {
    let tree = usvg::Tree::from_str(svg, &usvg::Options::default()).unwrap();
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
