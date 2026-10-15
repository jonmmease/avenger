//! Renders pattern fills from scene marks: stripes that three bars share through their group's
//! pattern origin, a lattice of dots, and a crosshatch that two stripe layers make with XOR.
//!
//! Run with `cargo run --release -p avenger-wgpu --example patterns [OUTPUT.png]`.

use avenger_color::ColorOrGradient;
use avenger_common::{canvas::CanvasDimensions, value::ScalarOrArray};
use avenger_scenegraph::{
    marks::{
        group::SceneGroup,
        mark::SceneMark,
        pattern::{
            PatternFill, PatternLayer, PatternLayerOperation, PatternSymbol, PatternSymbolShape,
            StripeLayer, SymbolLattice2d, SymbolLayer, SymbolPaint,
        },
        rect::SceneRectMark,
    },
    scene_graph::SceneGraph,
};
use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};

const HEIGHT: f32 = 160.0;

/// Bars from the bottom of the group, with one fill and one pattern.
fn bars(
    x: Vec<f32>,
    width: f32,
    heights: Vec<f32>,
    fill: [f32; 4],
    pattern: PatternFill,
) -> SceneMark {
    SceneRectMark {
        len: x.len() as u32,
        y: ScalarOrArray::new_array(heights.iter().map(|height| HEIGHT - height).collect()),
        x: ScalarOrArray::new_array(x),
        width: Some(ScalarOrArray::new_scalar(width)),
        height: Some(ScalarOrArray::new_array(heights)),
        fill: ScalarOrArray::new_scalar(ColorOrGradient::Color(fill)),
        fill_pattern: pattern.into(),
        ..Default::default()
    }
    .into()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "patterns.png".into());
    let stripes = PatternFill {
        layers: vec![StripeLayer::new(45.0, 12.0, 3.0).into()],
        ..Default::default()
    };
    let dots = PatternFill {
        layers: vec![SymbolLayer {
            lattice: SymbolLattice2d {
                u_spacing: 14.0,
                u_angle: 0.0,
                v_spacing: 14.0,
                v_angle: 60.0,
                u_phase: 0.0,
                v_phase: 0.0,
            },
            symbol: PatternSymbol {
                shape: PatternSymbolShape::Circle,
                size: 30.0,
                rotation: 0.0,
            },
            paint: SymbolPaint::Filled,
        }
        .into()],
        ..Default::default()
    };
    let crossing = PatternLayer::from(StripeLayer::new(-45.0, 12.0, 3.0))
        .with_operation(PatternLayerOperation::Xor);
    let crosshatch = PatternFill {
        layers: vec![StripeLayer::new(45.0, 12.0, 3.0).into(), crossing],
        ..Default::default()
    };
    let scene = SceneGraph {
        width: 480.0,
        height: 200.0,
        origin: [0.0, 0.0],
        marks: vec![SceneGroup {
            origin: [20.0, 20.0],
            // The bars' stripes start from the group's pattern origin, so they line up across
            // bars.
            pattern_origin: Some([0.0, 0.0]),
            marks: vec![
                bars(
                    vec![0.0, 70.0, 140.0],
                    60.0,
                    vec![110.0, 160.0, 80.0],
                    [0.3, 0.65, 0.8, 1.0],
                    stripes,
                ),
                bars(
                    vec![230.0],
                    100.0,
                    vec![140.0],
                    [0.95, 0.75, 0.35, 1.0],
                    dots,
                ),
                bars(
                    vec![340.0],
                    100.0,
                    vec![120.0],
                    [0.55, 0.8, 0.5, 1.0],
                    crosshatch,
                ),
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
        CanvasConfig::default(),
    ))?;
    canvas.set_scene(&scene, &avenger_typst_label::bundled_label_engine())?;
    pollster::block_on(canvas.render())?.save(&output)?;
    println!("Saved {output}");
    Ok(())
}
