//! Shared scene fixtures for vector export examples and regression tests.
use avenger_color::{ColorOrGradient, Gradient, GradientStop, LinearGradient};
use avenger_common::{types::SceneTextLeaderArrow, value::ScalarOrArray};
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
        text::SceneTextMark,
        warped_image::SceneWarpedImageMark,
    },
    scene_graph::SceneGraph,
};
use avenger_text::types::TextSyntaxMode;

pub fn fonts() -> avenger_text::FontResolutionOptions {
    avenger_text::FontResolutionOptions {
        load_system_fonts: false,
        ..avenger_text::default_font_resolution()
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
    let mut annotation = text("*Radius* $sqrt(x^2+y^2)$", 20.0, 107.0, 21.0);
    annotation.dx = ScalarOrArray::new_scalar(28.0);
    annotation.dy = ScalarOrArray::new_scalar(-53.0);
    annotation.angle = ScalarOrArray::new_scalar(-12.0);
    annotation.limit = ScalarOrArray::new_scalar(128.0);
    annotation.leader = ScalarOrArray::new_scalar(true);
    annotation.leader_arrow = ScalarOrArray::new_scalar(SceneTextLeaderArrow::Triangle);
    annotation.clip = true;
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
                annotation.into(),
            ],
            ..Default::default()
        }
        .into(),
    );
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
    let mut limited = text(
        "*Typeset first*, then clip $sqrt(x^2+y^2)$",
        28.0,
        454.0,
        22.0,
    );
    limited.limit = ScalarOrArray::new_scalar(285.0);
    marks.push(limited.into());
    marks.push(
        SceneImageMark {
            image: ScalarOrArray::new_scalar(SceneImageSource::Inline(checker())),
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
            image: SceneImageSource::Inline(checker()),
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
    marks.push(text("One scene. Shared fonts and geometry.", 28.0, 501.0, 14.0).into());
    SceneGraph {
        width: 800.0,
        height: 528.0,
        origin: [0.0, 0.0],
        marks,
    }
}
