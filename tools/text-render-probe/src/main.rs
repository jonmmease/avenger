use std::{path::PathBuf, sync::Arc};

use avenger_color::ColorOrGradient;
use avenger_common::{canvas::CanvasDimensions, value::ScalarOrArray};
use avenger_scenegraph::{
    marks::{mark::SceneMark, text::SceneTextMark},
    scene_graph::SceneGraph,
};
use avenger_text::types::{FontStyle, FontWeight, TextAlign, TextBaseline, TextSyntaxMode};
use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    pollster::block_on(run())
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/text-render-probe/hello.png"));

    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let dimensions = CanvasDimensions {
        size: [520.0, 220.0],
        scale: 2.0,
    };
    let mut canvas = PngCanvas::new(dimensions, canvas_config()).await?;
    canvas.set_scene(&hello_text_scene())?;
    let image = canvas.render().await?;
    image.save(&output)?;
    println!("{}", output.display());
    Ok(())
}

fn canvas_config() -> CanvasConfig {
    CanvasConfig::default()
}

fn hello_text_scene() -> SceneGraph {
    SceneGraph {
        width: 520.0,
        height: 220.0,
        origin: [0.0, 0.0],
        marks: vec![SceneMark::Text(Arc::new(SceneTextMark {
            name: "hello-text".to_string(),
            interactive: false,
            clip: false,
            len: 3,
            text: ScalarOrArray::new_array(vec![
                "Hello text renderer".to_string(),
                "Typst math: $E = mc^2$".to_string(),
                "Fraction: $sqrt(x) / (1 + x^2)$".to_string(),
            ]),
            text_syntax: TextSyntaxMode::TypstMarkup,
            text_params: avenger_text::LabelParams::default(),
            x: ScalarOrArray::new_array(vec![36.0, 36.0, 36.0]),
            y: ScalarOrArray::new_array(vec![48.0, 104.0, 160.0]),
            defined: ScalarOrArray::new_scalar(true),
            dx: ScalarOrArray::new_scalar(0.0),
            dy: ScalarOrArray::new_scalar(0.0),
            align: ScalarOrArray::new_scalar(TextAlign::Left),
            baseline: ScalarOrArray::new_scalar(TextBaseline::Alphabetic),
            angle: ScalarOrArray::new_scalar(0.0),
            color: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.08, 0.1, 0.15, 1.0])),
            opacity: ScalarOrArray::new_scalar(1.0),
            font: ScalarOrArray::new_scalar("sans-serif".to_string()),
            font_size: ScalarOrArray::new_array(vec![30.0, 28.0, 28.0]),
            font_weight: ScalarOrArray::new_array(vec![
                FontWeight::Number(700.0),
                FontWeight::default(),
                FontWeight::default(),
            ]),
            font_style: ScalarOrArray::new_scalar(FontStyle::Normal),
            limit: ScalarOrArray::new_scalar(f32::INFINITY),
            leader: ScalarOrArray::new_scalar(false),
            leader_stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color([
                0.08, 0.1, 0.15, 1.0,
            ])),
            leader_stroke_width: ScalarOrArray::new_scalar(1.0),
            leader_stroke_cap: ScalarOrArray::new_scalar(Default::default()),
            leader_stroke_join: ScalarOrArray::new_scalar(Default::default()),
            leader_stroke_dash: None,
            leader_label_padding: ScalarOrArray::new_scalar(2.0),
            leader_target_radius: ScalarOrArray::new_scalar(0.0),
            leader_min_length: ScalarOrArray::new_scalar(1.0),
            leader_shape: ScalarOrArray::new_scalar(Default::default()),
            leader_arrow: ScalarOrArray::new_scalar(Default::default()),
            leader_arrow_length: ScalarOrArray::new_scalar(6.0),
            leader_arrow_width: ScalarOrArray::new_scalar(5.0),
            indices: None,
            zindex: None,
        }))],
    }
}
