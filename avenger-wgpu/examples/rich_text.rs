//! Render rich labels and their width limits using a shared text engine.
use avenger_color::ColorOrGradient;
use avenger_common::canvas::CanvasDimensions;
use avenger_scenegraph::{
    marks::{group::SceneGroup, text::SceneTextMark},
    scene_graph::SceneGraph,
};
use avenger_text::types::TextSyntaxMode;
use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};

fn label(text: &str, y: f32, size: f32, syntax: TextSyntaxMode, limit: f32) -> SceneTextMark {
    SceneTextMark {
        text: text.to_string().into(),
        text_syntax: syntax,
        font: "Lato".to_string().into(),
        font_size: size.into(),
        x: 28.0.into(),
        y: y.into(),
        limit: limit.into(),
        color: ColorOrGradient::Color([0.12, 0.18, 0.26, 1.0]).into(),
        ..Default::default()
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "rich-text.png".into());
    let scene = SceneGraph {
        width: 660.0,
        height: 300.0,
        origin: [0.0, 0.0],
        marks: vec![
            label(
                "One text engine for labels and interaction",
                42.0,
                24.0,
                TextSyntaxMode::Plain,
                0.0,
            )
            .into(),
            label(
                "*Distance* $sqrt(x^2 + y^2)$ with _inline math_",
                110.0,
                26.0,
                TextSyntaxMode::TypstMarkup,
                0.0,
            )
            .into(),
            label(
                "Plain text is ellipsized at a grapheme boundary",
                184.0,
                22.0,
                TextSyntaxMode::Plain,
                330.0,
            )
            .into(),
            label(
                "*Markup* keeps its syntax and clips at the width limit",
                244.0,
                22.0,
                TextSyntaxMode::TypstMarkup,
                330.0,
            )
            .into(),
        ],
    };
    let scene = SceneGraph {
        marks: vec![SceneGroup {
            marks: scene.marks,
            ..Default::default()
        }
        .into()],
        ..scene
    };
    let engine = avenger_text::default_text_engine();
    let mut canvas = pollster::block_on(PngCanvas::new(
        CanvasDimensions {
            size: [scene.width, scene.height],
            scale: 2.0,
        },
        CanvasConfig {
            text_engine: Some(engine),
            ..Default::default()
        },
    ))?;
    canvas.set_scene(&scene)?;
    pollster::block_on(canvas.render())?.save(&output)?;
    println!("Saved {output}");
    Ok(())
}
