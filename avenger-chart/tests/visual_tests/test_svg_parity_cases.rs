use super::helpers::assert_scene_graph_visual_match_default;
use avenger_color::{ColorOrGradient, Gradient, GradientStop, LinearGradient};
use avenger_common::value::ScalarOrArray;
use avenger_scenegraph::{
    marks::{
        group::{Clip, SceneGroup},
        rect::SceneRectMark,
        text::SceneTextMark,
    },
    scene_graph::SceneGraph,
};

const BASELINE_CATEGORY: &str = "svg_parity";

#[tokio::test]
async fn svg_parity_subpixel_clip() {
    let scene_graph = SceneGraph {
        width: 80.0,
        height: 50.0,
        origin: [0.0, 0.0],
        marks: vec![
            SceneGroup {
                clip: Clip::Rect {
                    x: 13.25,
                    y: 10.5,
                    width: 42.5,
                    height: 24.75,
                },
                marks: vec![
                    SceneRectMark {
                        len: 1,
                        clip: true,
                        x: ScalarOrArray::new_scalar(6.0),
                        y: ScalarOrArray::new_scalar(7.0),
                        width: Some(ScalarOrArray::new_scalar(62.0)),
                        height: Some(ScalarOrArray::new_scalar(34.0)),
                        fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([
                            0.1, 0.42, 0.74, 1.0,
                        ])),
                        ..Default::default()
                    }
                    .into(),
                ],
                ..Default::default()
            }
            .into(),
        ],
    };

    assert_scene_graph_visual_match_default(&scene_graph, BASELINE_CATEGORY, "subpixel_clip").await;
}

#[tokio::test]
async fn svg_parity_linear_gradient() {
    let scene_graph = SceneGraph {
        width: 96.0,
        height: 42.0,
        origin: [0.0, 0.0],
        marks: vec![
            SceneRectMark {
                len: 1,
                gradients: vec![Gradient::LinearGradient(LinearGradient {
                    x0: 0.0,
                    y0: 0.0,
                    x1: 1.0,
                    y1: 0.0,
                    stops: vec![
                        GradientStop {
                            offset: 0.0,
                            color: [0.08, 0.16, 0.65, 1.0],
                        },
                        GradientStop {
                            offset: 0.5,
                            color: [0.1, 0.72, 0.45, 1.0],
                        },
                        GradientStop {
                            offset: 1.0,
                            color: [1.0, 0.82, 0.18, 1.0],
                        },
                    ],
                })],
                x: ScalarOrArray::new_scalar(8.0),
                y: ScalarOrArray::new_scalar(8.0),
                width: Some(ScalarOrArray::new_scalar(80.0)),
                height: Some(ScalarOrArray::new_scalar(26.0)),
                fill: ScalarOrArray::new_scalar(ColorOrGradient::GradientIndex(0)),
                ..Default::default()
            }
            .into(),
        ],
    };

    assert_scene_graph_visual_match_default(&scene_graph, BASELINE_CATEGORY, "linear_gradient")
        .await;
}

#[tokio::test]
async fn svg_parity_native_text() {
    let scene_graph = SceneGraph {
        width: 150.0,
        height: 42.0,
        origin: [0.0, 0.0],
        marks: vec![
            SceneTextMark {
                text: ScalarOrArray::new_scalar("Native text".to_string()),
                x: ScalarOrArray::new_scalar(8.0),
                y: ScalarOrArray::new_scalar(28.0),
                font: ScalarOrArray::new_scalar("Lato".to_string()),
                font_size: ScalarOrArray::new_scalar(20.0),
                color: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0])),
                ..Default::default()
            }
            .into(),
        ],
    };

    assert_scene_graph_visual_match_default(&scene_graph, BASELINE_CATEGORY, "native_text").await;
}
