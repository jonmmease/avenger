//! Compare the additional symbol shapes at three sizes.
use avenger_color::ColorOrGradient;
use avenger_common::{canvas::CanvasDimensions, types::SymbolShape};
use avenger_scenegraph::{
    marks::{symbol::SceneSymbolMark, text::SceneTextMark},
    scene_graph::SceneGraph,
};
use avenger_text::types::TextAlign;
use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "extra-symbols.png".into());
    let mut scene = SceneGraph {
        width: 600.0,
        height: 360.0,
        origin: [0.0, 0.0],
        marks: vec![],
    };
    for (column, name) in ["star", "wye", "pentagon", "cushion"]
        .into_iter()
        .enumerate()
    {
        let x = 75.0 + column as f32 * 150.0;
        scene.marks.push(
            SceneTextMark {
                text: name.into(),
                x: x.into(),
                y: 42.0.into(),
                font_size: 20.0.into(),
                align: TextAlign::Center.into(),
                ..Default::default()
            }
            .into(),
        );
        scene.marks.push(
            SceneSymbolMark {
                len: 3,
                x: x.into(),
                y: vec![95.0, 190.0, 290.0].into(),
                size: vec![64.0, 400.0, 1600.0].into(),
                shapes: vec![SymbolShape::from_vega_str(name)?],
                fill: ColorOrGradient::Color([0.13, 0.44, 0.64, 1.0]).into(),
                ..Default::default()
            }
            .into(),
        );
    }
    let mut canvas = pollster::block_on(PngCanvas::new(
        CanvasDimensions {
            size: [scene.width, scene.height],
            scale: 2.0,
        },
        CanvasConfig::default(),
    ))?;
    canvas.set_scene(&scene)?;
    pollster::block_on(canvas.render())?.save(output)?;
    Ok(())
}
