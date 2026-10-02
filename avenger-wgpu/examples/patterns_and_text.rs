//! Render plot-anchored patterns and a Typst label directly from scene marks.
use avenger_color::ColorOrGradient;
use avenger_common::{canvas::CanvasDimensions, value::ScalarOrArray};
use avenger_scenegraph::{
    marks::{
        group::SceneGroup,
        pattern::{
            PatternAnchor, PatternFill, PatternLayer, PatternReferenceFrame, StripePatternLayer,
        },
        rect::SceneRectMark,
        text::SceneTextMark,
    },
    scene_graph::SceneGraph,
};
use avenger_text::types::TextSyntaxMode;
use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "patterns-and-text.png".into());
    let pattern = PatternFill {
        anchor: PatternAnchor::Plot,
        layers: vec![PatternLayer::Stripe(StripePatternLayer::new(
            45.0, 12.0, 3.0,
        ))],
        ..Default::default()
    };
    let scene = SceneGraph {
        width: 480.0,
        height: 260.0,
        origin: [0.0, 0.0],
        marks: vec![
            SceneTextMark {
                text: ScalarOrArray::new_scalar("*Pattern fills* and $sqrt(x^2 + y^2)$".into()),
                text_syntax: TextSyntaxMode::TypstMarkup,
                font: ScalarOrArray::new_scalar("Lato".into()),
                font_size: ScalarOrArray::new_scalar(24.0),
                x: ScalarOrArray::new_scalar(24.0),
                y: ScalarOrArray::new_scalar(42.0),
                ..Default::default()
            }
            .into(),
            SceneGroup {
                origin: [24.0, 74.0],
                // The frame uses group-local coordinates. All bars share its stripe phase.
                pattern_reference_frame: Some(PatternReferenceFrame {
                    x: 0.0,
                    y: 0.0,
                    width: 432.0,
                    height: 160.0,
                }),
                marks: vec![SceneRectMark {
                    len: 3,
                    x: ScalarOrArray::new_array(vec![0.0, 150.0, 300.0]),
                    y: ScalarOrArray::new_array(vec![48.0, 0.0, 24.0]),
                    width: Some(ScalarOrArray::new_scalar(132.0)),
                    height: Some(ScalarOrArray::new_array(vec![112.0, 160.0, 136.0])),
                    fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.3, 0.65, 0.8, 1.0])),
                    fill_pattern: ScalarOrArray::new_scalar(Some(pattern)),
                    ..Default::default()
                }
                .into()],
                ..Default::default()
            }
            .into(),
        ],
    };
    let mut canvas = pollster::block_on(PngCanvas::new(
        CanvasDimensions {
            size: [scene.width, scene.height],
            scale: 2.0,
        },
        CanvasConfig::default(),
    ))?;
    canvas.set_scene(&scene)?;
    pollster::block_on(canvas.render())?.save(&output)?;
    println!("Saved {output}");
    Ok(())
}
