//! Shared scene fixtures for vector export examples and regression tests.
use avenger_annotation::leader::{make_text_leaders, LeaderArrow, LeaderStyle, LeaderTarget};
use avenger_color::{ColorOrGradient, Gradient, GradientStop, LinearGradient};
use avenger_common::types::{TextAlign, TextBaseline, TextSyntaxMode};
use avenger_common::value::ScalarOrArray;
use avenger_image::RgbaImage;
use avenger_scenegraph::{
    marks::{
        group::{Clip, SceneGroup},
        image::{SceneImageMark, SceneImageSource},
        pattern::{
            PatternAnchor, PatternFill, PatternLayer, PatternLayerOperation, PatternReferenceFrame,
            StripePatternLayer,
        },
        rect::SceneRectMark,
        rule::SceneRuleMark,
        text::SceneTextMark,
        warped_image::SceneWarpedImageMark,
    },
    scene_graph::SceneGraph,
};
use avenger_typst_label::{LabelAlign, LabelLineHeight, LabelWidth};

const CLIP_GUIDE_COLOR: [f32; 4] = [0.80, 0.25, 0.05, 1.0];

pub fn fonts() -> avenger_typst_label::FontOptions {
    avenger_typst_label::FontOptions {
        load_system_fonts: false,
        ..avenger_typst_label::bundled_font_options()
    }
}

pub fn text(source: &str, x: f32, y: f32, size: f32) -> SceneTextMark {
    SceneTextMark {
        text: ScalarOrArray::new_scalar(source.into()),
        text_syntax: TextSyntaxMode::TypstMarkup,
        font: ScalarOrArray::new_scalar("Lato".into()),
        font_size: ScalarOrArray::new_scalar(size),
        color: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.10, 0.16, 0.25, 1.0])),
        x: ScalarOrArray::new_scalar(x),
        y: ScalarOrArray::new_scalar(y),
        ..Default::default()
    }
}

fn rect(x: f32, y: f32, width: f32, height: f32, color: [f32; 4]) -> SceneRectMark {
    SceneRectMark {
        x: ScalarOrArray::new_scalar(x),
        y: ScalarOrArray::new_scalar(y),
        width: Some(ScalarOrArray::new_scalar(width)),
        height: Some(ScalarOrArray::new_scalar(height)),
        fill: ScalarOrArray::new_scalar(ColorOrGradient::Color(color)),
        ..Default::default()
    }
}

fn text_width_boundary(label: [f32; 2], width: f32, angle: f32, size: f32) -> SceneRuleMark {
    let (sin, cos) = angle.to_radians().sin_cos();
    let point = |y: f32| {
        [
            label[0] + width * cos - y * sin,
            label[1] + width * sin + y * cos,
        ]
    };
    // A text width bounds only the right edge, in the label's rotated coordinates.
    let start = point(-1.2 * size);
    let end = point(0.6 * size);
    SceneRuleMark {
        clip: false,
        x: ScalarOrArray::new_scalar(start[0]),
        y: ScalarOrArray::new_scalar(start[1]),
        x2: ScalarOrArray::new_scalar(end[0]),
        y2: ScalarOrArray::new_scalar(end[1]),
        stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color(CLIP_GUIDE_COLOR)),
        stroke_width: ScalarOrArray::new_scalar(1.5),
        stroke_dash: Some(ScalarOrArray::new_scalar(vec![4.0, 3.0])),
        ..Default::default()
    }
}

/// A panel of multi-line labels: wrapped and cut, a fixed-width box with centered lines of
/// plain text, and the box and line box baselines against one guide.
fn multiline_labels() -> Vec<avenger_scenegraph::marks::mark::SceneMark> {
    let mut marks = vec![text("*Multi-line labels*", 286.0, 320.0, 17.0).into()];

    // Wrapped at a width, centered on its position, and cut to two lines on a relative grid.
    let mut wrapped = text(
        "Long category names wrap at the width and stop after two lines with an ellipsis",
        395.0,
        344.0,
        13.0,
    );
    wrapped.align = ScalarOrArray::new_scalar(TextAlign::Center);
    wrapped.baseline = ScalarOrArray::new_scalar(TextBaseline::Top);
    wrapped.width = ScalarOrArray::new_scalar(LabelWidth::Max(210.0));
    wrapped.max_lines = std::num::NonZeroUsize::new(2);
    wrapped.ellipsis = true;
    wrapped.line_height = ScalarOrArray::new_scalar(LabelLineHeight::Relative(1.1));
    wrapped.line_align = ScalarOrArray::new_scalar(LabelAlign::Center);
    marks.push(wrapped.into());

    // A fixed-width box anchored at its left, with centered lines of plain text.
    marks.push(rect(286.0, 390.0, 210.0, 34.0, [0.94, 0.96, 0.98, 1.0]).into());
    let mut lines = text("Plain text whose\nnewlines end lines", 286.0, 390.0, 13.0);
    lines.text_syntax = TextSyntaxMode::PlainLines;
    lines.baseline = ScalarOrArray::new_scalar(TextBaseline::Top);
    lines.width = ScalarOrArray::new_scalar(LabelWidth::Fixed(210.0));
    lines.line_align = ScalarOrArray::new_scalar(LabelAlign::Center);
    marks.push(lines.into());

    // The box's top and bottom, and the line box's, which add half the gap between lines.
    marks.push(
        SceneRuleMark {
            x: ScalarOrArray::new_scalar(330.0),
            y: ScalarOrArray::new_scalar(456.0),
            x2: ScalarOrArray::new_scalar(504.0),
            y2: ScalarOrArray::new_scalar(456.0),
            stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.6, 0.65, 0.7, 1.0])),
            stroke_width: ScalarOrArray::new_scalar(1.0),
            ..Default::default()
        }
        .into(),
    );
    for (source, x, baseline) in [
        ("Top", 330.0, TextBaseline::Top),
        ("LineTop", 354.0, TextBaseline::LineTop),
        ("Bottom", 400.0, TextBaseline::Bottom),
        ("LineBottom", 442.0, TextBaseline::LineBottom),
    ] {
        let mut label = text(source, x, 456.0, 11.0);
        label.baseline = ScalarOrArray::new_scalar(baseline);
        marks.push(label.into());
    }
    marks
}

pub fn checker() -> RgbaImage {
    let mut data = Vec::new();
    for y in 0..8 {
        for x in 0..8 {
            data.extend_from_slice(if (x + y) % 2 == 0 {
                &[36, 115, 150, 255]
            } else {
                &[251, 193, 103, 255]
            });
        }
    }
    RgbaImage {
        width: 8,
        height: 8,
        data,
    }
}

pub fn gallery() -> SceneGraph {
    let blue = [0.32, 0.66, 0.79, 1.0];
    let mut marks = vec![
        text("*Scene graph to vector document*", 28.0, 43.0, 27.0).into(),
        text(
            "Patterns, typeset labels, clipping, gradients, and images",
            28.0,
            72.0,
            16.0,
        )
        .into(),
        text("*Shared pattern coordinates*", 28.0, 116.0, 17.0).into(),
        text("*Gradient & layer masks*", 286.0, 116.0, 17.0).into(),
        text("*Clipped annotation*", 544.0, 116.0, 17.0).into(),
    ];
    let mut bars = rect(0.0, 0.0, 62.0, 100.0, blue);
    bars.len = 3;
    bars.x = ScalarOrArray::new_array(vec![0.0, 78.0, 156.0]);
    bars.y = ScalarOrArray::new_array(vec![42.0, 0.0, 22.0]);
    bars.height = Some(ScalarOrArray::new_array(vec![90.0, 132.0, 110.0]));
    bars.fill_pattern = ScalarOrArray::new_scalar(Some(PatternFill {
        anchor: PatternAnchor::Plot,
        layers: vec![PatternLayer::Stripe(StripePatternLayer::new(
            45.0, 12.0, 3.0,
        ))],
        ..Default::default()
    }));
    marks.push(
        SceneGroup {
            origin: [28.0, 138.0],
            pattern_reference_frame: Some(PatternReferenceFrame {
                x: 0.0,
                y: 0.0,
                width: 218.0,
                height: 132.0,
            }),
            marks: vec![bars.into()],
            ..Default::default()
        }
        .into(),
    );
    let mut gradient = rect(286.0, 138.0, 218.0, 132.0, blue);
    gradient.fill = ScalarOrArray::new_scalar(ColorOrGradient::GradientIndex(0));
    gradient.gradients = vec![Gradient::LinearGradient(LinearGradient {
        x0: 0.0,
        y0: 0.0,
        x1: 1.0,
        y1: 1.0,
        stops: vec![
            GradientStop {
                offset: 0.0,
                color: [0.25, 0.62, 0.77, 1.0],
            },
            GradientStop {
                offset: 1.0,
                color: [0.95, 0.76, 0.40, 1.0],
            },
        ],
    })];
    let mut subtract = StripePatternLayer::new(-45.0, 18.0, 5.0);
    subtract.operation = PatternLayerOperation::Subtract;
    gradient.fill_pattern = ScalarOrArray::new_scalar(Some(PatternFill {
        anchor: PatternAnchor::Mark,
        layers: vec![
            PatternLayer::Stripe(StripePatternLayer::new(45.0, 12.0, 5.0)),
            PatternLayer::Stripe(subtract),
        ],
        ..Default::default()
    }));
    marks.push(gradient.into());
    let mut annotation = text("*Radius* $sqrt(x^2+y^2)$", 48.0, 54.0, 21.0);
    let annotation_angle = -12.0;
    let annotation_width = 128.0;
    annotation.angle = ScalarOrArray::new_scalar(annotation_angle);
    annotation.width =
        ScalarOrArray::new_scalar(avenger_typst_label::LabelWidth::Max(annotation_width));
    annotation.wrap = false;
    annotation.ellipsis = true;
    annotation.clip = true;
    let engine = avenger_typst_label::LabelEngine::new(avenger_typst_label::EngineOptions {
        fonts: fonts(),
    });
    let style = LeaderStyle {
        arrow: LeaderArrow::Triangle,
        ..Default::default()
    };
    let target = LeaderTarget {
        position: [20.0, 107.0],
        radius: 0.0,
    };
    let leaders = make_text_leaders(&annotation, &[Some(target)], &style, &engine)
        .expect("the annotation lays out");
    marks.push(
        SceneGroup {
            origin: [544.0, 138.0],
            clip: Clip::Rect {
                x: 0.0,
                y: 0.0,
                width: 218.0,
                height: 132.0,
            },
            marks: vec![
                rect(0.0, 0.0, 218.0, 132.0, [0.94, 0.96, 0.98, 1.0]).into(),
                leaders.into(),
                annotation.into(),
                text_width_boundary([48.0, 54.0], annotation_width, annotation_angle, 21.0).into(),
            ],
            ..Default::default()
        }
        .into(),
    );
    let mut group_boundary = rect(544.0, 138.0, 218.0, 132.0, [0.0; 4]);
    group_boundary.clip = false;
    group_boundary.stroke = ScalarOrArray::new_scalar(ColorOrGradient::Color(CLIP_GUIDE_COLOR));
    group_boundary.stroke_width = ScalarOrArray::new_scalar(1.5);
    marks.push(group_boundary.into());
    marks.extend([
        text("*Typeset text & mathematics*", 28.0, 320.0, 17.0).into(),
        text("*Radius* $sqrt(x^2+y^2)$", 28.0, 366.0, 28.0).into(),
        text(
            "H#sub[2]O  #underline[Decorations]  _Italic_",
            28.0,
            411.0,
            23.0,
        )
        .into(),
        text("*Embedded & warped images*", 544.0, 320.0, 17.0).into(),
    ]);
    let mut cut = text(
        "*Typeset first*, then cut $sqrt(x^2+y^2)$",
        28.0,
        454.0,
        22.0,
    );
    let text_width = 285.0;
    cut.width = ScalarOrArray::new_scalar(avenger_typst_label::LabelWidth::Max(text_width));
    cut.wrap = false;
    cut.ellipsis = true;
    marks.push(cut.into());
    marks.push(text_width_boundary([28.0, 454.0], text_width, 0.0, 22.0).into());
    marks.push(
        SceneImageMark {
            image: ScalarOrArray::new_scalar(SceneImageSource::inline(checker())),
            x: ScalarOrArray::new_scalar(544.0),
            y: ScalarOrArray::new_scalar(346.0),
            width: ScalarOrArray::new_scalar(82.0),
            height: ScalarOrArray::new_scalar(100.0),
            smooth: false,
            aspect: false,
            ..Default::default()
        }
        .into(),
    );
    marks.push(
        SceneWarpedImageMark {
            image: SceneImageSource::inline(checker()),
            positions: vec![
                [650.0, 356.0],
                [751.0, 336.0],
                [773.0, 440.0],
                [641.0, 458.0],
            ],
            uvs: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            indices: vec![0, 1, 2, 0, 2, 3],
            smooth: false,
            ..Default::default()
        }
        .into(),
    );
    marks.extend(multiline_labels());
    marks.push(text("One scene. Shared fonts and geometry.", 28.0, 501.0, 14.0).into());
    let mut clip_key = text("Orange guides: clips and text widths", 430.0, 501.0, 14.0);
    clip_key.color = ScalarOrArray::new_scalar(ColorOrGradient::Color(CLIP_GUIDE_COLOR));
    marks.push(clip_key.into());
    SceneGraph {
        width: 800.0,
        height: 528.0,
        origin: [0.0, 0.0],
        marks,
    }
}
