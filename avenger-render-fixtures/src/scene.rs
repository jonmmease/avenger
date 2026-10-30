//! Shared scene fixtures for vector export examples and regression tests.
use avenger_annotation::leader::{make_text_leaders, LeaderArrow, LeaderStyle, LeaderTarget};
use avenger_color::{ColorOrGradient, Gradient, GradientStop, LinearGradient};
use avenger_common::pattern::{
    PatternAnchor, PatternFill, PatternLayer, PatternLayerOperation, StripeLayer,
};
use avenger_common::types::{TextAlign, TextBaseline, TextSyntaxMode};
use avenger_common::value::ScalarOrArray;
use avenger_image::RgbaImage;
use avenger_scenegraph::{
    marks::{
        group::{Clip, SceneGroup},
        image::SceneImageMark,
        mesh_image::{MeshImage, SceneMeshImageMark},
        path::ScenePathMark,
        rect::SceneRectMark,
        rule::SceneRuleMark,
        text::{contrast_halo_colors, contrast_text_colors, SceneTextMark, TextHalo, TextPaths},
    },
    scene_graph::SceneGraph,
};
use avenger_typst_label::{LabelAlign, LabelLineHeight, LabelWidth, PathOffset, PathOverflow};

const CLIP_GUIDE_COLOR: [f32; 4] = [0.80, 0.25, 0.05, 1.0];

pub fn fonts() -> avenger_typst_label::FontOptions {
    avenger_typst_label::FontOptions {
        load_system_fonts: false,
        ..avenger_typst_label::bundled_font_options()
    }
}

/// A copy of a single-face font whose `OS/2` table's embedding flags are `fs_type`.
pub fn with_embedding_flags(font: &[u8], fs_type: u16) -> Vec<u8> {
    let mut font = font.to_vec();
    let tables = u16::from_be_bytes([font[4], font[5]]) as usize;
    let record = (0..tables)
        .map(|table| 12 + table * 16)
        .find(|record| &font[*record..record + 4] == b"OS/2")
        .expect("the font has an OS/2 table");
    let os2 = u32::from_be_bytes(font[record + 8..record + 12].try_into().unwrap()) as usize;
    font[os2 + 8..os2 + 10].copy_from_slice(&fs_type.to_be_bytes());
    font
}

pub fn text(source: &str, x: f32, y: f32, size: f32) -> SceneTextMark {
    SceneTextMark {
        text: ScalarOrArray::new_scalar(source.into()),
        text_syntax: TextSyntaxMode::TypstMarkup,
        font: ScalarOrArray::new_scalar("Lato".into()),
        font_size: ScalarOrArray::new_scalar(size),
        color: ScalarOrArray::new_scalar([0.10, 0.16, 0.25, 1.0]),
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

/// A line of Shakespeare along a curve, after the TextPath demo of Konva
/// (<https://konvajs.org/docs/shapes/TextPath.html>): its path data, its text at 16 pixels in dark
/// gray, and its path at `x` 0 and `y` 50, with a faint stroke along the path. The text's baseline
/// lies on the path. The path turns sharply in its first few pixels, where a baseline moved off
/// it would fold over itself and crowd the first letters.
pub fn konva_text_path() -> SceneGraph {
    let path = avenger_common::lyon::parse_svg_path("M10,10 C0,0 10,150 100,100 S300,150 400,50")
        .expect("the path parses");
    let stroke = ScenePathMark {
        path: path.clone().into(),
        fill: ColorOrGradient::Color([0.0; 4]).into(),
        stroke: ColorOrGradient::Color([0.8, 0.8, 0.8, 1.0]).into(),
        stroke_width: Some(1.0),
        ..Default::default()
    };
    let text = SceneTextMark {
        text: "All the world's a stage, and all the men and women merely players."
            .to_string()
            .into(),
        align: TextAlign::Left.into(),
        baseline: TextBaseline::Alphabetic.into(),
        font: "Lato".to_string().into(),
        font_size: 16.0.into(),
        color: [0.2, 0.2, 0.2, 1.0].into(),
        text_path: Some(TextPaths {
            path: path.into(),
            offset: PathOffset::Fraction(0.0).into(),
            shift: 0.0.into(),
            upright: true,
            overflow: PathOverflow::Extend,
        }),
        ..Default::default()
    };
    SceneGraph {
        width: 420.0,
        height: 240.0,
        origin: [0.0, 0.0],
        marks: vec![SceneGroup {
            origin: [0.0, 50.0],
            marks: vec![stroke.into(), text.into()],
            ..Default::default()
        }
        .into()],
    }
}

/// Value labels inside bars of three palettes in black or white text, white on bars with Oklch
/// lightness up to 0.6, first alone and then with halos that raise the contrast to 9:1 behind
/// black text and 7:1 behind white text.
pub fn contrast_labels() -> SceneGraph {
    let palettes: [(&str, [u32; 10]); 3] = [
        (
            "Tableau 10",
            [
                0x4e79a7, 0xf28e2b, 0xe15759, 0x76b7b2, 0x59a14f, 0xedc948, 0xb07aa1, 0xff9da7,
                0x9c755f, 0xbab0ac,
            ],
        ),
        (
            "Viridis",
            [
                0x440154, 0x482878, 0x3e4989, 0x31688e, 0x26828e, 0x1f9e89, 0x35b779, 0x6ece58,
                0xb5de2b, 0xfde725,
            ],
        ),
        (
            "Greys",
            [
                0x111111, 0x333333, 0x555555, 0x666666, 0x767676, 0x888888, 0x999999, 0xaaaaaa,
                0xcccccc, 0xeeeeee,
            ],
        ),
    ];
    let hex = |code: u32| {
        let channel = |shift: u32| ((code >> shift) & 0xff) as f32 / 255.0;
        [channel(16), channel(8), channel(0), 1.0]
    };
    let (left, cell, bar_width, bar_height) = (130.0, 86.0, 80.0, 34.0);
    let xs: Vec<f32> = (0..10).map(|i| left + i as f32 * cell).collect();
    let mut marks = Vec::new();
    let mut y = 16.0;
    for (name, codes) in palettes {
        let mut title = text(&format!("*{name}*"), 16.0, y, 15.0);
        title.baseline = TextBaseline::Top.into();
        marks.push(title.into());
        y += 28.0;
        let fills: Vec<[f32; 4]> = codes.into_iter().map(hex).collect();
        let backgrounds = ScalarOrArray::new_array(fills.clone());
        let colors = contrast_text_colors(&backgrounds, [0.0, 0.0, 0.0, 1.0], [1.0; 4], 0.6);
        let halos = contrast_halo_colors(&colors, &backgrounds, 9.0, 7.0);
        for (caption, halo) in [("Contrast text", false), ("Contrast halo", true)] {
            let mut row_title = text(caption, 16.0, y + bar_height / 2.0, 13.0);
            row_title.baseline = TextBaseline::Middle.into();
            marks.push(row_title.into());
            let mut bars = rect(0.0, y, bar_width, bar_height, [0.0; 4]);
            bars.len = 10;
            bars.x = ScalarOrArray::new_array(xs.clone());
            bars.fill = ScalarOrArray::new_array(
                fills.iter().copied().map(ColorOrGradient::Color).collect(),
            );
            marks.push(bars.into());
            marks.push(
                SceneTextMark {
                    len: 10,
                    text: ScalarOrArray::new_scalar("42.7".into()),
                    font: ScalarOrArray::new_scalar("Lato".into()),
                    font_size: ScalarOrArray::new_scalar(16.0),
                    x: ScalarOrArray::new_array(xs.iter().map(|x| x + bar_width / 2.0).collect()),
                    y: ScalarOrArray::new_scalar(y + bar_height / 2.0),
                    align: TextAlign::Center.into(),
                    baseline: TextBaseline::Middle.into(),
                    color: colors.clone(),
                    halo: halo.then(|| TextHalo {
                        color: halos.clone(),
                        width: 2.5,
                    }),
                    ..Default::default()
                }
                .into(),
            );
            y += bar_height + 12.0;
        }
        y += 10.0;
    }
    SceneGraph {
        width: left + 10.0 * cell + 10.0,
        height: y,
        origin: [0.0; 2],
        marks,
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
    bars.fill_pattern = PatternFill {
        anchor: PatternAnchor::Group,
        layers: vec![StripeLayer::new(45.0, 12.0, 3.0).into()],
        ..Default::default()
    }
    .into();
    marks.push(
        SceneGroup {
            origin: [28.0, 138.0],
            pattern_origin: Some([0.0, 0.0]),
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
    let subtract = PatternLayer::from(StripeLayer::new(-45.0, 18.0, 5.0))
        .with_operation(PatternLayerOperation::Subtract);
    gradient.fill_pattern = PatternFill {
        anchor: PatternAnchor::Mark,
        layers: vec![StripeLayer::new(45.0, 12.0, 5.0).into(), subtract],
        ..Default::default()
    }
    .into();
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
        text("*Embedded & mesh images*", 544.0, 320.0, 17.0).into(),
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
            image: ScalarOrArray::new_scalar(checker().into()),
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
        SceneMeshImageMark {
            instances: vec![MeshImage {
                image: checker().into(),
                positions: vec![
                    [650.0, 356.0],
                    [751.0, 336.0],
                    [773.0, 440.0],
                    [641.0, 458.0],
                ],
                uvs: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
                indices: vec![0, 1, 2, 0, 2, 3],
            }],
            smooth: false,
            ..Default::default()
        }
        .into(),
    );
    marks.extend(multiline_labels());
    marks.push(text("One scene. Shared fonts and geometry.", 28.0, 501.0, 14.0).into());
    let mut clip_key = text("Orange guides: clips and text widths", 430.0, 501.0, 14.0);
    clip_key.color = ScalarOrArray::new_scalar(CLIP_GUIDE_COLOR);
    marks.push(clip_key.into());
    SceneGraph {
        width: 800.0,
        height: 528.0,
        origin: [0.0, 0.0],
        marks,
    }
}
