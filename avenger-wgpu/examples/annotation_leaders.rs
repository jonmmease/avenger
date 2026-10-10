//! Render a label with a leader to the point it annotates, built with avenger-annotation's leaders.
use avenger_annotation::leader::{
    make_text_leaders, LeaderArrow, LeaderShape, LeaderStyle, LeaderTarget,
};
use avenger_color::ColorOrGradient;
use avenger_common::{canvas::CanvasDimensions, types::TextSyntaxMode};
use avenger_scenegraph::{
    marks::{group::SceneGroup, symbol::SceneSymbolMark, text::SceneTextMark},
    scene_graph::SceneGraph,
};
use avenger_wgpu::canvas::{Canvas, PngCanvas};

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
    let engine = avenger_typst_label::bundled_label_engine();
    let blue = [0.13, 0.44, 0.64, 0.8];
    let selected = point(83);
    let label = SceneTextMark {
        text: "*Distance* $sqrt(x^2 + y^2)$".to_string().into(),
        text_syntax: TextSyntaxMode::TypstMarkup,
        x: (selected[0] + 110.0).into(),
        y: (selected[1] + 116.0).into(),
        font_size: 20.0.into(),
        ..Default::default()
    };
    let leaders = make_text_leaders(
        &label,
        &[Some(LeaderTarget {
            position: selected,
            radius: 7.0,
        })],
        &LeaderStyle {
            shape: LeaderShape::Curved,
            arrow: LeaderArrow::Triangle,
            stroke: [0.16, 0.24, 0.32, 1.0],
            stroke_width: 1.5,
            label_padding: 7.0,
            ..Default::default()
        },
        &engine,
    )?;
    let scene = SceneGraph {
        width: 700.0,
        height: 340.0,
        origin: [0.0, 0.0],
        marks: vec![SceneGroup {
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
                    stroke: ColorOrGradient::Color(blue).into(),
                    stroke_width: Some(1.5),
                    ..Default::default()
                }
                .into(),
                // The annotated point, filled so that it stands out from its neighbors.
                SceneSymbolMark {
                    x: selected[0].into(),
                    y: selected[1].into(),
                    size: 64.0.into(),
                    fill: ColorOrGradient::Color(blue).into(),
                    stroke: ColorOrGradient::Color(blue).into(),
                    stroke_width: Some(1.5),
                    ..Default::default()
                }
                .into(),
                // Leaders draw before their labels, so labels draw over them.
                leaders.into(),
                label.into(),
            ],
            ..Default::default()
        }
        .into()],
    };
    let mut canvas = pollster::block_on(PngCanvas::new(
        CanvasDimensions {
            size: [scene.width, scene.height],
            scale: 2.0,
        },
        Default::default(),
    ))?;
    canvas.set_scene(&scene, &engine)?;
    pollster::block_on(canvas.render())?.save(&output)?;
    println!("Saved {output}");
    Ok(())
}
