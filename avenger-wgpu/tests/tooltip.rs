//! A tooltip draws in a canvas's overlay, over the scene, without installing the scene again.

mod support;

use avenger_color::ColorOrGradient;
use avenger_common::{canvas::CanvasDimensions, types::SymbolShape};
use avenger_scenegraph::marks::{rect::SceneRectMark, symbol::SceneSymbolMark};
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_tooltip::draw::{Tooltip, TooltipRow};
use avenger_typst_label::{bundled_font_options, EngineOptions, FontOptions, LabelEngine};
use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};
use support::assert_baseline;

/// A tooltip for a scatter point, in the overlay, tinted by the point's color, points its callout
/// at the point's edge. The image is compared with `tests/baselines/tooltip-overlay.png`.
#[test]
fn tooltip_overlay_draws_over_the_scene() {
    let engine = LabelEngine::new(EngineOptions {
        fonts: FontOptions {
            load_system_fonts: false,
            ..bundled_font_options()
        },
    });
    let blue = [0.27, 0.51, 0.71, 1.0];
    let circle = SymbolShape::from_vega_str("circle").unwrap();
    let points = SceneSymbolMark {
        len: 3,
        x: vec![40.0, 90.0, 150.0].into(),
        y: vec![80.0, 50.0, 70.0].into(),
        shapes: vec![circle],
        fill: ColorOrGradient::Color(blue).into(),
        size: 100.0.into(),
        ..Default::default()
    };
    let plot = SceneRectMark {
        x: 10.0.into(),
        y: 10.0.into(),
        width: Some(180.0.into()),
        height: Some(100.0.into()),
        fill: ColorOrGradient::Color([0.95, 0.95, 0.95, 1.0]).into(),
        ..Default::default()
    };
    let scene = SceneGraph {
        width: 200.0,
        height: 120.0,
        origin: [0.0; 2],
        marks: vec![plot.into(), points.into()],
    };
    // Tinted by the point's color.
    let tooltip = Tooltip {
        header: Some("Setosa".to_string()),
        rows: vec![
            TooltipRow::new("length", "5.1"),
            TooltipRow::new("width", "3.5"),
        ],
        tint: Some(blue),
        ..Default::default()
    };
    // The middle point, of radius 5.
    let [x, y, radius] = [90.0, 50.0, 5.0];
    let group = tooltip
        .marks(
            |side| {
                let [dx, dy] = side.direction();
                [x + dx * radius, y + dy * radius]
            },
            [0.0, 0.0, scene.width, scene.height],
            &engine,
        )
        .unwrap()
        .group;
    let overlay = SceneGraph {
        marks: vec![group.into()],
        ..scene.clone()
    };

    let mut canvas = pollster::block_on(PngCanvas::new(
        CanvasDimensions {
            size: [scene.width, scene.height],
            scale: 2.0,
        },
        CanvasConfig::default(),
    ))
    .unwrap();
    canvas.set_scene(&scene, &engine).unwrap();
    pollster::block_on(canvas.render()).unwrap();
    canvas.set_overlay(&overlay, &engine).unwrap();
    let image = pollster::block_on(canvas.render()).unwrap();
    assert!(
        !canvas.frame_stats().installed,
        "the overlay draws without installing the scene again"
    );
    assert_baseline(&image, "tooltip-overlay");
}
