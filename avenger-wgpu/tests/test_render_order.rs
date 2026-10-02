use avenger_color::ColorOrGradient;
use avenger_common::canvas::CanvasDimensions;
use avenger_scenegraph::{
    marks::{group::SceneGroup, rect::SceneRectMark},
    scene_graph::SceneGraph,
};
use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};

#[test]
fn root_marks_and_nested_marks_share_global_zindex_order() {
    let rect = |color: [f32; 4], zindex| SceneRectMark {
        x: 0.0.into(),
        y: 0.0.into(),
        width: Some(20.0.into()),
        height: Some(20.0.into()),
        fill: ColorOrGradient::Color(color).into(),
        zindex,
        ..Default::default()
    };
    let scene = SceneGraph {
        width: 20.0,
        height: 20.0,
        origin: [0.0, 0.0],
        marks: vec![
            SceneGroup {
                zindex: Some(20),
                marks: vec![rect([1.0, 0.0, 0.0, 1.0], None).into()],
                ..Default::default()
            }
            .into(),
            rect([0.0, 0.0, 1.0, 1.0], Some(5)).into(),
        ],
    };
    let mut canvas = pollster::block_on(PngCanvas::new(
        CanvasDimensions {
            size: [20.0, 20.0],
            scale: 1.0,
        },
        CanvasConfig::default(),
    ))
    .unwrap();
    canvas.set_scene(&scene).unwrap();
    let image = pollster::block_on(canvas.render()).unwrap();
    assert_eq!(image.get_pixel(10, 10).0, [255, 0, 0, 255]);
}
