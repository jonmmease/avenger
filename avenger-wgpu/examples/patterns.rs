//! Renders the pattern fill images that the scenegraph README shows: an overview, and galleries of
//! stripes, symbols, layer operations, and anchors with contrasting ink.
//!
//! Run with `cargo run --release -p avenger-wgpu --example patterns [OUTPUT_DIR]`.

use std::path::Path;

use avenger_color::ColorOrGradient;
use avenger_common::{canvas::CanvasDimensions, types::TextAlign, value::ScalarOrArray};
use avenger_scenegraph::{
    marks::{
        group::SceneGroup,
        mark::SceneMark,
        pattern::{
            PatternAnchor, PatternFill, PatternInk, PatternLayer, PatternLayerOperation,
            PatternSymbol, PatternSymbolShape, StripeDash, StripeLayer, SymbolLattice2d,
            SymbolLayer, SymbolPaint,
        },
        rect::SceneRectMark,
        text::SceneTextMark,
    },
    scene_graph::SceneGraph,
};
use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};

const HEIGHT: f32 = 160.0;
const TILE: f32 = 72.0;
const GAP: f32 = 16.0;
const LABEL_HEIGHT: f32 = 22.0;
const TILE_FILL: [f32; 4] = [0.87, 0.92, 0.97, 1.0];

/// Rects with one fill each and one pattern.
fn rects(
    x: Vec<f32>,
    y: Vec<f32>,
    width: f32,
    heights: Vec<f32>,
    fills: Vec<[f32; 4]>,
    pattern: PatternFill,
) -> SceneMark {
    SceneRectMark {
        len: x.len() as u32,
        x: ScalarOrArray::new_array(x),
        y: ScalarOrArray::new_array(y),
        width: Some(ScalarOrArray::new_scalar(width)),
        height: Some(ScalarOrArray::new_array(heights)),
        fill: ScalarOrArray::new_array(fills.into_iter().map(ColorOrGradient::Color).collect()),
        fill_pattern: pattern.into(),
        ..Default::default()
    }
    .into()
}

/// Bars from the bottom of the group, with one fill and one pattern.
fn bars(
    x: Vec<f32>,
    width: f32,
    heights: Vec<f32>,
    fill: [f32; 4],
    pattern: PatternFill,
) -> SceneMark {
    let y = heights.iter().map(|height| HEIGHT - height).collect();
    let fills = vec![fill; x.len()];
    rects(x, y, width, heights, fills, pattern)
}

fn label(x: f32, y: f32, text: &str) -> SceneMark {
    SceneTextMark {
        text: text.into(),
        x: x.into(),
        y: y.into(),
        font_size: 11.0.into(),
        align: TextAlign::Center.into(),
        color: ColorOrGradient::Color([0.25, 0.25, 0.25, 1.0]).into(),
        ..Default::default()
    }
    .into()
}

/// A dark ink that shows clearly on the light tiles.
fn solid(layers: Vec<PatternLayer>) -> PatternFill {
    PatternFill {
        ink: PatternInk::Solid {
            color: [0.1, 0.22, 0.42, 1.0],
            opacity: 0.8,
        },
        layers,
        ..Default::default()
    }
}

fn lattice(spacing: f32, v_angle: f32) -> SymbolLattice2d {
    SymbolLattice2d {
        u_spacing: spacing,
        u_angle: 0.0,
        v_spacing: spacing,
        v_angle,
        u_phase: 0.0,
        v_phase: 0.0,
    }
}

fn dashed(angle: f32, spacing: f32, length: f32, gap: f32) -> StripeLayer {
    let mut stripe = StripeLayer::new(angle, spacing, 2.0);
    stripe.dash = Some(StripeDash {
        length,
        gap,
        phase: 0.0,
    });
    stripe
}

/// Tiles in rows of `columns`, each labeled below unless its label is empty.
fn gallery(columns: usize, tiles: Vec<(&str, PatternFill)>) -> SceneGraph {
    let mut marks = Vec::new();
    let mut y = GAP;
    for row in tiles.chunks(columns) {
        let labeled = row.iter().any(|(text, _)| !text.is_empty());
        for (column, (text, pattern)) in row.iter().enumerate() {
            let x = GAP + column as f32 * (TILE + GAP);
            let tile = rects(
                vec![x],
                vec![y],
                TILE,
                vec![TILE],
                vec![TILE_FILL],
                pattern.clone(),
            );
            marks.push(tile);
            if !text.is_empty() {
                marks.push(label(x + TILE / 2.0, y + TILE + 14.0, text));
            }
        }
        y += TILE + if labeled { LABEL_HEIGHT } else { GAP };
    }
    SceneGraph {
        width: GAP + columns as f32 * (TILE + GAP),
        height: y,
        origin: [0.0, 0.0],
        marks,
    }
}

/// Stripes that three bars share through their group's pattern origin, a lattice of dots, and a
/// crosshatch that two stripe layers make with XOR.
fn overview() -> SceneGraph {
    let stripes = PatternFill {
        layers: vec![StripeLayer::new(45.0, 12.0, 3.0).into()],
        ..Default::default()
    };
    let dots = PatternFill {
        layers: vec![SymbolLayer {
            lattice: lattice(14.0, 60.0),
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
    SceneGraph {
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
    }
}

fn stripes() -> SceneGraph {
    let mut wide = StripeLayer::new(-30.0, 14.0, 6.0);
    wide.phase = 4.0;
    gallery(
        6,
        vec![
            ("0°", solid(vec![StripeLayer::new(0.0, 8.0, 2.0).into()])),
            ("45°", solid(vec![StripeLayer::new(45.0, 8.0, 2.0).into()])),
            ("-30°, wide", solid(vec![wide.into()])),
            ("dashed", solid(vec![dashed(45.0, 9.0, 6.0, 4.0).into()])),
            (
                "crosshatch",
                solid(vec![
                    StripeLayer::new(45.0, 10.0, 2.0).into(),
                    PatternLayer::from(StripeLayer::new(-45.0, 10.0, 2.0))
                        .with_operation(PatternLayerOperation::Xor),
                ]),
            ),
            ("dotted", solid(vec![dashed(0.0, 6.0, 2.0, 4.0).into()])),
        ],
    )
}

/// Each shape filled, above, and open.
fn symbols() -> SceneGraph {
    let tile = |shape, size, paint| {
        solid(vec![SymbolLayer {
            lattice: lattice(16.0, 90.0),
            symbol: PatternSymbol {
                shape,
                size,
                rotation: 0.0,
            },
            paint,
        }
        .into()])
    };
    // Rows of filled shapes, each above a row of the same shapes open and named. Five columns
    // split the shapes into full rows.
    let columns = 5;
    assert_eq!(PatternSymbolShape::ALL.len() % columns, 0);
    let mut tiles = Vec::new();
    for shapes in PatternSymbolShape::ALL.chunks(columns) {
        tiles.extend(
            shapes
                .iter()
                .map(|shape| ("", tile(*shape, 81.0, SymbolPaint::Filled))),
        );
        tiles.extend(shapes.iter().map(|shape| {
            (
                shape.name(),
                tile(*shape, 121.0, SymbolPaint::Open { stroke_width: 1.5 }),
            )
        }));
    }
    gallery(columns, tiles)
}

/// Stripes and a lattice of circles, and the three ways to combine them.
fn operations() -> SceneGraph {
    let stripes = || PatternLayer::from(StripeLayer::new(45.0, 8.0, 3.0));
    let circles = || {
        PatternLayer::from(SymbolLayer {
            lattice: lattice(24.0, 90.0),
            symbol: PatternSymbol {
                shape: PatternSymbolShape::Circle,
                size: 324.0,
                rotation: 0.0,
            },
            paint: SymbolPaint::Filled,
        })
    };
    gallery(
        5,
        vec![
            ("stripes", solid(vec![stripes()])),
            ("circles", solid(vec![circles()])),
            ("add", solid(vec![stripes(), circles()])),
            (
                "subtract",
                solid(vec![
                    stripes(),
                    circles().with_operation(PatternLayerOperation::Subtract),
                ]),
            ),
            (
                "xor",
                solid(vec![
                    stripes(),
                    circles().with_operation(PatternLayerOperation::Xor),
                ]),
            ),
        ],
    )
}

/// The same stripes anchored at their group's pattern origin, so they continue across bars, and
/// at each bar, so they restart. AutoContrast ink is black on the light bars and white on the
/// dark ones.
fn anchors() -> SceneGraph {
    let fills = vec![
        [0.93, 0.89, 0.74, 1.0],
        [0.84, 0.72, 0.5, 1.0],
        [0.62, 0.55, 0.48, 1.0],
        [0.36, 0.38, 0.5, 1.0],
        [0.16, 0.2, 0.36, 1.0],
    ];
    let heights = vec![90.0, 130.0, 70.0, 110.0, 150.0];
    let x: Vec<f32> = (0..5).map(|i| i as f32 * 60.0).collect();
    let row = |anchor, left: f32, text: &str| -> SceneMark {
        let pattern = PatternFill {
            anchor,
            ink: PatternInk::AutoContrast { opacity: 0.4 },
            layers: vec![StripeLayer::new(0.0, 12.0, 4.0).into()],
        };
        let y = heights.iter().map(|height| HEIGHT - height).collect();
        SceneGroup {
            origin: [left, 20.0],
            pattern_origin: Some([0.0, 0.0]),
            marks: vec![
                rects(x.clone(), y, 50.0, heights.clone(), fills.clone(), pattern),
                label(145.0, HEIGHT + 18.0, text),
            ],
            ..Default::default()
        }
        .into()
    };
    SceneGraph {
        width: 680.0,
        height: 210.0,
        origin: [0.0, 0.0],
        marks: vec![
            row(PatternAnchor::Group, 20.0, "group anchor"),
            row(PatternAnchor::Mark, 360.0, "mark anchor"),
        ],
    }
}

fn render(scene: &SceneGraph, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let mut canvas = pollster::block_on(PngCanvas::new(
        CanvasDimensions {
            size: [scene.width, scene.height],
            scale: 2.0,
        },
        CanvasConfig::default(),
    ))?;
    canvas.set_scene(scene, &avenger_typst_label::bundled_label_engine())?;
    pollster::block_on(canvas.render())?.save(path)?;
    println!("Saved {}", path.display());
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    let output = Path::new(&output);
    std::fs::create_dir_all(output)?;
    for (name, scene) in [
        ("patterns.png", overview()),
        ("pattern-stripes.png", stripes()),
        ("pattern-symbols.png", symbols()),
        ("pattern-operations.png", operations()),
        ("pattern-anchors.png", anchors()),
    ] {
        render(&scene, &output.join(name))?;
    }
    Ok(())
}
