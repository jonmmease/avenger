//! A canvas draws its layers from the bottom, each over the layers below it, and installs a layer
//! without installing the others again.

mod support;

use avenger_color::ColorOrGradient;
use avenger_common::canvas::CanvasDimensions;
use avenger_scenegraph::marks::rect::SceneRectMark;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};
use support::assert_baseline;

/// A scene of one rect from `x`, `y`, `width` by `height`, in `color`.
fn rect(x: f32, y: f32, width: f32, height: f32, color: [f32; 4]) -> SceneGraph {
    let rect = SceneRectMark {
        x: x.into(),
        y: y.into(),
        width: Some(width.into()),
        height: Some(height.into()),
        fill: ColorOrGradient::Color(color).into(),
        ..Default::default()
    };
    SceneGraph {
        marks: vec![rect.into()],
        width: 100.0,
        height: 60.0,
        origin: [0.0; 2],
    }
}

fn pixel(image: &image::RgbaImage, x: u32, y: u32) -> [u8; 4] {
    image.get_pixel(x, y).0
}

/// A red square on the top layer draws over the chart's blue bar. Installing another top layer
/// leaves the bottom layer installed and draws the new top layer over it, and installing a scene
/// as the canvas's one scene drops the top layer.
#[test]
fn layers_draw_over_the_layers_below() {
    let engine = avenger_typst_label::bundled_label_engine();
    let mut canvas = pollster::block_on(PngCanvas::new(
        CanvasDimensions {
            size: [100.0, 60.0],
            scale: 1.0,
        },
        CanvasConfig::default(),
    ))
    .unwrap();
    let (blue, red, green) = (
        [0.0, 0.0, 1.0, 1.0],
        [1.0, 0.0, 0.0, 1.0],
        [0.0, 1.0, 0.0, 1.0],
    );
    let chart = rect(10.0, 10.0, 80.0, 40.0, blue);
    canvas.set_scene(&chart, &engine).unwrap();
    (canvas.set_layer(1, &rect(20.0, 20.0, 20.0, 20.0, red), &engine)).unwrap();
    let image = pollster::block_on(canvas.render()).unwrap();
    assert_baseline(&image, "layers");

    (canvas.set_layer(1, &rect(60.0, 20.0, 20.0, 20.0, green), &engine)).unwrap();
    let image = pollster::block_on(canvas.render()).unwrap();
    assert!(!canvas.frame_stats().installed);
    assert_eq!(pixel(&image, 30, 30), [0, 0, 255, 255]);
    assert_eq!(pixel(&image, 70, 30), [0, 255, 0, 255]);
    assert_eq!(pixel(&image, 5, 5), [255, 255, 255, 255]);

    canvas.set_scene(&chart, &engine).unwrap();
    let image = pollster::block_on(canvas.render()).unwrap();
    assert_eq!(pixel(&image, 70, 30), [0, 0, 255, 255]);
}
