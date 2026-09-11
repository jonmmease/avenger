//! Render annotation leaders and open circles directly from scene marks.
use avenger_color::ColorOrGradient;
use avenger_common::{
    canvas::CanvasDimensions,
    types::{SceneTextLeaderArrow, SceneTextLeaderShape},
};
use avenger_scenegraph::{
    marks::{symbol::SceneSymbolMark, text::SceneTextMark},
    scene_graph::SceneGraph,
};
use avenger_text::types::TextSyntaxMode;
use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};

fn point(i: usize) -> [f32; 2] {
    let t = i as f32 / 127.0;
    [
        40.0 + t * 400.0,
        230.0 - t * 100.0 + (i as f32 * 2.4).sin() * 40.0,
    ]
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "annotation-leaders.png".into());
    let selected = point(83);
    let scene = SceneGraph {
        width: 700.0,
        height: 340.0,
        origin: [0.0, 0.0],
        marks: vec![
            SceneTextMark {
                text: "Annotations on a scatter plot".to_string().into(),
                x: 28.0.into(),
                y: 44.0.into(),
                font_size: 26.0.into(),
                ..Default::default()
            }
            .into(),
            SceneSymbolMark {
                len: 128,
                x: (0..128).map(|i| point(i)[0]).collect::<Vec<_>>().into(),
                y: (0..128).map(|i| point(i)[1]).collect::<Vec<_>>().into(),
                size: 64.0.into(),
                fill: ColorOrGradient::transparent().into(),
                stroke: ColorOrGradient::Color([0.13, 0.44, 0.64, 0.8]).into(),
                stroke_width: Some(1.5),
                ..Default::default()
            }
            .into(),
            SceneTextMark {
                text: "*Distance* $sqrt(x^2 + y^2)$".to_string().into(),
                text_syntax: TextSyntaxMode::TypstMarkup,
                x: selected[0].into(),
                y: selected[1].into(),
                dx: 110.0.into(),
                dy: 116.0.into(),
                font_size: 20.0.into(),
                leader: true.into(),
                leader_shape: SceneTextLeaderShape::Curved.into(),
                leader_arrow: SceneTextLeaderArrow::Triangle.into(),
                leader_stroke: ColorOrGradient::Color([0.16, 0.24, 0.32, 1.0]).into(),
                leader_stroke_width: 1.5.into(),
                leader_target_radius: 7.0.into(),
                leader_label_padding: 7.0.into(),
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
